//! Cloud executor — fires OpenAI-compatible chat completions at Nous / OpenRouter.
//! Same message shape as the local path so tests are provider-agnostic.
use reqwest::Client;
use std::time::{Duration, Instant};

use crate::error::{AppError, AppResult};

fn endpoint_for(provider: &str) -> AppResult<&'static str> {
    match provider {
        "openrouter" => Ok("https://openrouter.ai/api/v1/chat/completions"),
        "nous" => Ok("https://inference-api.nousresearch.com/v1/chat/completions"),
        other => Err(AppError::Executor(format!("Unknown provider: {}", other))),
    }
}

/// Resolve the API key for a cloud provider at RUN time (not process start).
///
/// Order: explicit env-derived key from Config, then — for Nous — the live
/// Hermes OAuth agent key in ~/.hermes/auth.json. That token rotates (hours),
/// so reading it fresh per run self-heals across rotations; a key snapshotted
/// at service start would silently expire. Read-only access, never logged.
pub fn resolve_api_key(provider: &str, config_key: &Option<String>) -> AppResult<String> {
    if let Some(k) = config_key {
        return Ok(k.clone());
    }
    if provider == "nous" {
        let path = dirs_home().join(".hermes/auth.json");
        let raw = std::fs::read_to_string(&path).map_err(|e| {
            AppError::Executor(format!(
                "No NOUS_API_KEY env and {} unreadable: {}",
                path.display(),
                e
            ))
        })?;
        let json: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| AppError::Executor(format!("auth.json parse error: {}", e)))?;
        if let Some(key) = json
            .pointer("/providers/nous/agent_key")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            return Ok(key.to_string());
        }
        return Err(AppError::Executor(
            "auth.json has no providers.nous.agent_key — run `hermes setup --portal`".into(),
        ));
    }
    Err(AppError::Executor(format!(
        "No API key configured for provider '{}' (set NOUS_API_KEY / OPENROUTER_API_KEY)",
        provider
    )))
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/"))
}

/// Execute one chat completion against a cloud provider.
/// Returns (content, latency_ms).
pub async fn chat(
    client: &Client,
    provider: &str,
    api_key: &str,
    model: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
) -> AppResult<(String, u64)> {
    let endpoint = endpoint_for(provider)?;
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": 0.0,
    });

    let start = Instant::now();
    let resp = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .timeout(Duration::from_secs(120))
        .send()
        .await?;
    let elapsed = start.elapsed().as_millis() as u64;

    let status = resp.status();
    let json: serde_json::Value = resp.json().await?;

    if !status.is_success() {
        return Err(AppError::Executor(format!(
            "{} returned HTTP {}: {}",
            provider,
            status,
            &json.to_string().chars().take(300).collect::<String>()
        )));
    }

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
                "{} returned no content (raw: {})",
                provider,
                &json.to_string().chars().take(300).collect::<String>()
            ))
        })?;

    Ok((content, elapsed))
}
