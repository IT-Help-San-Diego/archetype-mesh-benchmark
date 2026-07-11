//! Speculative Decode panel — pair discovery + live timing test.
//!
//! Two endpoints:
//!   GET  /api/spec-decode/pairs   — configured pairs + live LM Studio state
//!   POST /api/spec-decode/test    — run a timing comparison for one pair
//!
//! Why this lives here instead of in the benchmark executor:
//! The executor enforces single-model isolation (memory guard, clean-room
//! semantics). Speculative decoding requires BOTH models resident. This
//! module bypasses the executor and talks directly to LM Studio's own
//! /v1/chat/completions endpoint, then restores config afterward.
use axum::{
    extract::State,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use walkdir::WalkDir;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

// data shapes

#[derive(Debug, Serialize)]
pub struct SpecPair {
    pub main_model: String,
    pub draft_model: String,
    pub draft_source: DraftSource,
    pub main_loaded: bool,
    pub draft_loaded: bool,
    pub spec_active: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum DraftSource {
    Simple,
    Mtp,
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct PairsResponse {
    pub pairs: Vec<SpecPair>,
    pub lmstudio_connected: bool,
    pub lmstudio_base_url: String,
}

#[derive(Debug, Deserialize)]
pub struct TestRequest {
    pub main_model: String,
    pub prompt: Option<String>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct TestResult {
    pub main_model: String,
    pub draft_model: String,
    pub with_draft: TimingRun,
    pub without_draft: TimingRun,
    pub speedup_ratio: f64,
    pub draft_was_active: bool,
    pub verdict: String,
}

#[derive(Debug, Serialize)]
pub struct TimingRun {
    pub elapsed_secs: f64,
    pub total_tokens: u32,
    pub completion_tokens: u32,
    pub tokens_per_sec: f64,
}

// config scanning

const LMSTUDIO_CONFIG_DIR: &str =
    ".lmstudio/.internal/user-concrete-model-default-config";

fn scan_draft_pairs() -> Vec<SpecPair> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let config_root = home.join(LMSTUDIO_CONFIG_DIR);
    let mut pairs = Vec::new();

    for entry in WalkDir::new(&config_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if path.file_name().and_then(|s| s.to_str()).map(|n| n.ends_with(".json.bak")) == Some(true) {
            continue;
        }

        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&raw) {
                let fields = cfg
                    .get("load")
                    .and_then(|l| l.get("fields"))
                    .and_then(|f| f.as_array());

                let mut draft_model = None;
                let mut draft_source = DraftSource::Unknown;

                if let Some(fields) = fields {
                    for f in fields {
                        let key = f.get("key").and_then(|k| k.as_str()).unwrap_or("");
                        let value = f.get("value");
                        match key {
                            "llm.load.llama.speculativeDecoding.draftModel" => {
                                draft_model = value.and_then(|v| v.as_str()).map(|s| s.to_string());
                                let mtp = fields.iter().any(|f2| {
                                    f2.get("key").and_then(|k| k.as_str())
                                        == Some("llm.load.llama.speculativeDecoding.draftMtp")
                                        && f2.get("value") == Some(&serde_json::json!(true))
                                });
                                draft_source = if mtp { DraftSource::Mtp } else { DraftSource::Simple };
                            }
                            _ => {}
                        }
                    }
                }

                if let Some(draft) = draft_model {
                    let main_model = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    pairs.push(SpecPair {
                        main_model,
                        draft_model: draft,
                        draft_source,
                        main_loaded: false,
                        draft_loaded: false,
                        spec_active: false,
                        reason: None,
                    });
                }
            }
        }
    }

    pairs
}

// LM Studio API helpers

fn normalize_for_match(s: &str) -> String {
    let lower = s.to_lowercase();
    let no_pub = lower.split('/').last().unwrap_or(&lower);
    let no_ext = no_pub.strip_suffix(".gguf").unwrap_or(no_pub);
    let no_ext = no_ext.strip_suffix(".gguf.bak").unwrap_or(no_ext);
    no_ext
        .replace("@", "-")
        .replace(".", "-")
}

async fn fetch_ls_models(base_url: &str) -> AppResult<Vec<serde_json::Value>> {
    let url = format!("{}/api/v0/models", base_url);
    let resp = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(10))
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Executor(format!(
            "LM Studio HTTP {} from {} — {}",
            status, url, body
        )));
    }

    let json: serde_json::Value = resp.json().await?;
    Ok(json.get("data").and_then(|d| d.as_array()).map(|a| a.clone()).unwrap_or_default())
}

// endpoints

pub async fn spec_decode_pairs(State(state): State<AppState>) -> AppResult<Json<PairsResponse>> {
    let base_url = state.config.lmstudio_base_url.clone();
    let mut pairs = scan_draft_pairs();

    match fetch_ls_models(&base_url).await {
        Ok(models) => {
            let loaded_ids: Vec<String> = models
                .iter()
                .filter(|m| m.get("state").and_then(|s| s.as_str()) == Some("loaded"))
                .filter_map(|m| m.get("id").and_then(|s| s.as_str()))
                .map(|s| s.to_string())
                .collect();

            for pair in &mut pairs {
                let main_norm = normalize_for_match(&pair.main_model);
                let draft_norm = normalize_for_match(&pair.draft_model);
                let main_loaded = loaded_ids.iter().any(|id| normalize_for_match(id) == main_norm);
                let draft_loaded = loaded_ids.iter().any(|id| normalize_for_match(id) == draft_norm);

                pair.main_loaded = main_loaded;
                pair.draft_loaded = draft_loaded;
                pair.spec_active = main_loaded && draft_loaded;
                pair.reason = if main_loaded && draft_loaded {
                    None
                } else if !main_loaded {
                    Some(format!("Main model not loaded (LM Studio has: {})", loaded_ids.join(", ")))
                } else {
                    Some("Draft model not loaded".to_string())
                };
            }
        }
        Err(_) => {
            for pair in &mut pairs {
                pair.reason = Some(format!("LM Studio unreachable at {}", base_url));
            }
        }
    }

    Ok(Json(PairsResponse {
        lmstudio_connected: true,
        lmstudio_base_url: base_url,
        pairs,
    }))
}

