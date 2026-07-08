//! POST /api/runs — start a benchmark run. GET /api/runs — run history.
//!
//! POST body: {"model_key": "...", "axes": ["vision","tools",...]}
//! One test_runs row is created per requested axis; each executes as its own
//! background task. Local-model runs are serialized by a global mutex so two
//! clean-room runs can never fight over LM Studio RAM residency.
use axum::extract::State;
use axum::response::Json;
use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

// "auxiliary" is an experimental axis (added 2026-07-07) tracking whether
// non-frontier / local models are reliable enough for Hermes' auxiliary
// tasks (approval classification, MCP sampling relay) — see migration 009.
// Deliberately kept separate from the core 4-axis capability grid.
const VALID_AXES: [&str; 5] = ["vision", "tools", "reasoning", "security", "auxiliary"];

#[derive(Debug, Deserialize)]
pub struct StartRunRequest {
    pub model_key: String,
    pub axes: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct ModelRow {
    id: i32,
    key: String,
    provider: String,
    location: String,
}

pub async fn start_runs(
    State(state): State<AppState>,
    Json(req): Json<StartRunRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if req.axes.is_empty() {
        return Err(AppError::Executor("axes must be non-empty".into()));
    }
    // Validate, then dedup preserving order: ["reasoning","reasoning"] must
    // cost ONE battery, not two. Duplicate axes double real GPU time on a
    // machine that is someone's daily driver — same protection class as the
    // run budget.
    let mut axes: Vec<&str> = Vec::new();
    for axis in &req.axes {
        if !VALID_AXES.contains(&axis.as_str()) {
            return Err(AppError::Executor(format!("Invalid axis: {}", axis)));
        }
        if !axes.contains(&axis.as_str()) {
            axes.push(axis.as_str());
        }
    }

    let model = sqlx::query_as::<_, ModelRow>(
        "SELECT id, key, provider, location FROM models WHERE key = $1 AND active = true",
    )
    .bind(&req.model_key)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Executor(format!("Unknown model key: {}", req.model_key)))?;

    // Refuse to stack a duplicate battery behind an identical one already
    // queued/running: the clean-room lock serializes local runs, so a repeat
    // click (or an impatient script) would silently commit HOURS of extra
    // grind. Finished runs don't block — re-measurement is always allowed.
    // 'aborted' is ALSO terminal here — an operator-stopped run must not
    // permanently block re-running that (model, axis) pair. Found live
    // 2026-07-08: this predicate was written before the 'aborted' status
    // existed and wasn't updated when it landed, so an aborted run looked
    // permanently "in flight" to this check.
    for axis in &axes {
        let (in_flight,): (i64,) = sqlx::query_as(
            r#"SELECT COUNT(*) FROM test_runs
               WHERE model_id = $1 AND axis = $2 AND status NOT IN ('done', 'error', 'aborted')"#,
        )
        .bind(model.id)
        .bind(axis)
        .fetch_one(&state.db)
        .await?;
        if in_flight > 0 {
            return Err(AppError::Executor(format!(
                "A '{}' run for {} is already queued or running — wait for it to finish (or check /api/runs). Re-running after completion is always allowed.",
                axis, model.key
            )));
        }
    }

    let mut run_ids = Vec::new();
    // ONE run per (model, axis). The executor runs every active test on the
    // axis inside that single run — pass_count/total_count aggregate the whole
    // battery. (Previously this inserted one run per test while the executor
    // still ran the full battery per run: N² executions. Fixed 2026-07-07.)
    for axis in &axes {
        let (test_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM tests WHERE active = true AND axis = $1",
        )
        .bind(axis)
        .fetch_one(&state.db)
        .await?;

        if test_count == 0 {
            return Err(AppError::Executor(format!("No active tests for axis '{}'", axis)));
        }

        let (run_id,): (i32,) = sqlx::query_as(
            r#"INSERT INTO test_runs (model_id, test_id, axis, status)
               VALUES ($1, NULL, $2, 'queued') RETURNING id"#,
        )
        .bind(model.id)
        .bind(axis)
        .fetch_one(&state.db)
        .await?;
        run_ids.push(run_id);

        let db = state.db.clone();
        let config = state.config.clone();
        let tx = state.events_tx.clone();
        let cancellations = state.cancellations.clone();
        let model_id = model.id;
        let model_key = model.key.clone();
        let location = model.location.clone();
        let provider = model.provider.clone();
        let axis = axis.to_string();

        tokio::spawn(async move {
            // Serialize LOCAL LM Studio access via the shared process-wide
            // gate (lm_guard::acquire) — NOT a route-local mutex. Prior to
            // 2026-07-08 this used a private LOCAL_RUN_LOCK that only
            // benchmark runs took; POST /api/prompt-check and the Prompt
            // Builder called LM Studio directly with zero serialization —
            // the actual self-harm gap an audit found (unbounded concurrent
            // local model loads on a daily-driver machine). Every LM
            // Studio-touching route now goes through the same gate.
            let _permit = if location == "local" {
                Some(crate::lm_guard::acquire().await)
            } else {
                None
            };
            crate::executor::execute_run(
                db, config, tx, cancellations, run_id, model_id, model_key, location, provider,
                axis,
            )
            .await;
        });
    }

    Ok(Json(serde_json::json!({
        "run_id": run_ids[0],
        "run_ids": run_ids,
        "model_key": req.model_key,
        "axes": axes,
    })))
}

/// Row shape for the run-history listing (typed, not a bare tuple —
/// clippy::type_complexity and future column changes both point the same way).
#[derive(sqlx::FromRow)]
struct RunListRow {
    id: i32,
    key: String,
    axis: String,
    status: String,
    pass_count: i32,
    total_count: i32,
    sha3_provenance: Option<String>,
    created_at: Option<chrono::NaiveDateTime>,
}

pub async fn list_runs(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    let rows: Vec<RunListRow> = sqlx::query_as(
        r#"SELECT r.id, m.key, r.axis, r.status, r.pass_count, r.total_count,
                  r.sha3_provenance, r.created_at
           FROM test_runs r JOIN models m ON m.id = r.model_id
           ORDER BY r.created_at DESC LIMIT 100"#,
    )
    .fetch_all(&state.db)
    .await?;

    let runs: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id, "model_key": r.key, "axis": r.axis, "status": r.status,
                "pass_count": r.pass_count, "total_count": r.total_count,
                "sha3_provenance": r.sha3_provenance,
                "created_at": r.created_at.map(|c| c.to_string()),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "runs": runs })))
}

/// POST /api/runs/:id/abort — the abort button.
///
/// Signals the run's CancellationToken (registered by execute_run_inner at
/// start). The executor's select! around every LM Studio/cloud call reacts
/// within one HTTP round-trip's time, drops the outbound connection, and
/// LM Studio itself stops the GPU work — verified live 2026-07-08 (killing
/// a streaming client caused the llmworker process's CPU to drop from
/// 11.2% to 0.1% within 1 second). This is a real abort, not a UI-only one.
///
/// Idempotent: aborting a run that's already done/error/aborted, or that
/// never existed, returns {aborted: false} rather than an error — asking to
/// stop something that isn't running anymore is not a mistake to punish.
pub async fn abort_run(
    State(state): State<AppState>,
    axum::extract::Path(run_id): axum::extract::Path<i32>,
) -> AppResult<Json<serde_json::Value>> {
    let signaled = state.cancellations.cancel(run_id).await;
    Ok(Json(serde_json::json!({ "run_id": run_id, "aborted": signaled })))
}
