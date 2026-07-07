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
