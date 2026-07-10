//! /api/tests — test-definition CRUD for the Test Builder.
//!
//! GET  /api/tests        — list all active tests (blind mode: expected_result
//!                          and prompt_text are included only when ?full=true,
//!                          so a casual list view can't leak ground truth to
//!                          anyone shoulder-surfing a screen share).
//! POST /api/tests        — create a test definition.
//! PUT  /api/tests/:id    — update fields or deactivate (soft delete).
//!
//! Ground truth (expected_result) lives ONLY in this table; the executor
//! assembles prompts server-side and never sends the expected answer to the
//! model. That is the anti-cheating core of the whole benchmark.
use axum::extract::{Path, Query, State};
use axum::response::Json;
use serde::Deserialize;

use crate::error::AppResult;
use crate::state::AppState;

#[derive(sqlx::FromRow, serde::Serialize)]
pub struct TestRow {
    pub id: i32,
    pub name: String,
    pub axis: String,
    pub prompt_text: Option<String>,
    pub attachment_path: Option<String>,
    pub attachment_sha3: Option<String>,
    pub expected_result: Option<String>,
    pub scoring_method: String,
    pub trials_per_run: Option<i32>,
    pub active: Option<bool>,
    pub created_at: Option<chrono::NaiveDateTime>,
    pub updated_at: Option<chrono::NaiveDateTime>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    /// When true, include prompt_text and expected_result (builder/edit view).
    /// Default false: list view shows metadata only — blind by default.
    #[serde(default)]
    pub full: bool,
}

pub async fn list_tests(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, TestRow>(
        r#"SELECT id, name, axis, prompt_text, attachment_path, attachment_sha3,
                  expected_result, scoring_method, trials_per_run, active,
                  created_at, updated_at
           FROM tests WHERE active = true
           ORDER BY axis, id"#,
    )
    .fetch_all(&state.db)
    .await?;

    let tests: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|t| {
            let mut v = serde_json::json!({
                "id": t.id,
                "name": t.name,
                "axis": t.axis,
                "scoring_method": t.scoring_method,
                "trials_per_run": t.trials_per_run.unwrap_or(3),
                "has_attachment": t.attachment_path.is_some(),
                "created_at": t.created_at.map(|d| d.to_string()),
            });
            if q.full {
                v["prompt_text"] = serde_json::json!(t.prompt_text);
                v["expected_result"] = serde_json::json!(t.expected_result);
                v["attachment_path"] = serde_json::json!(t.attachment_path);
                v["attachment_sha3"] = serde_json::json!(t.attachment_sha3);
            }
            v
        })
        .collect();

    Ok(Json(serde_json::json!({ "tests": tests, "count": tests.len() })))
}

const VALID_AXES: [&str; 6] = ["vision", "tools", "reasoning", "security", "literary", "auxiliary"];
const VALID_SCORING: [&str; 6] = ["exact", "substring", "spatial", "nested_tool", "security", "regex"];

fn validate(axis: &str, scoring: &str) -> Option<String> {
    if !VALID_AXES.contains(&axis) {
        return Some(format!(
            "Invalid axis '{}' — must be one of: {}",
            axis,
            VALID_AXES.join(", ")
        ));
    }
    if !VALID_SCORING.contains(&scoring) {
        return Some(format!(
            "Invalid scoring_method '{}' — must be one of: {}",
            scoring,
            VALID_SCORING.join(", ")
        ));
    }
    None
}

pub async fn create_test(
    State(state): State<AppState>,
    axum::extract::Json(req): axum::extract::Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let Some(name) = req.get("name").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) else {
        return Ok(Json(serde_json::json!({ "error": "Missing name" })));
    };
    let Some(axis) = req.get("axis").and_then(|v| v.as_str()) else {
        return Ok(Json(serde_json::json!({ "error": "Missing axis" })));
    };
    let Some(prompt_text) = req.get("prompt_text").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) else {
        return Ok(Json(serde_json::json!({ "error": "Missing prompt_text" })));
    };
    let Some(expected) = req.get("expected_result").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) else {
        return Ok(Json(serde_json::json!({ "error": "Missing expected_result — a test without ground truth can't be scored objectively" })));
    };
    let scoring = req.get("scoring_method").and_then(|v| v.as_str()).unwrap_or("exact");
    if let Some(msg) = validate(axis, scoring) {
        return Ok(Json(serde_json::json!({ "error": msg })));
    }
    // Guard against ground truth leaking into the prompt itself (answer leakage).
    if prompt_text.to_lowercase().contains(&expected.to_lowercase()) && expected.len() > 3 {
        return Ok(Json(serde_json::json!({
            "error": "Answer leakage: expected_result appears verbatim inside prompt_text. A valid test never contains its own answer."
        })));
    }
    let trials = req.get("trials_per_run").and_then(|v| v.as_i64()).unwrap_or(3).clamp(1, 10) as i32;

    let (id,): (i32,) = sqlx::query_as(
        r#"INSERT INTO tests (name, axis, prompt_text, expected_result, scoring_method, trials_per_run, active)
           VALUES ($1, $2, $3, $4, $5, $6, true) RETURNING id"#,
    )
    .bind(name)
    .bind(axis)
    .bind(prompt_text)
    .bind(expected)
    .bind(scoring)
    .bind(trials)
    .fetch_one(&state.db)
    .await?;

    tracing::info!("Test created: id={} name={} axis={}", id, name, axis);
    Ok(Json(serde_json::json!({ "id": id, "name": name, "axis": axis, "created": true })))
}

