//! GET /api/lmstudio/status — LM Studio connection status + loaded models
//! POST /api/lmstudio/sync — trigger full LM Studio model registry sync
use axum::extract::State;
use axum::response::Json;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// (lmstudio_key, id, supports_vision, publisher, quantization, arch, active)
/// — the shape of an active LM Studio registry row. Factored out of the
/// sync function's tuple-heavy query so clippy doesn't flag type complexity.
type LmStudioRow = (
    String,
    i32,
    Option<bool>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<bool>,
);

#[derive(Debug, Deserialize)]
pub struct LsModelInfo {
    pub id: String,
    pub state: String,
    pub max_context_length: Option<i64>,
    /// LM Studio's model type: "llm", "vlm", "embeddings". Vision capability
    /// is signaled HERE (type == "vlm"), NOT in a capabilities field — that
    /// field is an ARRAY like ["tool_use"] and never contains a vision entry.
    /// Ground truth verified against /api/v0/models live 2026-07-08:
    /// qwen3-vl-8b reports type=vlm, capabilities=["tool_use"]. The old
    /// capabilities.vision probe synced EVERY model to supports_vision=false,
    /// which made the pre-flight gate block all vision runs after any sync.
    #[serde(rename = "type")]
    pub model_type: Option<String>,
    /// Provider-stated facts, threaded verbatim (migration 026): the maker's
    /// publisher handle, the quantization label (e.g. "4bit", "Q8_0"), and
    /// the architecture family (e.g. "qwen3", "gemma3"). None when LM Studio
    /// omits them — never synthesized.
    pub publisher: Option<String>,
    pub quantization: Option<String>,
    pub arch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LsModelsResponse {
    pub data: Vec<LsModelInfo>,
}

#[derive(Serialize)]
pub struct SyncResult {
    pub models_seen: i64,
    pub models_added: i64,
    pub models_updated: i64,
    pub models_deactivated: i64,
    pub duration_ms: i64,
}

#[derive(Serialize)]
pub struct LoadedModel {
    pub id: String,
    pub state: String,
    pub max_context_length: Option<i64>,
    pub model_type: Option<String>,
    pub publisher: Option<String>,
    pub quantization: Option<String>,
    pub arch: Option<String>,
}

#[derive(Serialize)]
pub struct LmStudioStatus {
    pub connected: bool,
    pub base_url: String,
    pub total_models: i64,
    pub loaded_models: i64,
    pub registered_models: i64,
    pub last_sync: Option<String>,
    pub loaded: Vec<LoadedModel>,
}

async fn fetch_lmstudio_models(client: &Client, base_url: &str) -> AppResult<Vec<LsModelInfo>> {
    let url = format!("{}/api/v0/models", base_url);
    tracing::info!("Fetching LM Studio models from: {}", url);
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            tracing::error!("LM Studio request failed: {:?}", e);
            AppError::Executor(format!("Request failed: {}", e))
        })?;
    
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        tracing::error!("LM Studio returned HTTP {}: {}", status, body);
        return Err(AppError::Executor(format!("LM Studio HTTP {}: {}", status, body)));
    }
    
    let json: LsModelsResponse = resp.json().await.map_err(|e| {
        tracing::error!("Failed to parse LM Studio response: {:?}", e);
        AppError::Executor(format!("Parse error: {}", e))
    })?;
    
    tracing::info!("LM Studio returned {} models", json.data.len());
    Ok(json.data)
}

pub async fn lmstudio_status(State(state): State<AppState>) -> AppResult<Json<LmStudioStatus>> {
    let client = Client::new();
    let base_url = &state.config.lmstudio_base_url;

    let models = match fetch_lmstudio_models(&client, base_url).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("LM Studio unreachable at {}: {}", base_url, e);
            return Ok(Json(LmStudioStatus {
                connected: false,
                base_url: base_url.clone(),
                total_models: 0,
                loaded_models: 0,
                registered_models: 0,
                last_sync: None,
                loaded: vec![],
            }));
        }
    };

    let total = models.len() as i64;
    let loaded = models.iter().filter(|m| m.state == "loaded").count() as i64;

    let registered: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM models WHERE provider = 'lmstudio' AND active = true",
    )
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    let last_sync: Option<String> = sqlx::query_scalar(
        "SELECT MAX(finished_at)::text FROM lmstudio_sync_log WHERE finished_at IS NOT NULL",
    )
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None)
    .flatten();

    Ok(Json(LmStudioStatus {
        connected: true,
        base_url: base_url.clone(),
        total_models: total,
        loaded_models: loaded,
        registered_models: registered,
        last_sync,
        loaded: models
            .into_iter()
            .filter(|m| m.state == "loaded")
            .map(|m| LoadedModel {
                id: m.id,
                state: m.state,
                max_context_length: m.max_context_length,
                model_type: m.model_type,
                publisher: m.publisher,
                quantization: m.quantization,
                arch: m.arch,
            })
            .collect(),
    }))
}

