//! POST /api/cloud/sync — discover and register every cloud model the
//! configured API keys can actually reach, mirroring the LM Studio sync.
//!
//! Providers:
//!   nous       — GET https://inference-api.nousresearch.com/v1/models
//!   openrouter — GET https://openrouter.ai/api/v1/models
//!   openai     — GET https://api.openai.com/v1/models
//!
//! A provider is synced only when a key resolves for it (env, secrets file,
//! or — for Nous — the live Hermes OAuth key). No key → provider skipped and
//! reported as such; never an error. The scientific contract: the grid shows
//! exactly what your credentials can reach, verified by asking the provider,
//! not by hand-maintained rows.
use axum::extract::State;
use axum::response::Json;
use reqwest::Client;
use serde::Serialize;

use crate::error::AppResult;
use crate::executor::cloud;
use crate::state::AppState;

#[derive(Serialize)]
pub struct ProviderSync {
    pub provider: String,
    pub reachable: bool,
    pub models_seen: i64,
    pub models_added: i64,
    pub models_updated: i64,
    pub skipped_reason: Option<String>,
}

#[derive(Serialize)]
pub struct CloudSyncResult {
    pub providers: Vec<ProviderSync>,
    pub duration_ms: i64,
}

fn models_endpoint(provider: &str) -> &'static str {
    match provider {
        "nous" => "https://inference-api.nousresearch.com/v1/models",
        "openrouter" => "https://openrouter.ai/api/v1/models",
        "openai" => "https://api.openai.com/v1/models",
        _ => unreachable!(),
    }
}

/// Filter out non-chat entries: embeddings, image/audio/video generators,
/// moderation endpoints, and alias rows (Nous prefixes those with '~').
/// Embedding families are matched by id prefix too — bge/e5/minilm/mpnet
/// style ids don't contain the word "embedding" (found live 2026-07-09:
/// sentence-transformers/* slipped through and synced as chat models).
fn is_chat_model(id: &str) -> bool {
    let lower = id.to_lowercase();
    !(id.starts_with('~')
        || lower.contains("embedding")
        || lower.contains("embed-")
        || lower.starts_with("sentence-transformers/")
        || lower.starts_with("baai/bge")
        || lower.starts_with("intfloat/e5")
        || lower.contains("-image")
        || lower.contains("image-")
        || lower.contains("audio")
        || lower.contains("video")
        || lower.contains("moderation")
        || lower.contains("whisper")
        || lower.contains("tts")
        || lower.contains("dall-e"))
}

/// Vision support, when the catalog states it (OpenRouter architecture
/// metadata). Absence of metadata means false — never guessed.
fn vision_from_entry(entry: &serde_json::Value) -> bool {
    entry
        .pointer("/architecture/input_modalities")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().any(|m| m.as_str() == Some("image")))
        .unwrap_or(false)
}

fn context_from_entry(entry: &serde_json::Value) -> i64 {
    entry
        .get("context_length")
        .and_then(|v| v.as_i64())
        .or_else(|| entry.pointer("/top_provider/context_length").and_then(|v| v.as_i64()))
        .unwrap_or(0)
}

/// Catalog unit price in USD per token, exactly as the provider states it.
/// Both Nous and OpenRouter serve pricing.prompt / pricing.completion as
/// decimal STRINGS ("0.0000002000") — parsed here, never guessed. None when
/// the field is absent or unparseable: an unpriced model stays honestly
/// unpriced rather than defaulting to 0 (which would claim "free").
fn price_from_entry(entry: &serde_json::Value, field: &str) -> Option<f64> {
    entry
        .pointer(&format!("/pricing/{}", field))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|p| p.is_finite() && *p >= 0.0)
}