pub async fn update_test(
    State(state): State<AppState>,
    Path(id): Path<i32>,
    axum::extract::Json(req): axum::extract::Json<serde_json::Value>,
) -> AppResult<Json<serde_json::Value>> {
    let exists: Option<(i32,)> = sqlx::query_as("SELECT id FROM tests WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    if exists.is_none() {
        return Ok(Json(serde_json::json!({ "error": format!("No test with id {}", id) })));
    }

    // Deactivation is its own fast path (soft delete — runs keep their FK).
    if req.get("active").and_then(|v| v.as_bool()) == Some(false) {
        sqlx::query("UPDATE tests SET active = false, updated_at = CURRENT_TIMESTAMP WHERE id = $1")
            .bind(id)
            .execute(&state.db)
            .await?;
        tracing::info!("Test {} deactivated", id);
        return Ok(Json(serde_json::json!({ "id": id, "deactivated": true })));
    }

    if let (Some(axis), Some(scoring)) = (
        req.get("axis").and_then(|v| v.as_str()),
        req.get("scoring_method").and_then(|v| v.as_str()),
    ) {
        if let Some(msg) = validate(axis, scoring) {
            return Ok(Json(serde_json::json!({ "error": msg })));
        }
    }

    // Apply only the fields provided (partial update). Static SQL per column —
    // no dynamic query strings, no injection surface.
    let mut updated = Vec::new();
    if let Some(v) = req.get("name").and_then(|v| v.as_str()) {
        sqlx::query("UPDATE tests SET name = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
            .bind(v).bind(id).execute(&state.db).await?;
        updated.push("name");
    }
    if let Some(v) = req.get("axis").and_then(|v| v.as_str()) {
        if !VALID_AXES.contains(&v) {
            return Ok(Json(serde_json::json!({ "error": format!("Invalid axis '{}'", v) })));
        }
        sqlx::query("UPDATE tests SET axis = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
            .bind(v).bind(id).execute(&state.db).await?;
        updated.push("axis");
    }
    if let Some(v) = req.get("prompt_text").and_then(|v| v.as_str()) {
        sqlx::query("UPDATE tests SET prompt_text = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
            .bind(v).bind(id).execute(&state.db).await?;
        updated.push("prompt_text");
    }
    if let Some(v) = req.get("expected_result").and_then(|v| v.as_str()) {
        sqlx::query("UPDATE tests SET expected_result = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
            .bind(v).bind(id).execute(&state.db).await?;
        updated.push("expected_result");
    }
    if let Some(v) = req.get("scoring_method").and_then(|v| v.as_str()) {
        if !VALID_SCORING.contains(&v) {
            return Ok(Json(serde_json::json!({ "error": format!("Invalid scoring_method '{}'", v) })));
        }
        sqlx::query("UPDATE tests SET scoring_method = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
            .bind(v).bind(id).execute(&state.db).await?;
        updated.push("scoring_method");
    }
    if let Some(trials) = req.get("trials_per_run").and_then(|v| v.as_i64()) {
        sqlx::query("UPDATE tests SET trials_per_run = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
            .bind(trials.clamp(1, 10) as i32)
            .bind(id)
            .execute(&state.db)
            .await?;
        updated.push("trials_per_run");
    }

    tracing::info!("Test {} updated: {:?}", id, updated);
    Ok(Json(serde_json::json!({ "id": id, "updated_fields": updated })))
}

/// POST /api/tests/:id/duplicate — copy a test definition so users can
/// create anti-cheat variants by swapping a few words in the prompt.
/// The duplicate is active and immediately usable. The name gets
/// " (copy)" appended so it's distinguishable in the list.
pub async fn duplicate_test(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> AppResult<Json<serde_json::Value>> {
    let source: TestRow = sqlx::query_as(
        r#"SELECT id, name, axis, prompt_text, attachment_path, attachment_sha3,
                  expected_result, scoring_method, trials_per_run, active,
                  created_at, updated_at
           FROM tests WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| crate::error::AppError::Executor(format!("Test {} not found", id)))?;

    if !source.active.unwrap_or(true) {
        return Ok(Json(serde_json::json!({
            "error": "Cannot duplicate a deactivated test — activate it first"
        })));
    }

    let new_name = format!("{} (copy)", source.name);
    let new_prompt = source.prompt_text.as_deref().unwrap_or("");

    // Anti-cheat leakage guard: same as create_test — reject if the
    // expected answer appears verbatim in the prompt text.
    if let Some(expected) = &source.expected_result {
        if new_prompt.to_lowercase().contains(&expected.to_lowercase()) {
            return Ok(Json(serde_json::json!({
                "error": "expected_result appears verbatim inside prompt_text — not duplicating a leaky test"
            })));
        }
    }

    let _result = sqlx::query(
        r#"INSERT INTO tests (name, axis, prompt_text, attachment_path, attachment_sha3,
                              expected_result, scoring_method, trials_per_run, active)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, true)
           RETURNING id"#,
    )
    .bind(&new_name)
    .bind(&source.axis)
    .bind(&source.prompt_text)
    .bind(&source.attachment_path)
    .bind(&source.attachment_sha3)
    .bind(&source.expected_result)
    .bind(&source.scoring_method)
    .bind(source.trials_per_run)
    .fetch_one(&state.db)
    .await?;

    let new_id: i32 = sqlx::query_scalar("SELECT id FROM tests ORDER BY id DESC LIMIT 1")
        .fetch_one(&state.db)
        .await?;

    tracing::info!("Duplicated test {} → new test {}", id, new_id);
    Ok(Json(serde_json::json!({
        "id": new_id,
        "name": new_name,
        "source_id": id,
        "message": "Duplicate created — edit the prompt to create an anti-cheat variant"
    })))
}
