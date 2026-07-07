//! LM Studio REST client — the local half of the executor.
//!
//! Uses two API surfaces (both verified live against LM Studio 2026-07-07):
//!   /api/v0/models          — model list with per-model `state` (loaded | not-loaded)
//!   /api/v1/models          — model list with `loaded_instances` (instance ids + config)
//!   /api/v1/models/unload   — body {"instance_id": "..."} (verified: unloaded granite live)
//!   /api/v1/models/load     — body {"model": "..."} (endpoint verified; falls back to a
//!                             1-token JIT chat probe if it errors)
//!   /api/v0/chat/completions — OpenAI-compatible chat, supports vision content arrays
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Instant;

use crate::error::{AppError, AppResult};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LsModelInfo {
    pub id: String,
    #[serde(rename = "type")]
    pub model_type: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub arch: String,
    #[serde(rename = "state")]
    pub load_state: String,
    #[serde(rename = "max_context_length", default)]
    pub context_length: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LsModelsResponse {
    pub data: Vec<LsModelInfo>,
}

/// Query LM Studio for all selectable models (loaded and unloaded).
pub async fn list_ls_models(client: &Client, base_url: &str) -> AppResult<Vec<LsModelInfo>> {
    let resp = client
        .get(format!("{}/api/v0/models", base_url))
        .send()
        .await?
        .error_for_status()?;
    let json: LsModelsResponse = resp.json().await?;
    Ok(json.data)
}

/// Clean-room step 1: eject EVERY loaded instance so the target model runs
/// with zero cross-contamination and honest RAM/latency numbers.
/// Returns the ids of the instances that were ejected.
pub async fn eject_all(client: &Client, base_url: &str) -> AppResult<Vec<String>> {
    let resp = client
        .get(format!("{}/api/v1/models", base_url))
        .send()
        .await?
        .error_for_status()?;
    let v: serde_json::Value = resp.json().await?;

    let mut ejected = Vec::new();
    if let Some(models) = v.get("models").and_then(|m| m.as_array()) {
        for m in models {
            if let Some(instances) = m.get("loaded_instances").and_then(|i| i.as_array()) {
                for inst in instances {
                    if let Some(id) = inst.get("id").and_then(|i| i.as_str()) {
                        let r = client
                            .post(format!("{}/api/v1/models/unload", base_url))
                            .json(&serde_json::json!({ "instance_id": id }))
                            .send()
                            .await?;
                        if r.status().is_success() {
                            ejected.push(id.to_string());
                        } else {
                            tracing::warn!("Failed to unload instance {}: HTTP {}", id, r.status());
                        }
                    }
                }
            }
        }
    }
    Ok(ejected)
}

/// Clean-room step 2: load ONLY the target model, then poll until LM Studio
/// reports it resident (state == "loaded"). Never assume readiness — verify.
pub async fn ensure_loaded(
    client: &Client,
    base_url: &str,
    model_key: &str,
    max_wait_secs: u64,
) -> AppResult<bool> {
    // Preferred: explicit load endpoint.
    let load_resp = client
        .post(format!("{}/api/v1/models/load", base_url))
        .json(&serde_json::json!({ "model": model_key }))
        .timeout(std::time::Duration::from_secs(max_wait_secs))
        .send()
        .await;

    let explicit_load_ok = matches!(&load_resp, Ok(r) if r.status().is_success());
    if !explicit_load_ok {
        // Fallback: a 1-token chat probe triggers LM Studio's JIT loader.
        tracing::warn!("Explicit load failed for {}; falling back to JIT probe", model_key);
        let _ = client
            .post(format!("{}/api/v0/chat/completions", base_url))
            .json(&serde_json::json!({
                "model": model_key,
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 1
            }))
            .timeout(std::time::Duration::from_secs(max_wait_secs))
            .send()
            .await;
    }

    // Verify residency by polling — the scientific contract: never assume.
    let start = Instant::now();
    loop {
        let models = list_ls_models(client, base_url).await?;
        if models
            .iter()
            .any(|m| m.id == model_key && m.load_state == "loaded")
        {
            return Ok(true);
        }
        if start.elapsed().as_secs() >= max_wait_secs {
            return Ok(false);
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Execute one chat completion. `messages` are raw OpenAI-shaped values so
/// callers can pass plain text or vision content arrays identically.
/// Returns (content, latency_ms).
pub async fn chat(
    client: &Client,
    base_url: &str,
    model_key: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
    temperature: f32,
) -> AppResult<(String, u64)> {
    let body = serde_json::json!({
        "model": model_key,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": temperature,
    });

    let start = Instant::now();
    let resp = client
        .post(format!("{}/api/v0/chat/completions", base_url))
        .json(&body)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await?
        .error_for_status()?;
    let elapsed = start.elapsed().as_millis() as u64;

    let json: serde_json::Value = resp.json().await?;
    let content = json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            AppError::Executor(format!(
                "LM Studio returned no content for {} (raw: {})",
                model_key,
                &json.to_string().chars().take(300).collect::<String>()
            ))
        })?;

    Ok((content, elapsed))
}
