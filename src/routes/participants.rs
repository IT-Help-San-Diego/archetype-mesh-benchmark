//! Human-participant CRUD + "take the battery" submission API.
//!
//! Migration 043 built the schema (participants table, test_runs.participant_id,
//! owl_signal_carrier view). This module builds the WRITE half — the read half
//! is signal_carrier.rs (same view, both subjects in one shape).
//!
//! Design: a human calibration session is NOT an executor run. The executor
//! auto-generates trials by calling an LLM; a human answers one question at a
//! time in the browser. So the flow is:
//!
//!   1. POST /api/participants            → create or reuse a participant
//!   2. POST /api/participants/:id/start   → create a test_runs row (participant_id
//!      set, model_id NULL, status='running') seeded with the I+N tests for a
//!      chosen axis/family. Returns the run_id + the list of test prompts.
//!   3. POST /api/participants/:id/answer  → submit one verdict; scored by the
//!      same exact-match grader the executor uses; writes a trial_results row.
//!   4. POST /api/participants/:id/finish   → seal the run (status='done',
//!      recompute pass_count/total_count, set sha3_provenance).
//!
//! No LLM is ever called. No model judges the human. The grader is the same
//! deterministic `score_response` function — exact string match against
//! expected_result, identical to what models face.

use axum::extract::{Path, State};
use axum::response::Json;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

// ── Participant CRUD ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateParticipant {
    pub display_name: String,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Participant {
    pub id: i32,
    pub kind: String,
    pub display_name: String,
    pub notes: Option<String>,
    pub created_at: chrono::NaiveDateTime,
}

pub async fn create_participant(
    State(state): State<AppState>,
    Json(req): Json<CreateParticipant>,
) -> AppResult<Json<Participant>> {
    let row: Participant = sqlx::query_as(
        r#"INSERT INTO participants (display_name, notes)
           VALUES ($1, $2)
           RETURNING id, kind, display_name, notes, created_at"#,
    )
    .bind(&req.display_name)
    .bind(&req.notes)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(row))
}

pub async fn list_participants(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<Participant>>> {
    let rows: Vec<Participant> = sqlx::query_as(
        r#"SELECT id, kind, display_name, notes, created_at
           FROM participants ORDER BY created_at DESC"#,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

// ── Start a human calibration session ─────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StartSession {
    /// Which axis to calibrate on (reasoning, literary, ...).
    pub axis: Option<String>,
    /// Optional: restrict to one owl family root (e.g. test id=26 for LOGIC-01).
    /// Omit for all I+N tests on the axis.
    pub family_root_id: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct SessionStart {
    pub run_id: i32,
    pub participant_id: i32,
    pub tests: Vec<SessionTest>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SessionTest {
    pub id: i32,
    pub name: String,
    pub prompt_text: String,
    pub axis: String,
    pub owl_type: String,
    pub owl_root_id: Option<i32>,
    pub formal_spec: Option<String>,
}

pub async fn start_session(
    State(state): State<AppState>,
    Path(participant_id): Path<i32>,
    Json(req): Json<StartSession>,
) -> AppResult<Json<SessionStart>> {
    // Validate participant exists
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM participants WHERE id = $1)",
    )
    .bind(participant_id)
    .fetch_one(&state.db)
    .await?;
    if !exists {
        return Err(AppError::Executor(format!(
            "participant {participant_id} not found"
        )));
    }

    // Fetch the I+N tests for the chosen axis/family — same scope as the
    // owl_signal_carrier view (migration 043), so what the human answers
    // is what the view scores.
    let tests: Vec<SessionTest> = sqlx::query_as(
        r#"SELECT id, name, prompt_text, axis, owl_type::text AS owl_type,
                  owl_root_id, formal_spec
           FROM tests
           WHERE active = true
             AND owl_type IN ('I', 'N')
             AND ($1::text IS NULL OR axis = $1)
             AND ($2::int IS NULL
                  OR id = $2
                  OR owl_root_id = $2)
           ORDER BY COALESCE(owl_root_id, id), id"#,
    )
    .bind(&req.axis)
    .bind(req.family_root_id)
    .fetch_all(&state.db)
    .await?;

    if tests.is_empty() {
        return Err(AppError::Executor(
            "no active I+N tests match the given axis/family".into(),
        ));
    }

    // Create the test_runs row. We use axis='reasoning' as a default if the
    // request didn't specify — but store the actual per-test axes on each
    // trial_results row, so the signal_carrier view (which groups by family
    // and axis) works correctly regardless.
    let run_axis = req.axis.unwrap_or("reasoning".into());
    let test_ids: Vec<i32> = tests.iter().map(|t| t.id).collect();

    let run_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO test_runs
             (participant_id, axis, status, started_at, test_ids)
           VALUES ($1, $2, 'running', NOW(), $3)
           RETURNING id"#,
    )
    .bind(participant_id)
    .bind(&run_axis)
    .bind(serde_json::to_value(&test_ids).unwrap_or(serde_json::Value::Null))
    .fetch_one(&state.db)
    .await?;

    Ok(Json(SessionStart {
        run_id,
        participant_id,
        tests,
    }))
}

// ── Submit a single answer ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SubmitAnswer {
    pub run_id: i32,
    pub test_id: i32,
    pub answer: String,
}

