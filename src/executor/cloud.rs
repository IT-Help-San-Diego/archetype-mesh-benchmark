//! Cloud executor — fires OpenAI-compatible chat completions at Nous / OpenRouter.
//! Same message shape as the local path so tests are provider-agnostic.
use reqwest::Client;
use std::time::{Duration, Instant};

use crate::error::{AppError, AppResult};

/// Chat-completions endpoint per provider. `pub` because fountain probes
/// (routes::fountain) fire raw requests to read HTTP status + Retry-After —
/// evidence cloud::chat's error path doesn't surface.
pub fn endpoint_for(provider: &str) -> AppResult<&'static str> {
    match provider {
        "openrouter" => Ok("https://openrouter.ai/api/v1/chat/completions"),
        "nous" => Ok("https://inference-api.nousresearch.com/v1/chat/completions"),
        "openai" => Ok("https://api.openai.com/v1/chat/completions"),
        // Gemini is NOT OpenAI-compatible: it uses the generativelanguage
        // v1beta generateContent shape (contents[].parts[]). The model id is
        // appended as a path segment + key query param (see chat() below).
        "gemini" => Ok("https://generativelanguage.googleapis.com/v1beta/models"),
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
    if provider == "gemini" {
        // Gemini key is supplied by the deployer via GEMINI_API_KEY env (the
        // Config.gemini_api_key field). It must be present — there is no
        // secondary resolution path, by design (no shared/rotating token).
        return Err(AppError::Executor(
            "No GEMINI_API_KEY env set — supply your own Google AI Studio key (free tier, no prepay).".into(),
        ));
    }
    Err(AppError::Executor(format!(
        "No API key configured for provider '{}' (set NOUS_API_KEY / OPENROUTER_API_KEY / OPENAI_API_KEY / GEMINI_API_KEY)",
        provider
    )))
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/"))
}

/// Execute one chat completion against a cloud provider.
/// Returns a ChatOutcome — see lmstudio::chat for the reasoning_content
/// contract (None when no trace was produced). Token counts come from the
/// provider's usage object: their own billing meter, read back verbatim.
pub async fn chat(
    client: &Client,
    provider: &str,
    api_key: &str,
    model: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
) -> AppResult<super::ChatOutcome> {
    let endpoint = endpoint_for(provider)?;
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": 0.0,
    });

    let start = Instant::now();
    // Hard watchdog: a stalled cloud provider (Nous 400 "missing user tag"
    // on free models, hung TLS, slow endpoint) must NOT hang the run forever
    // — a hung call would leave the run stuck in `running` at 0/0 and block
    // all future runs of that model. Bound the whole request at 90s; on
    // timeout treat as infrastructure error and let the trial loop continue.
    let send_fut = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .timeout(Duration::from_secs(120))
        .send();
    let resp = match tokio::time::timeout(Duration::from_secs(90), send_fut).await {
        Ok(inner) => inner?,
        Err(_) => {
            return Err(AppError::Executor(format!(
                "{} request timed out after 90s (stalled provider connection)",
                provider
            )))
        }
    };
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

    let message = json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"));

    // Provider's own billing meter, read back verbatim (None if omitted).
    let (usage_prompt, usage_completion) = super::usage_tokens(&json);

    // Primary: the content field (the model's committed answer).
    let content = message
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Fallback: some cloud models (GLM-5.2, Fable 5 with extended thinking)
    // spend ALL their token budget on the reasoning/thinking trace and never
    // emit a content field — finish_reason is "length" and content is null.
    // In that case, the LAST token of the reasoning trace IS the answer
    // (e.g., the reasoning ends with "...therefore VALID" — VALID is the
    // committed answer, just never separated into the content field because
    // the token budget ran out mid-thought). This is the exact failure mode
    // found live on 2026-07-09: GLM-5.2 run 162, LOGIC-06 trial 1 — the model
    // reasoned correctly but the 512-token budget was consumed before the
    // final "VALID" could land in the content field.
    //
    // The reasoning field name varies by provider:
    //   - LM Studio: "reasoning_content"
    //   - Nous (GLM): "reasoning"
    //   - Nous (Claude): "reasoning_content"
    //   - OpenRouter: varies — check both
    let reasoning_content = message
        .and_then(|m| {
            m.get("reasoning_content")
                .or_else(|| m.get("reasoning"))
        })
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // If content is null but reasoning exists, try to extract the committed
    // answer from the tail of the reasoning trace (last alphanumeric token,
    // same logic as the local scoring path — first-token OR final-run match).
    let content = match content {
        Some(c) if !c.is_empty() => c,
        _ => {
            // Content was empty/null — see if the reasoning trace contains
            // the answer as its final token (VALID/INVALID/TRUE/FALSE/etc.)
            if let Some(ref r) = reasoning_content {
                let last_token = r
                    .split(|c: char| !c.is_ascii_alphanumeric()).rfind(|t| !t.is_empty())
                    .unwrap_or("");
                if !last_token.is_empty() {
                    tracing::warn!(
                        "Cloud model {} returned null content but reasoning trace ends with '{}' — extracting committed answer from reasoning tail",
                        model, last_token
                    );
                    last_token.to_string()
                } else {
                    return Err(AppError::Executor(format!(
                        "{} returned no content and reasoning trace had no extractable answer (raw: {})",
                        provider,
                        &json.to_string().chars().take(300).collect::<String>()
                    )));
                }
            } else {
                return Err(AppError::Executor(format!(
                    "{} returned no content and no reasoning trace (raw: {})",
                    provider,
                    &json.to_string().chars().take(300).collect::<String>()
                )));
            }
        }
    };

    Ok(super::ChatOutcome {
        content,
        reasoning_content,
        latency_ms: elapsed,
        prompt_tokens: usage_prompt,
        completion_tokens: usage_completion,
        speculative_decode: None,
    })
}

