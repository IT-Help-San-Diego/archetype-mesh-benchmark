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
    #[serde(rename = "state", default)]
    pub load_state: String,
    #[serde(rename = "loaded_instances", default)]
    pub loaded_instances: Vec<serde_json::Value>,
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

/// Eject every loaded instance EXCEPT `keep_key`. Used by clean-room mode
/// when the target model is already resident: we keep the target in place
/// (patience principle — don't tear down a working large model just to
/// re-load it) and only clear other models for isolation.
pub async fn eject_others(
    client: &Client,
    base_url: &str,
    keep_key: &str,
) -> AppResult<Vec<String>> {
    let resp = client
        .get(format!("{}/api/v1/models", base_url))
        .send()
        .await?
        .error_for_status()?;
    let v: serde_json::Value = resp.json().await?;

    let mut ejected = Vec::new();
    if let Some(models) = v.get("models").and_then(|m| m.as_array()) {
        for m in models {
            let key = m.get("key").and_then(|k| k.as_str()).unwrap_or("");
            if key == keep_key {
                continue; // keep the target resident
            }
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
                        }
                    }
                }
            }
        }
    }
    Ok(ejected)
}

/// pairs for every resident instance. Used by speculative-pair mode to
/// verify both primary and draft models are loaded simultaneously.
pub async fn list_loaded_instances(
    client: &Client,
    base_url: &str,
) -> AppResult<Vec<(String, String)>> {
    let resp = client
        .get(format!("{}/api/v1/models", base_url))
        .send()
        .await?
        .error_for_status()?;
    let v: serde_json::Value = resp.json().await?;

    let mut out = Vec::new();
    if let Some(models) = v.get("models").and_then(|m| m.as_array()) {
        for m in models {
            let key = m
                .get("key")
                .and_then(|i| i.as_str())
                .or_else(|| m.get("id").and_then(|i| i.as_str()))
                .unwrap_or("");
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
            let key = m
                .get("key")
                .and_then(|i| i.as_str())
                .or_else(|| m.get("id").and_then(|i| i.as_str()))
                .unwrap_or("");
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
        let instances = list_loaded_instances(client, base_url)
            .await?
            .into_iter()
            .filter(|(k, _)| k == target)
            .collect::<Vec<_>>();
        if instances.len() > 1 {
            tracing::warn!(
                "ensure_pair_loaded: unloading duplicates for {} found={:?}",
                target,
                instances
            );
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
        tracing::warn!(
            "ensure_pair_loaded: loading primary {} with draft={}",
            primary_key,
            draft_key
        );
        let mut payload = serde_json::json!({
            "model": primary_key,
            "context_length": 131072,
            "eval_batch_size": 4096,
            "physical_batch_size": 1024,
            "parallel": 4,
            "flash_attention": true,
            "offload_kv_cache_to_gpu": true,
        });
        if !draft_key.is_empty() {
            payload.as_object_mut().unwrap().insert(
                "speculative_draft_model".into(),
                serde_json::json!(draft_key),
            );
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
            payload.as_object_mut().unwrap().insert(
                "speculative_draft_min_continue_probability".into(),
                serde_json::json!(0.75),
            );
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
        tracing::warn!(
            "ensure_pair_loaded: unloading duplicate primary {} found={:?}",
            primary_key,
            primary_instances
        );
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
        tracing::warn!(
            "ensure_pair_loaded: unloading duplicate draft {} found={:?}",
            draft_key,
            draft_instances
        );
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
        let primary_inst = loaded
            .iter()
            .find(|(k, _)| *k == primary_key)
            .map(|(_, iid)| iid.clone());
        let draft_inst = loaded
            .iter()
            .find(|(k, _)| *k == draft_key)
            .map(|(_, iid)| iid.clone());

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
/// Read LM Studio's stored per-model default load config for `model_key`.
///
/// LM Studio persists the user's GUI-tuned load settings to
/// `~/.lmstudio/.internal/user-concrete-model-default-config/<path>.json`
/// as `{ preset, operation, load: { fields: [{key,value}] } }` where keys
/// are *dotted* engine paths (e.g. `llm.load.contextLength`,
/// `llm.load.numExperts`). We translate the ones our load endpoint
/// understands into flat API keys (`context_length`, `num_experts`, ...)
/// and return them so a known-good, model-specific config is never
/// overridden by our generic preset profile. Stored values win on conflict.
///
/// Returns `None` if the file is absent/unreadable or has no useful params.
pub fn read_stored_model_default(model_key: &str) -> Option<serde_json::Value> {
    let home = std::env::var("HOME").ok()?;
    let candidates = [
        format!(
            "{}/.lmstudio/.internal/user-concrete-model-default-config/{}.json",
            home, model_key
        ),
        format!(
            "{}/.lmstudio/.internal/user-concrete-model-default-config/{}.json",
            home,
            model_key.replace('/', "-")
        ),
    ];
    // dotted LM Studio engine key -> flat load-API key.
    // NOTE: only map keys the /api/v1/models/load endpoint accepts.
    // `cpu_thread_pool_size` (llm.load.llama.cpuThreadPoolSize) is a valid
    // engine field but the LOAD endpoint rejects it as an "unrecognized
    // key" — so we deliberately do NOT map it, letting LM Studio use its
    // default. Mapping it caused HTTP 400 on every large-model load.
    let translate = |k: &str| -> Option<&'static str> {
        match k {
            "llm.load.contextLength" => Some("context_length"),
            "llm.load.numExperts" | "llm.load.llama.num_experts" => Some("num_experts"),
            "llm.load.evalBatchSize" => Some("eval_batch_size"),
            "llm.load.physicalBatchSize" => Some("physical_batch_size"),
            "llm.load.parallel" => Some("parallel"),
            "llm.load.flashAttention" => Some("flash_attention"),
            "llm.load.offloadKvCacheToGpu" => Some("offload_kv_cache_to_gpu"),
            _ => None,
        }
    };
    for path in candidates {
        if let Ok(s) = std::fs::read_to_string(&path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                if let Some(load) = v.get("load").and_then(|l| l.get("fields")) {
                    if let Some(arr) = load.as_array() {
                        let mut merged = serde_json::Map::new();
                        for f in arr {
                            if let (Some(k), Some(val)) = (
                                f.get("key").and_then(|x| x.as_str()),
                                f.get("value"),
                            ) {
                                if let Some(api_key) = translate(k) {
                                    merged.insert(api_key.to_string(), val.clone());
                                }
                            }
                        }
                        if !merged.is_empty() {
                            return Some(serde_json::Value::Object(merged));
                        }
                    }
                }
            }
        }
    }
    None
}

pub async fn ensure_loaded(
    client: &Client,
    base_url: &str,
    model_key: &str,
    preset: &crate::routes::runs::LoadPreset,
    draft_model: Option<&str>,
    max_wait_secs: u64,
) -> AppResult<bool> {
    // PATIENCE PRINCIPLE: if the model is already resident (loaded by the
    // user's GUI, a prior run, or anything else), USE IT. Never re-POST a
    // load — re-loading a large model thrashes RAM and can abort the engine
    // on slow / memory-constrained hardware. Verify residency first.
    //
    // Residency signal: LM Studio reports a loaded model either via
    // `state: "loaded"` (v0 /models) or a non-empty `loaded_instances`
    // list (v1 /models). Check both so neither endpoint's shape hides a
    // resident model from us.
    {
        let models = list_ls_models(client, base_url).await?;
        let resident = models.iter().any(|m| {
            m.id == model_key
                && (m.load_state == "loaded" || !m.loaded_instances.is_empty())
        });
        if resident {
            tracing::info!("ensure_loaded: {} already resident — using it", model_key);
            return Ok(true);
        }
    }

    // Build the load body from the requested preset, then MERGE the model's
    // stored LM Studio per-model defaults (the user-maintained
    // user-concrete-model-default-config/*.json). This preserves
    // model-specific fields our preset template omits — critically
    // `num_experts` for MoE models (e.g. gpt-oss-120b), which LM Studio
    // needs or the engine startup aborts. Stored values win on conflict so
    // a known-good config is never overridden by our generic profile.
    let mut load_body = preset.to_load_json(model_key, draft_model);
    if let Some(stored) = read_stored_model_default(model_key) {
        if let Some(obj) = stored.as_object() {
            for (k, v) in obj {
                // Only merge engine fields we don't already control
                // explicitly; never clobber model/draft identity.
                if k != "model" && k != "speculative_draft_model" {
                    load_body[k.clone()] = v.clone();
                }
            }
        }
        tracing::debug!("ensure_loaded: merged stored defaults for {}", model_key);
    }

    let load_resp = client
        .post(format!("{}/api/v1/models/load", base_url))
        .json(&load_body)
        .timeout(std::time::Duration::from_secs(max_wait_secs))
        .send()
        .await;

    match &load_resp {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => {
            let status = r.status();
            return Err(AppError::Executor(format!(
                "LM Studio explicitly rejected loading {} (HTTP {}). The model is registered but not currently loadable — check for an in-progress download of a sibling quant blocking the model directory, or a corrupt/incomplete file.",
                model_key, status
            )));
        }
        Err(_) => {}
    }

    let explicit_load_ok = matches!(&load_resp, Ok(r) if r.status().is_success());
    if !explicit_load_ok {
        tracing::warn!(
            "Explicit load failed for {}; falling back to JIT probe",
            model_key
        );
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
    // PER-TRIAL TIMEOUT: a single chat call must answer within 90s on this
    // hardware. A model that cannot return within that window is broken or
    // pathologically slow — fail fast rather than hang the whole run (and
    // every queued run behind it). 300s was too long: a silent/empty model
    // could block 102 trials x 300s = hours of frozen executor.
    let resp = client
        .post(format!("{}/api/v0/chat/completions", base_url))
        .json(&body)
        .timeout(std::time::Duration::from_secs(90))
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
        .map(|s| s.to_string());

    // EMPTY CONTENT = non-functional model. A model returning "" (not
    // missing, but blank) is broken — treat it as an infrastructure failure
    // so the run aborts instead of scoring 102 silent zeros across hours.
    let content = match content {
        Some(c) if !c.trim().is_empty() => c,
        _ => {
            return Err(AppError::Executor(format!(
                "LM Studio returned empty content for {} (model is non-functional / not answering) — aborting trial. Raw: {}",
                model_key,
                &json.to_string().chars().take(300).collect::<String>()
            )));
        }
    };

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