#[derive(Debug, Serialize)]
pub struct AnswerResult {
    pub trial_result_id: i32,
    pub passed: bool,
    pub expected: String,
    /// The test's name — so the UI can show "LOGIC-01N: correct" without
    /// a second round-trip.
    pub test_name: String,
}

pub async fn submit_answer(
    State(state): State<AppState>,
    Path(_participant_id): Path<i32>,
    Json(req): Json<SubmitAnswer>,
) -> AppResult<Json<AnswerResult>> {
    // Fetch the test to get expected_result + scoring_method + name
    #[derive(sqlx::FromRow)]
    struct TestRow {
        name: String,
        expected_result: Option<String>,
        scoring_method: String,
    }
    let test: TestRow = sqlx::query_as(
        r#"SELECT name, expected_result, scoring_method
           FROM tests WHERE id = $1 AND active = true"#,
    )
    .bind(req.test_id)
    .fetch_one(&state.db)
    .await?;

    let expected = test.expected_result.ok_or_else(|| {
        AppError::Executor(format!("test {} has no expected_result", req.test_id))
    })?;

    // Same grader the executor uses: exact match (case-insensitive, trimmed).
    let passed = match test.scoring_method.as_str() {
        "exact" => req.answer.trim().eq_ignore_ascii_case(&expected),
        // Future: fuzzy/contains scoring can be added here.
        _ => req.answer.trim().eq_ignore_ascii_case(&expected),
    };

    // Write the trial_result row. trial_num is sequential within the run.
    let next_trial: i32 = sqlx::query_scalar(
        r#"SELECT COALESCE(MAX(trial_num), 0) + 1
           FROM trial_results WHERE run_id = $1"#,
    )
    .bind(req.run_id)
    .fetch_one(&state.db)
    .await?;

    let trial_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO trial_results
             (run_id, trial_num, test_id, raw_response, passed, latency_ms,
              is_infra_error)
           VALUES ($1, $2, $3, $4, $5, 0, false)
           RETURNING id"#,
    )
    .bind(req.run_id)
    .bind(next_trial)
    .bind(req.test_id)
    .bind(&req.answer)
    .bind(passed)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(AnswerResult {
        trial_result_id: trial_id,
        passed,
        expected,
        test_name: test.name,
    }))
}

// ── Seal the session ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct FinishSession {
    pub run_id: i32,
}

#[derive(Debug, Serialize)]
pub struct SessionResult {
    pub run_id: i32,
    pub status: String,
    pub pass_count: i32,
    pub total_count: i32,
    pub sha3_provenance: Option<String>,
}

pub async fn finish_session(
    State(state): State<AppState>,
    Path(_participant_id): Path<i32>,
    Json(req): Json<FinishSession>,
) -> AppResult<Json<SessionResult>> {
    // Recompute pass_count / total_count from the trial_results.
    #[derive(sqlx::FromRow)]
    struct Counts {
        pass_count: i32,
        total_count: i32,
    }
    let counts: Counts = sqlx::query_as(
        r#"SELECT
              COUNT(*) FILTER (WHERE passed)::int AS pass_count,
              COUNT(*)::int AS total_count
           FROM trial_results
           WHERE run_id = $1 AND is_infra_error = false"#,
    )
    .bind(req.run_id)
    .fetch_one(&state.db)
    .await?;
    let pass_count = counts.pass_count;
    let total_count = counts.total_count;

    // Seal: set status='done', finished_at=NOW, and a SHA-3 provenance hash
    // over the trial verdicts — computed in Rust (same discipline as model
    // runs via provenance::sha3_hex), NOT in SQL (no pgcrypto dependency).
    #[derive(sqlx::FromRow)]
    struct TrialVerdict {
        trial_num: i32,
        test_id: i32,
        passed: bool,
    }
    let verdicts: Vec<TrialVerdict> = sqlx::query_as(
        r#"SELECT trial_num, test_id, passed
           FROM trial_results
           WHERE run_id = $1 AND is_infra_error = false
           ORDER BY trial_num"#,
    )
    .bind(req.run_id)
    .fetch_all(&state.db)
    .await?;

    let evidence = verdicts
        .iter()
        .map(|v| format!("{}:{}:{}", v.trial_num, v.test_id, v.passed))
        .collect::<Vec<_>>()
        .join("|");
    let sha3 = crate::executor::provenance::sha3_hex(&evidence);

    sqlx::query(
        r#"UPDATE test_runs
           SET status = 'done',
               finished_at = NOW(),
               pass_count = $1,
               total_count = $2,
               sha3_provenance = $3
           WHERE id = $4"#,
    )
    .bind(pass_count)
    .bind(total_count)
    .bind(&sha3)
    .bind(req.run_id)
    .execute(&state.db)
    .await?;

    Ok(Json(SessionResult {
        run_id: req.run_id,
        status: "done".into(),
        pass_count,
        total_count,
        sha3_provenance: Some(sha3),
    }))
}
