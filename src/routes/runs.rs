//! POST /api/runs — start a benchmark run. GET /api/runs — run history.
//!
//! POST body: {"model_key": "...", "axes": ["vision","tools",...]}
//! One test_runs row is created per requested axis; each executes as its own
//! background task. Local-model runs are serialized by a global mutex so two
//! clean-room runs can never fight over LM Studio RAM residency.
use axum::extract::State;
use axum::response::Json;
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Global clean-room lock: only one LOCAL run may drive LM Studio at a time.
/// Cloud runs don't take it (no shared hardware to contaminate).
static LOCAL_RUN_LOCK: Mutex<()> = Mutex::const_new(());

const VALID_AXES: [&str; 4] = ["vision", "tools", "reasoning", "security"];

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
    for axis in &req.axes {
        if !VALID_AXES.contains(&axis.as_str()) {
            return Err(AppError::Executor(format!("Invalid axis: {}", axis)));
        }
    }

    let model = sqlx::query_as::<_, ModelRow>(
        "SELECT id, key, provider, location FROM models WHERE key = $1 AND active = true",
    )
    .bind(&req.model_key)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Executor(format!("Unknown model key: {}", req.model_key)))?;

    let mut run_ids = Vec::new();
    for axis in &req.axes {
        // Each run must reference a concrete test; use the first active test on
        // the axis as the anchor (the executor runs ALL active tests on the axis).
        let anchor_test: Option<(i32,)> = sqlx::query_as(
            "SELECT id FROM tests WHERE active = true AND axis = $1 ORDER BY id LIMIT 1",
        )
        .bind(axis)
        .fetch_optional(&state.db)
        .await?;
        let Some((test_id,)) = anchor_test else {
            return Err(AppError::Executor(format!("No active tests for axis '{}'", axis)));
        };

        let (run_id,): (i32,) = sqlx::query_as(
            r#"INSERT INTO test_runs (model_id, test_id, axis, status)
               VALUES ($1, $2, $3, 'queued') RETURNING id"#,
        )
        .bind(model.id)
        .bind(test_id)
        .bind(axis)
        .fetch_one(&state.db)
        .await?;
        run_ids.push(run_id);

        let db = state.db.clone();
        let config = state.config.clone();
        let tx = state.events_tx.clone();
        let model_id = model.id;
        let model_key = model.key.clone();
        let location = model.location.clone();
        let provider = model.provider.clone();
        let axis = axis.clone();

        tokio::spawn(async move {
            // Serialize local runs — clean-room integrity.
            let _guard = if location == "local" {
                Some(LOCAL_RUN_LOCK.lock().await)
            } else {
                None
            };
            crate::executor::execute_run(
                db, config, tx, run_id, model_id, model_key, location, provider, axis,
            )
            .await;
        });
    }

    Ok(Json(serde_json::json!({
        "run_id": run_ids[0],
        "run_ids": run_ids,
        "model_key": req.model_key,
        "axes": req.axes,
    })))
}

pub async fn list_runs(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    let rows: Vec<(i32, String, String, String, i32, i32, Option<String>, Option<chrono::NaiveDateTime>)> =
        sqlx::query_as(
            r#"SELECT r.id, m.key, r.axis, r.status, r.pass_count, r.total_count,
                      r.sha3_provenance, r.created_at
               FROM test_runs r JOIN models m ON m.id = r.model_id
               ORDER BY r.created_at DESC LIMIT 100"#,
        )
        .fetch_all(&state.db)
        .await?;

    let runs: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, key, axis, status, pass, total, sha3, created)| {
            serde_json::json!({
                "id": id, "model_key": key, "axis": axis, "status": status,
                "pass_count": pass, "total_count": total,
                "sha3_provenance": sha3,
                "created_at": created.map(|c| c.to_string()),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "runs": runs })))
}
