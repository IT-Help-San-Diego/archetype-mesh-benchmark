//! Fountain probe — empirical interrogation of a cloud model's REAL rate
//! posture. "Free" that 429s on request 2 is a marketing lie; this measures
//! what a model actually sustains, with per-request evidence.
//!
//! POST /api/fountain  {model_key, provider, requests?: N, interval_ms?: M}
//! GET  /api/fountain            — probe history (verdicts + evidence roll-up)
//! GET  /api/fountain/{id}       — one probe with its full request log
//!
//! Method: N sequential minimal chat completions (max_tokens=16, temp 0,
//! fixed 1-token-answer prompt) spaced interval_ms apart. Every request's
//! HTTP status, latency, usage, and Retry-After header is recorded. The
//! probe streams per-request telemetry over the SSE channel (no-spinner
//! mandate: the UI shows each request land in real time).
//!
//! Verdict vocabulary (computed, single source of truth = verdict_for()):
//!   FOUNTAIN  — every request succeeded; no throttling observed at this rate
//!   TRICKLE   — ≥90% succeeded, at least one 429; usable with patience
//!   THROTTLED — 50–90% succeeded; real work degraded
//!   MIRAGE    — <50% succeeded; advertised access is effectively a lie
//!
//! A verdict is always relative to the probed request count & spacing —
//! both are stored with the evidence, never implied.
use axum::extract::{Path, State};
use axum::response::Json;
use reqwest::Client;
use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::executor::{cloud, provenance};
use crate::state::AppState;

/// Hard cap on probe size: 60 requests at default spacing is ~1 minute of
/// wall time and a few thousand tokens — enough to expose a per-minute
/// limiter without burning real money on a paid model.
const MAX_REQUESTS: i32 = 60;
const DEFAULT_REQUESTS: i32 = 20;
/// Default spacing ~1s: probes the "actually doing work" cadence a real
/// agent produces, not an artificial burst the provider is right to block.
const DEFAULT_INTERVAL_MS: u64 = 1000;
const MIN_INTERVAL_MS: u64 = 250;

#[derive(Deserialize)]
pub struct FountainReq {
    pub model_key: String,
    pub provider: String,
    pub requests: Option<i32>,
    pub interval_ms: Option<u64>,
}

/// Single source of truth for the fountain verdict vocabulary.
fn verdict_for(ok: i64, sent: i64, rate_limited: i64) -> &'static str {
    if sent == 0 {
        return "MIRAGE"; // nothing even got out — key/endpoint dead
    }
    let ratio = ok as f64 / sent as f64;
    if ratio >= 1.0 {
        "FOUNTAIN"
    } else if ratio >= 0.9 && rate_limited > 0 {
        "TRICKLE"
    } else if ratio >= 0.5 {
        "THROTTLED"
    } else {
        "MIRAGE"
    }
}

fn emit(state: &AppState, value: serde_json::Value) {
    if let Ok(json) = serde_json::to_string(&value) {
        let _ = state.events_tx.send(json);
    }
}

pub async fn start_probe(
    State(state): State<AppState>,
    Json(req): Json<FountainReq>,
) -> AppResult<Json<serde_json::Value>> {
    let n = req.requests.unwrap_or(DEFAULT_REQUESTS).clamp(1, MAX_REQUESTS);
    let interval = req.interval_ms.unwrap_or(DEFAULT_INTERVAL_MS).max(MIN_INTERVAL_MS);

    if req.provider == "lmstudio" {
        return Err(AppError::Executor(
            "Fountain probes target cloud rate limits; local models have no rate poster to test".into(),
        ));
    }

    // Key must resolve BEFORE we create a probe row — a missing key is a
    // caller error, not evidence about the provider's rate posture.
    let config_key = match req.provider.as_str() {
        "nous" => &state.config.nous_api_key,
        "openrouter" => &state.config.openrouter_api_key,
        "openai" => &state.config.openai_api_key,
        other => return Err(AppError::Executor(format!("Unknown provider: {}", other))),
    };
    let key = cloud::resolve_api_key(&req.provider, config_key)?;

    let probe_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO fountain_probes (model_key, provider, requests_planned, status)
           VALUES ($1, $2, $3, 'running') RETURNING id"#,
    )
    .bind(&req.model_key)
    .bind(&req.provider)
    .bind(n)
    .fetch_one(&state.db)
    .await?;

    // Fire-and-monitor: the probe runs as a background task and streams
    // telemetry over SSE; the POST returns immediately with the probe id.
    let task_state = state.clone();
    let model_key = req.model_key.clone();
    let provider = req.provider.clone();
    tokio::spawn(async move {
        if let Err(e) =
            run_probe(&task_state, probe_id, &model_key, &provider, &key, n, interval).await
        {
            tracing::error!("fountain probe {} failed: {}", probe_id, e);
            let _ = sqlx::query("UPDATE fountain_probes SET status = 'error' WHERE id = $1")
                .bind(probe_id)
                .execute(&task_state.db)
                .await;
            emit(
                &task_state,
                serde_json::json!({"type":"fountain_error","probe_id":probe_id,"message":e.to_string()}),
            );
        }
    });

    Ok(Json(serde_json::json!({
        "probe_id": probe_id,
        "requests_planned": n,
        "interval_ms": interval,
    })))
}

