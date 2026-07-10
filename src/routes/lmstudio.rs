//! GET /api/lmstudio/status — LM Studio connection status + model count
//! POST /api/lmstudio/sync — trigger full LM Studio model registry sync
use axum::extract::State;
use axum::response::Json;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

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
pub struct LmStudioStatus {
    pub connected: bool,
    pub base_url: String,
    pub total_models: i64,
    pub loaded_models: i64,
    pub registered_models: i64,
    pub last_sync: Option<String>,
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
    let existing: HashMap<String, i32> = sqlx::query_as::<_, (String, i32)>(
        "SELECT lmstudio_key, id FROM models WHERE provider = 'lmstudio' AND active = true AND lmstudio_key IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();

    let mut seen_keys = std::collections::HashSet::new();

    for lm in &lm_models {
        let key = &lm.id;
        seen_keys.insert(key.clone());

        // Vision capability detection — type == "vlm" is the real signal.
        // BUG HISTORY (2026-07-08, caught by a live 400 on a vision run):
        // this used to probe capabilities.vision as a bool, which NEVER
        // exists (capabilities is ["tool_use"]-style array) — so every model
        // including qwen3-vl-8b synced as supports_vision=false and the
        // pre-flight gate blocked ALL vision runs after any sync.
        let supports_vision = lm.model_type.as_deref() == Some("vlm");

        if let Some(&model_id) = existing.get(key) {
            // Update existing
            let rows = sqlx::query(
                r#"UPDATE models SET
                       display_name = $2,
                       context_length = $3,
                       supports_vision = $4,
                       last_seen_in_lmstudio = NOW(),
                       updated_at = NOW()
                   WHERE id = $1"#,
            )
            .bind(model_id)
            .bind(&lm.id)
            .bind(lm.max_context_length.unwrap_or(0))
            .bind(supports_vision)
            .execute(&state.db)
            .await?;
            if rows.rows_affected() > 0 {
                updated += 1;
            }
        } else {
            // Insert new
            sqlx::query(
                r#"INSERT INTO models (key, display_name, provider, location, context_length, supports_vision, lmstudio_key, last_seen_in_lmstudio, active)
                   VALUES ($1, $2, 'lmstudio', 'local', $3, $4, $5, NOW(), true)
                   ON CONFLICT (key) DO UPDATE SET
                       display_name = EXCLUDED.display_name,
                       context_length = EXCLUDED.context_length,
                       supports_vision = EXCLUDED.supports_vision,
                       lmstudio_key = EXCLUDED.lmstudio_key,
                       last_seen_in_lmstudio = NOW(),
                       updated_at = NOW(),
                       active = true"#,
            )
            .bind(&lm.id)
            .bind(&lm.id)
            .bind(lm.max_context_length.unwrap_or(0))
            .bind(supports_vision)
            .bind(&lm.id)
            .execute(&state.db)
            .await?;
            added += 1;
        }
    }

    // Deactivate models no longer in LM Studio
    for (key, id) in existing {
        if !seen_keys.contains(&key) {
            sqlx::query("UPDATE models SET active = false, updated_at = NOW() WHERE id = $1")
                .bind(id)
                .execute(&state.db)
                .await?;
            deactivated += 1;
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