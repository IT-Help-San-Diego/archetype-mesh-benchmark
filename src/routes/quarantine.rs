//! Quarantine review API — inspect, annotate, and release junk test runs
//! without letting them contaminate the leaderboard.
//!
//! Routes:
//!   GET /api/quarantine
//!     List quarantined runs, optionally filtered by model/axis/reason.
//!
//!   POST /api/quarantine/{id}/release
//!     Remove the quarantine flag from a run so it can re-enter scoring.
//!
//!   POST /api/quarantine/{id}/notes
//!     Append learning notes to a quarantined run for post-mortem analysis.
//!
//! Principle: quarantined runs are preserved in `trial_results` for learning,
//! but excluded from leaderboard/router/dossier/insights unless explicitly
//! released. This matches the user's requirement: "modular data so that we
//! can track everything and gain intelligence moving forward, even from the
//! user's failed test, in helping them get configured correctly and cleanly."

use axum::extract::{Json, Path, Query, State};
use axum::response::Json as ResponseJson;
use serde::{Deserialize, Serialize};
use sqlx::QueryBuilder;

use crate::error::AppResult;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct QuarantineListQuery {
    pub model_id: Option<i32>,
    pub axis: Option<String>,
    pub reason: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(sqlx::FromRow, Serialize)]
pub struct QuarantineRunRow {
    pub id: i32,
    pub model_id: i32,
    pub model_key: String,
    pub display_name: String,
    pub axis: String,
    pub status: String,
    pub quarantined: bool,
    pub quarantine_reason: Option<String>,
    pub quarantine_notes: Option<String>,
    pub pass_count: Option<i32>,
    pub total_count: Option<i32>,
    pub sha3_provenance: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReleaseRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NotesRequest {
    pub notes: String,
}

/// GET /api/quarantine
/// List quarantined runs with optional filters.
pub async fn list_quarantined(
    State(state): State<AppState>,
    Query(params): Query<QuarantineListQuery>,
) -> AppResult<ResponseJson<serde_json::Value>> {
    let limit = params.limit.unwrap_or(100).clamp(1, 500);
    let offset = params.offset.unwrap_or(0).max(0);

    let mut builder = QueryBuilder::new(
        r#"SELECT tr.id, tr.model_id, m.key AS model_key, m.display_name, tr.axis,
                  tr.status, tr.quarantined, tr.quarantine_reason, tr.quarantine_notes,
                  tr.pass_count, tr.total_count, tr.sha3_provenance,
                  tr.created_at::text, tr.started_at::text, tr.finished_at::text
           FROM test_runs tr
           JOIN models m ON m.id = tr.model_id
           WHERE tr.quarantined = TRUE"#,
    );

    if let Some(model_id) = params.model_id {
        builder.push(" AND tr.model_id = ").push_bind(model_id);
    }
    if let Some(ref axis) = params.axis {
        builder.push(" AND tr.axis = ").push_bind(axis);
    }
    if let Some(ref reason) = params.reason {
        builder.push(" AND tr.quarantine_reason = ").push_bind(reason);
    }

    builder.push(" ORDER BY tr.created_at DESC");
    builder.push(" LIMIT ").push_bind(limit);
    builder.push(" OFFSET ").push_bind(offset);

    let rows = builder
        .build_query_as::<QuarantineRunRow>()
        .fetch_all(&state.db)
        .await?;

    Ok(ResponseJson(serde_json::json!({
        "quarantined_runs": rows,
        "filters": {
            "model_id": params.model_id,
            "axis": params.axis,
            "reason": params.reason,
        },
        "pagination": { "limit": limit, "offset": offset }
    })))
}

/// POST /api/quarantine/{id}/release
/// Remove the quarantine flag from a run so it can re-enter leaderboard scoring.
pub async fn release_quarantined(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<ReleaseRequest>,
) -> AppResult<ResponseJson<serde_json::Value>> {
    let existing: Option<(bool, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT quarantined, quarantine_reason, quarantine_notes FROM test_runs WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;

    let (was_quarantined, _old_reason, old_notes) = existing.unwrap_or((false, None, None));

    if !was_quarantined {
        return Ok(ResponseJson(serde_json::json!({
            "released": false,
            "run_id": id,
            "message": "Run was not quarantined — no action taken."
        })));
    }

    let appended_notes = match (old_notes, req.reason) {
        (Some(notes), Some(reason)) => format!("{}\n[RELEASED] {}", notes, reason),
        (Some(notes), None) => format!("{}\n[RELEASED] Quarantine removed by operator.", notes),
        (None, Some(reason)) => format!("[RELEASED] {}", reason),
        (None, None) => "[RELEASED] Quarantine removed by operator.".to_string(),
    };

    sqlx::query(
        r#"UPDATE test_runs
           SET quarantined = FALSE,
               quarantine_reason = NULL,
               quarantine_notes = $1,
               updated_at = NOW()
           WHERE id = $2"#,
    )
    .bind(&appended_notes)
    .bind(id)
    .execute(&state.db)
    .await?;

    Ok(ResponseJson(serde_json::json!({
        "released": true,
        "run_id": id,
        "message": "Quarantine removed — run is now eligible for leaderboard scoring.",
        "appended_notes": appended_notes,
    })))
}

/// POST /api/quarantine/{id}/notes
/// Append learning notes to a quarantined run for post-mortem analysis.
pub async fn append_notes(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<NotesRequest>,
) -> AppResult<ResponseJson<serde_json::Value>> {
    let existing: Option<(bool, Option<String>)> =
        sqlx::query_as("SELECT quarantined, quarantine_notes FROM test_runs WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;

    let (is_quarantined, old_notes) = existing.unwrap_or((false, None));

    let new_notes = match old_notes {
        Some(notes) => format!("{}\n[NOTE] {}", notes, req.notes),
        None => format!("[NOTE] {}", req.notes),
    };

    sqlx::query(
        r#"UPDATE test_runs
           SET quarantine_notes = $1, updated_at = NOW()
           WHERE id = $2"#,
    )
    .bind(&new_notes)
    .bind(id)
    .execute(&state.db)
    .await?;

    Ok(ResponseJson(serde_json::json!({
        "run_id": id,
        "is_quarantined": is_quarantined,
        "appended_notes": new_notes,
    })))
}