pub async fn spec_decode_test(
    State(state): State<AppState>,
    Json(req): Json<TestRequest>,
) -> AppResult<Json<TestResult>> {
    let base_url = state.config.lmstudio_base_url.clone();

    let pairs = scan_draft_pairs();
    let pair = pairs
        .iter()
        .find(|p| {
            let exact = req.main_model == format!("{}-qat", p.main_model)
                || req.main_model == p.main_model
                || p.main_model.contains(&req.main_model)
                || req.main_model.contains(&p.main_model);
            exact
        })
        .ok_or_else(|| AppError::Executor(format!(
            "No speculative-decode pair configured for model '{}'. Check LM Studio persistent config (user-concrete-model-default-config/).",
            req.main_model
        )))?;

    let test_prompt = req.prompt.unwrap_or_else(|| {
        "Explain why speculative decoding speeds up inference in exactly one sentence.".to_string()
    });
    let max_tokens = req.max_tokens.unwrap_or(80);

    let with_draft = run_timing(&base_url, &pair.main_model, &test_prompt, max_tokens).await?;

    let without_draft = if pair.draft_loaded {
        run_without_draft(&base_url, &pair.main_model, &test_prompt, max_tokens).await?
    } else {
        run_timing(&base_url, &pair.main_model, &test_prompt, max_tokens).await?
    };

    let speedup = if without_draft.tokens_per_sec > 0.0 {
        with_draft.tokens_per_sec / without_draft.tokens_per_sec
    } else {
        0.0
    };

    let verdict = if with_draft.tokens_per_sec > without_draft.tokens_per_sec {
        let pct = ((speedup - 1.0) * 100.0).round() as i32;
        format!("[OK] Spec-decode active: {}% faster", pct)
    } else if (with_draft.tokens_per_sec - without_draft.tokens_per_sec).abs() < 1.0 {
        "[--] No measurable difference -- draft may not be active".to_string()
    } else {
        let pct = ((1.0 - speedup) * 100.0).round() as i32;
        format!("[SLOW] Slower with draft: {}% slower", pct)
    };

    Ok(Json(TestResult {
        main_model: pair.main_model.clone(),
        draft_model: pair.draft_model.clone(),
        with_draft,
        without_draft,
        speedup_ratio: speedup,
        draft_was_active: pair.spec_active,
        verdict,
    }))
}

async fn run_timing(base_url: &str, model: &str, prompt: &str, max_tokens: u32) -> AppResult<TimingRun> {
    let payload = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": max_tokens,
    });

    let start = std::time::Instant::now();
    let resp = reqwest::Client::new()
        .post(&format!("{}/v1/chat/completions", base_url))
        .json(&payload)
        .timeout(Duration::from_secs(120))
        .send()
        .await?;

    let elapsed = start.elapsed().as_secs_f64();
    let status = resp.status();

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Executor(format!(
            "LM Studio completion failed: HTTP {} -- {}",
            status, body
        )));
    }

    let result: serde_json::Value = resp.json().await?;
    let usage = result.get("usage").cloned().unwrap_or_default();

    let completion_tokens = usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let total_tokens = usage.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let tokens_per_sec = if elapsed > 0.0 { completion_tokens as f64 / elapsed } else { 0.0 };

    Ok(TimingRun {
        elapsed_secs: elapsed,
        total_tokens,
        completion_tokens,
        tokens_per_sec,
    })
}

async fn run_without_draft(
    base_url: &str,
    model: &str,
    prompt: &str,
    max_tokens: u32,
) -> AppResult<TimingRun> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let config_path = home.join(LMSTUDIO_CONFIG_DIR).join(&format!("{}.json", model));

    let original_config = std::fs::read_to_string(&config_path).map_err(|e| {
        AppError::Executor(format!("Failed to read LM Studio config for {}: {}", model, e))
    })?;

    let mut cfg: serde_json::Value = serde_json::from_str(&original_config).map_err(|e| {
        AppError::Executor(format!("Failed to parse config: {}", e))
    })?;

    if let Some(load) = cfg.get_mut("load") {
        if let Some(fields) = load.get_mut("fields").and_then(|f| f.as_array_mut()) {
            fields.retain(|f| {
                let key = f.get("key").and_then(|k| k.as_str()).unwrap_or("");
                !key.contains("speculativeDecoding")
            });
        }
    }

    let temp_config = serde_json::to_string_pretty(&cfg).map_err(|e| {
        AppError::Executor(format!("Failed to serialize temp config: {}", e))
    })?;

    std::fs::write(&config_path, &temp_config).map_err(|e| {
        AppError::Executor(format!("Failed to write temp config: {}", e))
    })?;

    let result = run_timing(base_url, model, prompt, max_tokens).await;

    let _ = std::fs::write(&config_path, &original_config);

    result
}

// router mount

pub fn router() -> Router<crate::state::AppState> {
    Router::new()
        .route("/api/spec-decode/pairs", get(spec_decode_pairs))
        .route("/api/spec-decode/test", post(spec_decode_test))
}