/// Gemini-native chat completion.
///
/// Gemini's REST API is NOT OpenAI-compatible: it expects `contents[]` with
/// `parts[]` where each part is either `{text}` or `{inline_data:{mime_type,
/// data(base64)}}`. The benchmark builds OpenAI-shaped `messages`
/// (role + content[text | image_url]), so we translate here. Vision tests
/// embed the image as `image_url` with a `data:image/png;base64,...` URL —
/// we decode the base64 and forward it as `inline_data`. Returns the same
/// `ChatOutcome` as the OpenAI path so the executor is provider-agnostic.
pub async fn gemini_chat(
    client: &Client,
    api_key: &str,
    model: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
) -> AppResult<super::ChatOutcome> {
    // Translate OpenAI messages -> Gemini contents (role: user|model).
    let mut contents: Vec<serde_json::Value> = Vec::new();
    for msg in messages {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        let gemini_role = if role == "assistant" { "model" } else { "user" };
        let mut parts: Vec<serde_json::Value> = Vec::new();
        match msg.get("content") {
            // Multimodal: content is an array of {type:text|image_url}
            Some(serde_json::Value::Array(arr)) => {
                for part in arr {
                    if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                        parts.push(serde_json::json!({"text": t}));
                    } else if let Some(obj) = part.get("image_url") {
                        if let Some(url) = obj.get("url").and_then(|v| v.as_str()) {
                            // url form: "data:image/png;base64,<DATA>"
                            if let Some(b64) = url.split("base64,").nth(1) {
                                parts.push(serde_json::json!({
                                    "inline_data": {"mime_type": "image/png", "data": b64}
                                }));
                            }
                        }
                    }
                }
            }
            // Plain string content
            Some(serde_json::Value::String(s)) => {
                parts.push(serde_json::json!({"text": s}));
            }
            _ => {}
        }
        if !parts.is_empty() {
            contents.push(serde_json::json!({"role": gemini_role, "parts": parts}));
        }
    }

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );
    let body = serde_json::json!({
        "contents": contents,
        "generationConfig": {"maxOutputTokens": max_tokens, "temperature": 0.0}
    });

    let start = Instant::now();
    let send_fut = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(Duration::from_secs(120))
        .send();
    let resp = match tokio::time::timeout(Duration::from_secs(90), send_fut).await {
        Ok(inner) => inner?,
        Err(_) => {
            return Err(AppError::Executor(
                "gemini request timed out after 90s (stalled provider connection)".into(),
            ))
        }
    };
    let elapsed = start.elapsed().as_millis() as u64;

    let status = resp.status();
    let json: serde_json::Value = resp.json().await?;

    if !status.is_success() {
        return Err(AppError::Executor(format!(
            "gemini returned HTTP {}: {}",
            status,
            &json.to_string().chars().take(400).collect::<String>()
        )));
    }

    // Gemini: candidates[0].content.parts[].text (may be multiple parts).
    let text = json
        .pointer("/candidates/0/content/parts")
        .and_then(|p| p.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|s: &String| !s.is_empty());

    let content = match text {
        Some(c) => c,
        None => {
            // Surface the raw error detail if present (e.g. safety / quota).
            let detail = json
                .pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("no content and no error message");
            return Err(AppError::Executor(format!(
                "gemini returned no text content (raw: {})",
                &detail.chars().take(300).collect::<String>()
            )));
        }
    };

    // Token accounting: Gemini reports usageMetadata.promptTokenCount /
    // candidatesTokenCount. Map to the same ChatOutcome fields (i64).
    let (prompt_tokens, completion_tokens) = (
        json.pointer("/usageMetadata/promptTokenCount")
            .and_then(|v| v.as_u64())
            .or_else(|| json.pointer("/usageMetadata/prompt_tokens").and_then(|v| v.as_u64()))
            .map(|v| v as i64),
        json.pointer("/usageMetadata/candidatesTokenCount")
            .and_then(|v| v.as_u64())
            .or_else(|| json.pointer("/usageMetadata/candidates_tokens").and_then(|v| v.as_u64()))
            .map(|v| v as i64),
    );

    Ok(super::ChatOutcome {
        content,
        reasoning_content: None,
        latency_ms: elapsed,
        prompt_tokens,
        completion_tokens,
        speculative_decode: None,
    })
}

