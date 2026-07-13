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

/// Inspect current loaded instances and return (model_key, instance_id)
/// pairs for every resident instance. Used by speculative-pair mode to
/// verify both primary and draft models are loaded simultaneously.
pub async fn list_loaded_instances(client: &Client, base_url: &str) -> AppResult<Vec<(String, String)>> {
    let resp = client
        .get(format!("{}/api/v1/models", base_url))
        .send()
        .await?
        .error_for_status()?;
    let v: serde_json::Value = resp.json().await?;

    let mut out = Vec::new();
    if let Some(models) = v.get("models").and_then(|m| m.as_array()) {
        for m in models {
            let key = m.get("key").and_then(|i| i.as_str()).or_else(|| m.get("id").and_then(|i| i.as_str())).unwrap_or("");
            if let Some(instances) = m.get("loaded_instances").and_then(|i| i.as_array()) {
                for inst in instances {
                    if let Some(iid) = inst.get("id").and_then(|i| i.as_str()) {
                        out.push((key.to_string(), iid.to_string()));
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Fetch the loaded instance config for a model key from `/api/v1/models`.
/// Returns None when the model is not resident or the config shape is missing.
pub async fn fetch_instance_config(
    client: &Client,
    base_url: &str,
    model_key: &str,
) -> AppResult<Option<serde_json::Value>> {
    let resp = client
        .get(format!("{}/api/v1/models", base_url))
        .send()
        .await?
        .error_for_status()?;
    let v: serde_json::Value = resp.json().await?;
    if let Some(models) = v.get("models").and_then(|m| m.as_array()) {
        for m in models {
            let key = m.get("key").and_then(|i| i.as_str()).or_else(|| m.get("id").and_then(|i| i.as_str())).unwrap_or("");
            if key != model_key {
                continue;
            }
            if let Some(instances) = m.get("loaded_instances").and_then(|i| i.as_array()) {
                if let Some(first) = instances.first() {
                    return Ok(first.get("config").cloned());
                }
            }
            return Ok(None);
        }
    }
    Ok(None)
}

/// Speculative-pair helper: ensure BOTH primary and draft models are loaded
/// and resident. Returns (primary_instance_id, draft_instance_id).
/// This does NOT eject first; it is the caller's responsibility to prepare
/// the clean-room state or deliberately preserve existing residents.
pub async fn ensure_pair_loaded(
    client: &Client,
    base_url: &str,
    primary_key: &str,
    draft_key: &str,
    max_wait_secs: u64,
) -> AppResult<(String, String)> {
    // Prevent duplicate-instance bloat from repeated runs. Only one
    // resident instance per model key is needed for a clean pair.
    for target in [primary_key, draft_key] {
        let instances = list_loaded_instances(client, base_url).await?
            .into_iter()
            .filter(|(k, _)| k == target)
            .collect::<Vec<_>>();
        if instances.len() > 1 {
            tracing::warn!("ensure_pair_loaded: unloading duplicates for {} found={:?}", target, instances);
            for (_, iid) in &instances[1..] {
                let _ = client
                    .post(format!("{}/api/v1/models/unload", base_url))
                    .json(&serde_json::json!({ "instance_id": iid }))
                    .send()
                    .await;
            }
        }
    }

    // Load primary with speculative binding when a draft key was supplied.
    // Empty draft_key means clean-room mode: load primary normally.
    let primary_instances = list_loaded_instances(client, base_url)
        .await?
        .into_iter()
        .filter(|(k, _)| k == primary_key)
        .collect::<Vec<_>>();
    if primary_instances.is_empty() {
        tracing::warn!("ensure_pair_loaded: loading primary {} with draft={}", primary_key, draft_key);
        let mut payload = serde_json::json!({
            "model": primary_key,
            "context_length": 131072,
            "eval_batch_size": 2048,
            "physical_batch_size": 512,
            "parallel": 4,
            "flash_attention": true,
            "offload_kv_cache_to_gpu": true,
        });
        if !draft_key.is_empty() {
            payload
                .as_object_mut()
                .unwrap()
                .insert("speculative_draft_model".into(), serde_json::json!(draft_key));
            payload
                .as_object_mut()
                .unwrap()
                .insert("speculative_draft_simple".into(), serde_json::json!(true));
            payload
                .as_object_mut()
                .unwrap()
                .insert("speculative_draft_max_tokens".into(), serde_json::json!(64));
            payload
                .as_object_mut()
                .unwrap()
                .insert("speculative_draft_min_tokens".into(), serde_json::json!(0));
            payload
                .as_object_mut()
                .unwrap()
                .insert("speculative_draft_min_continue_probability".into(), serde_json::json!(0.75));
        }
        let resp = client
            .post(format!("{}/api/v1/models/load", base_url))
            .json(&payload)
            .timeout(std::time::Duration::from_secs(max_wait_secs))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Executor(format!(
                "LM Studio rejected speculative pair primary load for {} (HTTP {}): {}",
                primary_key, status, body
            )));
        }
    } else if primary_instances.len() > 1 {
        tracing::warn!("ensure_pair_loaded: unloading duplicate primary {} found={:?}", primary_key, primary_instances);
        for (_, iid) in &primary_instances[1..] {
            let _ = client
                .post(format!("{}/api/v1/models/unload", base_url))
                .json(&serde_json::json!({ "instance_id": iid }))
                .send()
                .await;
        }
    }

    // Load draft if absent; unload extras so only one instance remains.
    let draft_instances = list_loaded_instances(client, base_url)
        .await?
        .into_iter()
        .filter(|(k, _)| k == draft_key)
        .collect::<Vec<_>>();
    if draft_instances.is_empty() {
        tracing::warn!("ensure_pair_loaded: loading draft {}", draft_key);
        let _ = client
            .post(format!("{}/api/v1/models/load", base_url))
            .json(&serde_json::json!({ "model": draft_key }))
            .timeout(std::time::Duration::from_secs(max_wait_secs))
            .send()
            .await;
    } else if draft_instances.len() > 1 {
        tracing::warn!("ensure_pair_loaded: unloading duplicate draft {} found={:?}", draft_key, draft_instances);
        for (_, iid) in &draft_instances[1..] {
            let _ = client
                .post(format!("{}/api/v1/models/unload", base_url))
                .json(&serde_json::json!({ "instance_id": iid }))
                .send()
                .await;
        }
    }

    // Poll until both are resident.
    let start = std::time::Instant::now();
    loop {
        let raw = client
            .get(format!("{}/api/v1/models", base_url))
            .send()
            .await;
        let raw_json = match raw {
            Ok(r) => r.text().await.unwrap_or_default(),
            Err(_) => String::new(),
        };
        let loaded = list_loaded_instances(client, base_url).await?;
        let primary_inst = loaded.iter().find(|(k, _)| *k == primary_key).map(|(_, iid)| iid.clone());
        let draft_inst = loaded.iter().find(|(k, _)| *k == draft_key).map(|(_, iid)| iid.clone());

        tracing::warn!(
            target: "pair_poll",
            "ensure_pair_loaded poll primary={} draft={} found={:?} raw_prefix={}",
            primary_key, draft_key, loaded, raw_json.chars().take(180).collect::<String>()
        );

        if let (Some(pi), Some(di)) = (primary_inst.clone(), draft_inst.clone()) {
            return Ok((pi, di));
        }
        if start.elapsed().as_secs() >= max_wait_secs {
            return Err(AppError::Executor(format!(
                "Speculative pair did not become resident within {}s: primary={} draft={} (found {:?})",
                max_wait_secs, primary_key, draft_key, loaded
            )));
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Clean-room step 2: load ONLY the target model, then poll until LM Studio
/// reports it resident (state == "loaded"). Never assume readiness — verify.
///
/// Fail-fast contract: if LM Studio's own /api/v1/models/load explicitly
/// rejects the model (HTTP 4xx with an error body — e.g. "Failed to load
/// model... Error loading model", which is exactly what an in-progress
/// multi-part download or corrupt file produces), we surface THAT error
/// immediately instead of blindly polling for max_wait_secs on a model that
/// LM Studio has already told us will never become resident. Verified live
/// 2026-07-08: three step-3.7-flash quants with a sibling .gguf.part download
/// in progress each returned this exact rejection; without this fix every
/// queued run against them burned a full 300s timeout before erroring.
pub async fn ensure_loaded(
    client: &Client,
    base_url: &str,
    model_key: &str,
    max_wait_secs: u64,
) -> AppResult<bool> {
    // Preferred: explicit load endpoint.
    let load_resp = client
        .post(format!("{}/api/v1/models/load", base_url))
        .json(&serde_json::json!({
            "model": model_key,
            "context_length": 131072,
            "eval_batch_size": 2048,
            "physical_batch_size": 512,
            "parallel": 4,
            "flash_attention": true,
            "offload_kv_cache_to_gpu": true,
        }))
        .timeout(std::time::Duration::from_secs(max_wait_secs))
        .send()
        .await;

    match &load_resp {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => {
            // LM Studio answered — but rejected the model. This is a real
            // verdict, not "endpoint doesn't exist"; don't paper over it
            // with a JIT-probe retry, and don't poll for it to change.
            let status = r.status();
            // Consume the body (can't re-read `r` after this without cloning
            // the response, so we do it once here and decide based on status).
            return Err(AppError::Executor(format!(
                "LM Studio explicitly rejected loading {} (HTTP {}). The model is registered but not currently loadable — check for an in-progress download of a sibling quant blocking the model directory, or a corrupt/incomplete file.",
                model_key, status
            )));
        }
        Err(_) => {
            // Transport-level failure (endpoint missing on older LM Studio,
            // connection issue) — fall through to the JIT probe below.
        }
    }

    let explicit_load_ok = matches!(&load_resp, Ok(r) if r.status().is_success());
    if !explicit_load_ok {
        // Fallback: a 1-token chat probe triggers LM Studio's JIT loader.
        tracing::warn!("Explicit load failed for {}; falling back to JIT probe", model_key);
        let probe = client
            .post(format!("{}/api/v0/chat/completions", base_url))
            .json(&serde_json::json!({
                "model": model_key,
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 1
            }))
            .timeout(std::time::Duration::from_secs(max_wait_secs))
            .send()
            .await;
        if let Ok(r) = &probe {
            if !r.status().is_success() {
                let status = r.status();
                return Err(AppError::Executor(format!(
                    "LM Studio's JIT-load probe also rejected {} (HTTP {}) — the model cannot be loaded right now.",
                    model_key, status
                )));
            }
        }
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
/// Returns a ChatOutcome; reasoning_content is None when the model produced
/// no separate thinking trace (the overwhelming common case) — LM Studio's
/// response always carries the field (empty string when unused), so an
/// empty/missing value is normalized to None here rather than persisted as
/// a meaningless "". Token counts are read from usage.* — LM Studio's own
/// meter (electricity is the real local cost; tokens are still the honest
/// throughput measurement).
pub async fn chat(
    client: &Client,
    base_url: &str,
    model_key: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
    temperature: f32,
) -> AppResult<super::ChatOutcome> {
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
    let message = json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"));

    let content = message
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

    // Extended-thinking / chain-of-thought trace — captured separately so a
    // model's reasoning can be audited against its final answer, not just
    // the answer alone. See migration 018 for the rationale.
    let reasoning_content = message
        .and_then(|m| m.get("reasoning_content"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let (prompt_tokens, completion_tokens) = super::usage_tokens(&json);

    let speculative_decode = extract_speculative_stats(&json);

    Ok(super::ChatOutcome {
        content,
        reasoning_content,
        latency_ms: elapsed,
        prompt_tokens,
        completion_tokens,
        speculative_decode,
    })
}

fn extract_speculative_stats(json: &serde_json::Value) -> Option<super::SpeculativeDecodeStats> {
    let draft_model = json
        .get("draft_model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let usage = json.get("usage")?;
    let total_draft_tokens_count = usage
        .get("total_draft_tokens_count")
        .and_then(|v| v.as_i64())
        .filter(|n| *n >= 0);
    let accepted_draft_tokens_count = usage
        .get("accepted_draft_tokens_count")
        .and_then(|v| v.as_i64())
        .filter(|n| *n >= 0);
    let rejected_draft_tokens_count = usage
        .get("rejected_draft_tokens_count")
        .and_then(|v| v.as_i64())
        .filter(|n| *n >= 0);

    if draft_model.is_none()
        && total_draft_tokens_count.is_none()
        && accepted_draft_tokens_count.is_none()
        && rejected_draft_tokens_count.is_none()
    {
        return None;
    }

    Some(super::SpeculativeDecodeStats {
        draft_model,
        total_draft_tokens_count,
        accepted_draft_tokens_count,
        rejected_draft_tokens_count,
    })
}
