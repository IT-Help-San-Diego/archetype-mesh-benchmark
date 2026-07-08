//! GET /api/prompt-check — prompt length validation against a model's context window.
//!
//! Default: instant heuristic estimate, ZERO inference cost, works for local
//! and cloud models alike.
//! Optional `?live=true` (local models only): fires one real max_tokens=1
//! completion at LM Studio and reads the EXACT token count back from
//! `usage.prompt_tokens` — genuine inference cost, opt-in only. LM Studio's
//! REST API has no standalone tokenizer endpoint (verified empirically), so
//! this is the only way to get an exact number; we ask before spending it.
use axum::extract::{Query, State};
use axum::response::Json;
use serde::Deserialize;

use crate::error::AppResult;
use crate::executor;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PromptCheckQuery {
    pub model_key: String,
    pub prompt: String,
    #[serde(default)]
    pub live: bool,
}

#[derive(sqlx::FromRow)]
struct ModelCtx {
    context_length: i32,
    location: String,
}

pub async fn prompt_check(
    State(state): State<AppState>,
    Query(q): Query<PromptCheckQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let model: Option<ModelCtx> =
        sqlx::query_as("SELECT context_length, location FROM models WHERE key = $1")
            .bind(&q.model_key)
            .fetch_optional(&state.db)
            .await?;

    let Some(model) = model else {
        return Ok(Json(serde_json::json!({
            "error": format!("Unknown model key: {}", q.model_key)
        })));
    };

    let (tokens, limit, fits, note, exact) = if q.live && model.location == "local" {
        // Same gate every LM Studio-touching route uses (see lm_guard.rs) —
        // this call used to fire completely unserialized; that was the
        // real self-harm gap, not a hypothetical one.
        let _permit = crate::lm_guard::acquire().await;
        let client = reqwest::Client::new();
        match executor::verify_prompt_length_live(
            &client,
            &state.config.lmstudio_base_url,
            &q.model_key,
            &q.prompt,
            model.context_length as i64,
        )
        .await
        {
            Ok((t, l, f, n)) => (t, l, f, n, true),
            Err(e) => {
                // Overflow errors ARE the answer — surface as a normal (not 500) result.
                let (t, l, f, n) = executor::validate_prompt_length(&q.prompt, model.context_length as i64);
                return Ok(Json(serde_json::json!({
                    "model_key": q.model_key, "tokens": t, "context_limit": l, "fits": f,
                    "percent_used": if l > 0 { (t as f64 / l as f64 * 1000.0).round() / 10.0 } else { 0.0 },
                    "note": format!("Live check failed ({}) — showing heuristic estimate instead: {}", e, n),
                    "exact": false,
                })));
            }
        }
    } else {
        let (t, l, f, n) = executor::validate_prompt_length(&q.prompt, model.context_length as i64);
        (t, l, f, n, false)
    };

    let pct = if limit > 0 { (tokens as f64 / limit as f64 * 1000.0).round() / 10.0 } else { 0.0 };

    Ok(Json(serde_json::json!({
        "model_key": q.model_key,
        "tokens": tokens,
        "context_limit": limit,
        "fits": fits,
        "percent_used": pct,
        "note": note,
        "exact": exact,
        "live_available": model.location == "local",
    })))
}

// POST /api/prompt-check — run a prompt against a model and return the response
pub async fn prompt_check_post(
    State(state): State<AppState>,
    axum::extract::Json(req): axum::extract::Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let Some(model_key) = req.get("model_key").and_then(|v| v.as_str()) else {
        return Ok(Json(serde_json::json!({ "error": "Missing model_key" })));
    };
    let Some(prompt) = req.get("prompt").and_then(|v| v.as_str()) else {
        return Ok(Json(serde_json::json!({ "error": "Missing prompt" })));
    };

    let model: Option<ModelCtx> =
        sqlx::query_as("SELECT context_length, location FROM models WHERE key = $1")
            .bind(model_key)
            .fetch_optional(&state.db)
            .await?;

    let Some(model) = model else {
        return Ok(Json(serde_json::json!({
            "error": format!("Unknown model key: {}", model_key)
        })));
    };
    // Cloud models are out of scope for this endpoint (LM Studio only).
    if model.location != "local" {
        return Ok(Json(serde_json::json!({
            "error": format!("Model '{}' is not local — live prompt test targets LM Studio only", model_key)
        })));
    }

    let base_url = &state.config.lmstudio_base_url;

    // Optional image (data URL from the Prompt Builder) — build OpenAI-style
    // multimodal content so vision models receive the actual pixels.
    let image = req.get("image").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
    let user_content = match image {
        Some(data_url) => {
            if !data_url.starts_with("data:image/") {
                return Ok(Json(serde_json::json!({
                    "error": "Image must be a data:image/* URL (base64)"
                })));
            }
            serde_json::json!([
                { "type": "text", "text": prompt },
                { "type": "image_url", "image_url": { "url": data_url } }
            ])
        }
        None => serde_json::json!(prompt),
    };

    let body = serde_json::json!({
        "model": model_key,
        "messages": [{"role": "user", "content": user_content}],
        "max_tokens": 512,
        "temperature": 0.0,
    });

    // Same gate every LM Studio-touching route uses (see lm_guard.rs) — this
    // was the actual self-harm mechanism an audit found: the Prompt Builder
    // called LM Studio directly with zero serialization, so N concurrent
    // clicks/retries meant N concurrent model loads on shared hardware.
    let _permit = crate::lm_guard::acquire().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/v0/chat/completions", base_url))
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Ok(Json(serde_json::json!({
            "error": format!("LM Studio returned HTTP {}: {}", status, body_text)
        })));
    }

    let json: serde_json::Value = resp.json().await?;
    let content = json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(Json(serde_json::json!({
        "model_key": model_key,
        "response": content,
        "prompt_tokens": json.get("usage").and_then(|u| u.get("prompt_tokens")).and_then(|t| t.as_u64()),
        "completion_tokens": json.get("usage").and_then(|u| u.get("completion_tokens")).and_then(|t| t.as_u64()),
    })))
}