pub async fn cloud_sync(State(state): State<AppState>) -> AppResult<Json<CloudSyncResult>> {
    let started = std::time::Instant::now();
    let client = Client::new();
    let mut providers = Vec::new();

    for provider in ["nous", "openrouter", "openai", "gemini"] {
        let config_key = match provider {
            "nous" => &state.config.nous_api_key,
            "openrouter" => &state.config.openrouter_api_key,
            "openai" => &state.config.openai_api_key,
            "gemini" => &state.config.gemini_api_key,
            _ => unreachable!(),
        };

        // No key → skip honestly, never fail the whole sync.
        let key = match cloud::resolve_api_key(provider, config_key) {
            Ok(k) => k,
            Err(e) => {
                providers.push(ProviderSync {
                    provider: provider.to_string(),
                    reachable: false,
                    models_seen: 0,
                    models_added: 0,
                    models_updated: 0,
                    skipped_reason: Some(format!("no key: {}", e)),
                });
                continue;
            }
        };

        // Gemini's list API differs from the OpenAI-compatible providers:
        // key is a query param (not Bearer), the list is under `models[]`
        // (not `data[]`), fields are `displayName` / `generationMethods` /
        // `inputTokenLimit`, and there is no per-model pricing in the catalog.
        // Parse it into the same (id, display, ctx, vision, prices) shape the
        // upsert expects. We keep only generateContent-capable chat models; we
        // cannot infer vision from the catalog reliably, so we flag vision
        // from the model id's known multimodal family (gemini-* with vision).
        if provider == "gemini" {
            let key = match cloud::resolve_api_key(provider, config_key) {
                Ok(k) => k,
                Err(e) => {
                    providers.push(ProviderSync {
                        provider: provider.to_string(),
                        reachable: false,
                        models_seen: 0,
                        models_added: 0,
                        models_updated: 0,
                        skipped_reason: Some(format!("no key: {}", e)),
                    });
                    continue;
                }
            };
            let list_url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models?key={}",
                key
            );
            let resp = client
                .get(&list_url)
                .timeout(std::time::Duration::from_secs(20))
                .send()
                .await;
            let json: serde_json::Value = match resp {
                Ok(r) if r.status().is_success() => match r.json().await {
                    Ok(j) => j,
                    Err(e) => {
                        providers.push(ProviderSync {
                            provider: provider.to_string(),
                            reachable: false,
                            models_seen: 0,
                            models_added: 0,
                            models_updated: 0,
                            skipped_reason: Some(format!("parse error: {}", e)),
                        });
                        continue;
                    }
                },
                Ok(r) => {
                    providers.push(ProviderSync {
                        provider: provider.to_string(),
                        reachable: false,
                        models_seen: 0,
                        models_added: 0,
                        models_updated: 0,
                        skipped_reason: Some(format!("HTTP {}", r.status())),
                    });
                    continue;
                }
                Err(e) => {
                    providers.push(ProviderSync {
                        provider: provider.to_string(),
                        reachable: false,
                        models_seen: 0,
                        models_added: 0,
                        models_updated: 0,
                        skipped_reason: Some(format!("request failed: {}", e)),
                    });
                    continue;
                }
            };
            let entries: Vec<serde_json::Value> = json
                .get("models")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut seen = 0i64;
            let mut added = 0i64;
            let mut updated = 0i64;
            for entry in &entries {
                let id = match entry.get("name").and_then(|v| v.as_str()) {
                    // Gemini returns "models/gemini-3.5-flash" — strip prefix.
                    Some(s) if is_chat_model(s.split('/').next_back().unwrap_or(s)) => {
                        s.split('/').next_back().unwrap_or(s).to_string()
                    }
                    _ => continue,
                };
                seen += 1;
                let display = entry
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string();
                let ctx = entry
                    .get("inputTokenLimit")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32;
                // Vision: Gemini catalog does not expose input_modalities; we
                // flag from the known multimodal family. gemini-* with vision
                // capability per Google docs; we mark the flash/pro/ultra 3.x
                // lines as vision-capable (they accept images).
                let vision = id.contains("gemini") && !id.contains("embedding");
                let price_prompt: Option<f64> = None;
                let price_completion: Option<f64> = None;
                let result = sqlx::query(
                    r#"
                    INSERT INTO models (key, display_name, provider, location, context_length,
                                        supports_vision, price_prompt, price_completion,
                                        pricing_updated_at, tags, active)
                    VALUES ($1, $2, $3, 'cloud', $4, $5, $6, $7,
                            CASE WHEN $6::numeric IS NULL AND $7::numeric IS NULL THEN NULL ELSE NOW() END,
                            ARRAY['cloud', $3], true)
                    ON CONFLICT (key, provider) DO UPDATE SET
                        context_length = EXCLUDED.context_length,
                        supports_vision = EXCLUDED.supports_vision,
                        price_prompt = EXCLUDED.price_prompt,
                        price_completion = EXCLUDED.price_completion,
                        pricing_updated_at = EXCLUDED.pricing_updated_at,
                        updated_at = CURRENT_TIMESTAMP
                    RETURNING (xmax = 0) AS inserted
                    "#,
                )
                .bind(&id)
                .bind(&display)
                .bind(provider)
                .bind(ctx)
                .bind(vision)
                .bind(price_prompt)
                .bind(price_completion)
                .fetch_one(&state.db)
                .await;
                match result {
                    Ok(row) => {
                        use sqlx::Row;
                        if row.try_get::<bool, _>("inserted").unwrap_or(false) {
                            added += 1;
                        } else {
                            updated += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("cloud_sync upsert failed for {}/{}: {}", provider, id, e);
                    }
                }
            }
            providers.push(ProviderSync {
                provider: provider.to_string(),
                reachable: true,
                models_seen: seen,
                models_added: added,
                models_updated: updated,
                skipped_reason: None,
            });
            continue;
        }

        let resp = client
            .get(models_endpoint(provider))
            .header("Authorization", format!("Bearer {}", key))
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await;

        let json: serde_json::Value = match resp {
            Ok(r) if r.status().is_success() => match r.json().await {
                Ok(j) => j,
                Err(e) => {
                    providers.push(ProviderSync {
                        provider: provider.to_string(),
                        reachable: false,
                        models_seen: 0,
                        models_added: 0,
                        models_updated: 0,
                        skipped_reason: Some(format!("parse error: {}", e)),
                    });
                    continue;
                }
            },
            Ok(r) => {
                providers.push(ProviderSync {
                    provider: provider.to_string(),
                    reachable: false,
                    models_seen: 0,
                    models_added: 0,
                    models_updated: 0,
                    skipped_reason: Some(format!("HTTP {}", r.status())),
                });
                continue;
            }
            Err(e) => {
                providers.push(ProviderSync {
                    provider: provider.to_string(),
                    reachable: false,
                    models_seen: 0,
                    models_added: 0,
                    models_updated: 0,
                    skipped_reason: Some(format!("request failed: {}", e)),
                });
                continue;
            }
        };

        let entries: Vec<serde_json::Value> = json
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut seen = 0i64;
        let mut added = 0i64;
        let mut updated = 0i64;

        for entry in &entries {
            let id = match entry.get("id").and_then(|v| v.as_str()) {
                Some(s) if is_chat_model(s) => s,
                _ => continue,
            };
            seen += 1;

            // Display name = the provider's own name field, verbatim. The old
            // code baked "(Cloud · provider)" into the string at sync time —
            // presentation manufactured in the data layer, triple-redundant
            // with the CLOUD badge + provider tag (migration 026 stripped it).
            let display = entry
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(id)
                .to_string();
            let ctx = context_from_entry(entry);
            let vision = vision_from_entry(entry);
            let price_prompt = price_from_entry(entry, "prompt");
            let price_completion = price_from_entry(entry, "completion");

            // Upsert keyed on (key, provider): a model id can exist on both
            // Nous and OpenRouter with different routing/pricing — those are
            // distinct test subjects and must stay distinct rows.
            let result = sqlx::query(
                r#"
                INSERT INTO models (key, display_name, provider, location, context_length,
                                    supports_vision, price_prompt, price_completion,
                                    pricing_updated_at, tags, active)
                VALUES ($1, $2, $3, 'cloud', $4, $5, $6, $7,
                        CASE WHEN $6::numeric IS NULL AND $7::numeric IS NULL THEN NULL ELSE NOW() END,
                        ARRAY['cloud', $3], true)
                ON CONFLICT (key, provider) DO UPDATE SET
                    context_length = EXCLUDED.context_length,
                    supports_vision = EXCLUDED.supports_vision,
                    price_prompt = EXCLUDED.price_prompt,
                    price_completion = EXCLUDED.price_completion,
                    pricing_updated_at = EXCLUDED.pricing_updated_at,
                    updated_at = CURRENT_TIMESTAMP
                RETURNING (xmax = 0) AS inserted
                "#,
            )
            .bind(id)
            .bind(&display)
            .bind(provider)
            .bind(ctx as i32)
            .bind(vision)
            .bind(price_prompt)
            .bind(price_completion)
            .fetch_one(&state.db)
            .await;

            match result {
                Ok(row) => {
                    use sqlx::Row;
                    if row.try_get::<bool, _>("inserted").unwrap_or(false) {
                        added += 1;
                    } else {
                        updated += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!("cloud_sync upsert failed for {}/{}: {}", provider, id, e);
                }
            }
        }

        providers.push(ProviderSync {
            provider: provider.to_string(),
            reachable: true,
            models_seen: seen,
            models_added: added,
            models_updated: updated,
            skipped_reason: None,
        });
    }

    // Registry mutated — push a fresh snapshot to every open SSE connection
    // immediately (same contract as lmstudio_sync; grid updates with zero
    // frontend fetch-back).
    if providers.iter().any(|p| p.models_added + p.models_updated > 0) {
        if let Some(json) = crate::routes::events::registry_envelope(&state, "refresh").await {
            let _ = state.events_tx.send(json); // Err = no subscribers; fine.
        }
    }

    Ok(Json(CloudSyncResult {
        providers,
        duration_ms: started.elapsed().as_millis() as i64,
    }))
}
