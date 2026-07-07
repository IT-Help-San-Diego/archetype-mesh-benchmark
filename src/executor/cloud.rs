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