pub async fn lmstudio_sync(State(state): State<AppState>) -> AppResult<Json<SyncResult>> {
    let start = std::time::Instant::now();
    let client = Client::new();
    let base_url = &state.config.lmstudio_base_url;

    let lm_models = match fetch_lmstudio_models(&client, base_url).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to fetch LM Studio models from {}: {:?}", base_url, e);
            return Err(AppError::Executor(format!("Upstream HTTP error: {}", e)));
        }
    };
    let models_seen = lm_models.len() as i64;

    let mut added = 0;
    let mut updated = 0;
    let mut deactivated = 0;

    // Get currently active LM Studio models in registry
    // LM Studio can expose the same GGUF file under multiple ids, e.g.
    // `unsloth/step-3.7-flash` and `stepfun-ai/step-3.7-flash@q3_k_m` for the
    // same underlying filename `Step-3.7-Flash-UD-Q3_K_M-00001-of-00003.gguf`.
    // If we insert both, they become duplicate local cards/rows. Normalize by
    // filename so sync is idempotent and newest-bots shows one entry per GGUF.
    let gguf_filename_for = |id: &str| -> String {
      id.rsplit('/').next().unwrap_or(id).to_string()
    };

    // Existing active registry rows by lmstudio_key.
    let existing_by_key: HashMap<String, i32> = sqlx::query_as::<_, (String, i32)>(
        "SELECT lmstudio_key, id FROM models WHERE provider = 'lmstudio' AND active = true AND lmstudio_key IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();

    // Existing active registry rows by GGUF filename so deduped LM entries
    // can find their canonical row even when the key changed.
    let existing_by_file: HashMap<String, i32> = sqlx::query_as::<_, (String, i32)>(
        "SELECT lmstudio_key, id FROM models WHERE provider = 'lmstudio' AND active = true AND lmstudio_key IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|(k, id)| (gguf_filename_for(&k), id))
    .collect();

    let canonical_by_file: HashMap<String, LmStudioRow> = sqlx::query_as::<_, LmStudioRow>(
        "SELECT lmstudio_key, context_length, supports_vision, publisher, quantization, arch, active FROM models WHERE provider = 'lmstudio' AND active = true AND lmstudio_key IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|row| (gguf_filename_for(&row.0), row))
    .collect();

    let mut seen_files = HashSet::new();
    let mut seen_keys = HashSet::new();

    for lm in &lm_models {
        let key = &lm.id;
        let file = gguf_filename_for(key);
        let supports_vision = lm.model_type.as_deref() == Some("vlm");

        // Prefer existing row by exact key, then by GGUF filename.
        let existing_id = existing_by_key.get(key)
            .or_else(|| existing_by_file.get(&file))
            .copied();

        // Prefer publisher-prefixed id when multiple LM ids map to one file.
        let canonical = canonical_by_file.get(&file).cloned();
        let canonical_key = canonical
            .as_ref()
            .map(|(k, _, _, _, _, _, _)| {
                let ck = gguf_filename_for(k);
                if ck.contains('/') { k.clone() } else { key.clone() }
            })
            .unwrap_or_else(|| key.clone());

        let final_key = if canonical_key.contains('/') { canonical_key } else { key.clone() };
        let final_context = canonical
            .as_ref()
            .and_then(|(_, ctx, _, _, _, _, _)| if ctx > &0 { Some(*ctx as i64) } else { lm.max_context_length })
            .or(lm.max_context_length)
            .and_then(|v| if v > 0 { Some(v) } else { None })
            .map(i32::try_from)
            .and_then(std::result::Result::ok)
            .unwrap_or(0);
        let final_supports_vision = canonical
            .as_ref()
            .and_then(|(_, _, sv, _, _, _, _)| *sv)
            .unwrap_or(supports_vision);
        let final_publisher = canonical
            .as_ref()
            .and_then(|(_, _, _, pub_, _, _, _)| pub_.clone())
            .or(lm.publisher.clone());
        let final_quantization = canonical
            .as_ref()
            .and_then(|(_, _, _, _, quant, _, _)| quant.clone())
            .or(lm.quantization.clone());
        let final_arch = canonical
            .as_ref()
            .and_then(|(_, _, _, _, _, arch, _)| arch.clone())
            .or(lm.arch.clone());

        let final_key_ref = &final_key;
        let final_context_ref = &final_context;
        let final_supports_vision_ref = &final_supports_vision;
        let final_publisher_ref = &final_publisher;
        let final_quantization_ref = &final_quantization;
        let final_arch_ref = &final_arch;

        seen_keys.insert(final_key.clone());
        seen_files.insert(file.clone());

        if let Some(model_id) = existing_id {
            let rows = sqlx::query(
                r#"UPDATE models SET
                       display_name = $2,
                       context_length = $3,
                       supports_vision = $4,
                       publisher = $5,
                       quantization = $6,
                       arch = $7,
                       lmstudio_key = $8,
                       last_seen_in_lmstudio = NOW(),
                       updated_at = NOW()
                   WHERE id = $1"#,
            )
            .bind(model_id)
            .bind(final_key_ref)
            .bind(*final_context_ref)
            .bind(*final_supports_vision_ref)
            .bind(final_publisher_ref)
            .bind(final_quantization_ref)
            .bind(final_arch_ref)
            .bind(final_key_ref)
            .execute(&state.db)
            .await?;
            if rows.rows_affected() > 0 {
                updated += 1;
            }
        } else {
            sqlx::query(
                r#"INSERT INTO models (key, display_name, provider, location, context_length, supports_vision, publisher, quantization, arch, lmstudio_key, last_seen_in_lmstudio, active)
                   VALUES ($1, $2, 'lmstudio', 'local', $3, $4, $5, $6, $7, $8, NOW(), true)
                   ON CONFLICT (key, provider) DO UPDATE SET
                       display_name = EXCLUDED.display_name,
                       context_length = EXCLUDED.context_length,
                       supports_vision = EXCLUDED.supports_vision,
                       publisher = EXCLUDED.publisher,
                       quantization = EXCLUDED.quantization,
                       arch = EXCLUDED.arch,
                       lmstudio_key = EXCLUDED.lmstudio_key,
                       last_seen_in_lmstudio = NOW(),
                       updated_at = NOW(),
                       active = true"#,
            )
            .bind(final_key_ref)
            .bind(final_key_ref)
            .bind(*final_context_ref)
            .bind(*final_supports_vision_ref)
            .bind(final_publisher_ref)
            .bind(final_quantization_ref)
            .bind(final_arch_ref)
            .bind(final_key_ref)
            .execute(&state.db)
            .await?;
            added += 1;
        }
    }

    // Deactivate only LM keys that were active before and neither duplicated
    // nor seen in this sync. Avoid nuking alternate ids for files we just
    // kept under a canonical key.
    for (key, id) in existing_by_key {
        if !seen_keys.contains(&key) {
            let file = gguf_filename_for(&key);
            if !seen_files.contains(&file) {
                sqlx::query("UPDATE models SET active = false, updated_at = NOW() WHERE id = $1")
                    .bind(id)
                    .execute(&state.db)
                    .await?;
                deactivated += 1;
            }
        }
    }

    let duration = start.elapsed().as_millis() as i64;

    sqlx::query(
        r#"INSERT INTO lmstudio_sync_log (models_seen, models_added, models_updated, models_deactivated, finished_at)
           VALUES ($1, $2, $3, $4, NOW())"#,
    )
    .bind(models_seen)
    .bind(added)
    .bind(updated)
    .bind(deactivated)
    .execute(&state.db)
    .await?;

    // Registry mutated — push a fresh snapshot to every open SSE connection
    // immediately. The dashboard's grid re-renders from this event; the
    // frontend never calls back for data (SSE-only contract, no polling).
    if added + updated + deactivated > 0 {
        if let Some(json) = crate::routes::events::registry_envelope(&state, "refresh").await {
            let _ = state.events_tx.send(json); // Err = no subscribers; fine.
        }
    }

    Ok(Json(SyncResult {
        models_seen,
        models_added: added,
        models_updated: updated,
        models_deactivated: deactivated,
        duration_ms: duration,
    }))
}