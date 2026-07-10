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
fn is_chat_model(id: &str) -> bool {
    let lower = id.to_lowercase();
    !(id.starts_with('~')
        || lower.contains("embedding")
        || lower.contains("embed-")
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

pub async fn cloud_sync(State(state): State<AppState>) -> AppResult<Json<CloudSyncResult>> {
    let started = std::time::Instant::now();
    let client = Client::new();
    let mut providers = Vec::new();

    for provider in ["nous", "openrouter", "openai"] {
        let config_key = match provider {
            "nous" => &state.config.nous_api_key,
            "openrouter" => &state.config.openrouter_api_key,
            "openai" => &state.config.openai_api_key,
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

            let display = entry
                .get("name")
                .and_then(|v| v.as_str())
                .map(|n| format!("{} (Cloud · {})", n, provider))
                .unwrap_or_else(|| format!("{} (Cloud · {})", id, provider));
            let ctx = context_from_entry(entry);
            let vision = vision_from_entry(entry);

            // Upsert keyed on (key, provider): a model id can exist on both
            // Nous and OpenRouter with different routing/pricing — those are
            // distinct test subjects and must stay distinct rows.
            let result = sqlx::query(
                r#"
                INSERT INTO models (key, display_name, provider, location, context_length,
                                    supports_vision, tags, active)
                VALUES ($1, $2, $3, 'cloud', $4, $5, ARRAY['cloud', $3], true)
                ON CONFLICT (key, provider) DO UPDATE SET
                    context_length = EXCLUDED.context_length,
                    supports_vision = EXCLUDED.supports_vision,
                    updated_at = CURRENT_TIMESTAMP
                RETURNING (xmax = 0) AS inserted
                "#,
            )
            .bind(id)
            .bind(&display)
            .bind(provider)
            .bind(ctx as i32)
            .bind(vision)
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