async fn run_probe(
    state: &AppState,
    probe_id: i32,
    model_key: &str,
    provider: &str,
    api_key: &str,
    n: i32,
    interval_ms: u64,
) -> AppResult<()> {
    let client = Client::new();
    let endpoint = cloud::endpoint_for(provider)?;
    // Fixed minimal stimulus: single deterministic token out. The probe
    // measures the PIPE, not the model's intelligence.
    let body = serde_json::json!({
        "model": model_key,
        "messages": [{"role": "user", "content": "Reply with exactly the word OK and nothing else."}],
        "max_tokens": 16,
        "temperature": 0.0,
    });

    let started = std::time::Instant::now();
    let (mut ok_n, mut limited_n, mut err_n) = (0i64, 0i64, 0i64);
    let mut first_429: Option<i32> = None;
    let (mut tok_in, mut tok_out) = (0i64, 0i64);
    let mut evidence = Vec::with_capacity(n as usize);

    emit(state, serde_json::json!({
        "type":"fountain_started","probe_id":probe_id,"model_key":model_key,
        "provider":provider,"requests_planned":n,"interval_ms":interval_ms}));

    for i in 1..=n {
        let t0 = std::time::Instant::now();
        let resp = client
            .post(endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await;
        let latency = t0.elapsed().as_millis() as i64;

        let (status_code, ok, retry_after, err_snip, p_tok, c_tok) = match resp {
            Ok(r) => {
                let status = r.status().as_u16() as i32;
                let retry_after = r
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                let json: serde_json::Value = r.json().await.unwrap_or(serde_json::Value::Null);
                if status == 200 {
                    let (p, c) = crate::executor::usage_tokens(&json);
                    (status, true, retry_after, None, p, c)
                } else {
                    let snip: String = json.to_string().chars().take(200).collect();
                    (status, false, retry_after, Some(snip), None, None)
                }
            }
            // Transport error: no HTTP status existed. 0 is the sentinel.
            Err(e) => (0, false, None, Some(e.to_string()), None, None),
        };

        if ok {
            ok_n += 1;
        } else if status_code == 429 {
            limited_n += 1;
            if first_429.is_none() {
                first_429 = Some(i);
            }
        } else {
            err_n += 1;
        }
        tok_in += p_tok.unwrap_or(0);
        tok_out += c_tok.unwrap_or(0);

        sqlx::query(
            r#"INSERT INTO fountain_probe_requests
               (probe_id, request_num, http_status, ok, latency_ms, prompt_tokens, completion_tokens, retry_after, error_snippet)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
        )
        .bind(probe_id)
        .bind(i)
        .bind(status_code)
        .bind(ok)
        .bind(latency)
        .bind(p_tok)
        .bind(c_tok)
        .bind(&retry_after)
        .bind(&err_snip)
        .execute(&state.db)
        .await?;

        evidence.push(format!(
            "req={} status={} ok={} latency_ms={} prompt_tokens={:?} completion_tokens={:?} retry_after={:?}",
            i, status_code, ok, latency, p_tok, c_tok, retry_after
        ));

        emit(state, serde_json::json!({
            "type":"fountain_request","probe_id":probe_id,"request_num":i,"of":n,
            "http_status":status_code,"ok":ok,"latency_ms":latency,
            "retry_after":retry_after}));

        if i < n {
            tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
        }
    }

    let sent = ok_n + limited_n + err_n;
    let verdict = verdict_for(ok_n, sent, limited_n);
    let duration = started.elapsed().as_millis() as i64;
    // Seal the full per-request evidence — same provenance discipline as runs.
    let record = format!(
        "fountain_probe id={} model={} provider={} planned={} interval_ms={}\n{}",
        probe_id, model_key, provider, n, interval_ms, evidence.join("\n")
    );
    let sha3 = provenance::sha3_hex(&record);

    sqlx::query(
        r#"UPDATE fountain_probes SET
               requests_sent = $2, requests_ok = $3, requests_rate_limited = $4,
               requests_errored = $5, first_429_at_request = $6,
               total_prompt_tokens = $7, total_completion_tokens = $8,
               duration_ms = $9, verdict = $10, status = 'done', sha3_provenance = $11
           WHERE id = $1"#,
    )
    .bind(probe_id)
    .bind(sent as i32)
    .bind(ok_n as i32)
    .bind(limited_n as i32)
    .bind(err_n as i32)
    .bind(first_429)
    .bind(tok_in)
    .bind(tok_out)
    .bind(duration)
    .bind(verdict)
    .bind(&sha3)
    .execute(&state.db)
    .await?;

    emit(state, serde_json::json!({
        "type":"fountain_verdict","probe_id":probe_id,"verdict":verdict,
        "ok":ok_n,"rate_limited":limited_n,"errored":err_n,"sent":sent,
        "first_429_at_request":first_429,"duration_ms":duration,
        "sha3": sha3}));

    Ok(())
}

pub async fn list_probes(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    let rows: Vec<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT to_jsonb(p) FROM (
               SELECT id, model_key, provider, requests_planned, requests_sent,
                      requests_ok, requests_rate_limited, requests_errored,
                      first_429_at_request, total_prompt_tokens, total_completion_tokens,
                      duration_ms, verdict, status, sha3_provenance, created_at
               FROM fountain_probes ORDER BY created_at DESC LIMIT 200
           ) p"#,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(serde_json::json!({ "probes": rows })))
}

pub async fn probe_detail(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> AppResult<Json<serde_json::Value>> {
    let probe: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(p) FROM fountain_probes p WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let probe = probe.ok_or_else(|| AppError::Executor(format!("No probe with id {}", id)))?;

    let requests: Vec<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT to_jsonb(r) FROM (
               SELECT request_num, http_status, ok, latency_ms, prompt_tokens,
                      completion_tokens, retry_after, error_snippet, created_at
               FROM fountain_probe_requests WHERE probe_id = $1 ORDER BY request_num
           ) r"#,
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(serde_json::json!({ "probe": probe, "requests": requests })))
}

#[cfg(test)]
mod tests {
    use super::verdict_for;

    #[test]
    fn all_ok_is_fountain() {
        assert_eq!(verdict_for(20, 20, 0), "FOUNTAIN");
    }

    #[test]
    fn one_429_in_twenty_is_trickle() {
        assert_eq!(verdict_for(19, 20, 1), "TRICKLE");
    }

    #[test]
    fn ninety_percent_without_429_is_throttled_not_trickle() {
        // Failures that aren't rate limits (500s, timeouts) are instability,
        // not a polite limiter — TRICKLE requires observed 429 behavior.
        assert_eq!(verdict_for(18, 20, 0), "THROTTLED");
    }

    #[test]
    fn two_thirds_is_throttled() {
        assert_eq!(verdict_for(14, 20, 6), "THROTTLED");
    }

    #[test]
    fn under_half_is_mirage() {
        assert_eq!(verdict_for(9, 20, 11), "MIRAGE");
    }

    #[test]
    fn nothing_sent_is_mirage() {
        assert_eq!(verdict_for(0, 0, 0), "MIRAGE");
    }
}
