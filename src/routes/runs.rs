//! POST /api/runs — start a benchmark run. GET /api/runs — run history.
//!
//! POST body: {"model_key": "...", "axes": ["vision","tools",...]}
//! One test_runs row is created per requested axis; each executes as its own
//! background task. Local-model runs are serialized by a global mutex so two
//! clean-room runs can never fight over LM Studio RAM residency.
use axum::extract::State;
use axum::response::Json;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

// "auxiliary" is an experimental axis (added 2026-07-07) tracking whether
// non-frontier / local models are reliable enough for Hermes' auxiliary
// tasks (approval classification, MCP sampling relay) — see migration 009.
// Deliberately kept separate from the core 4-axis capability grid.
const VALID_AXES: [&str; 6] = ["vision", "tools", "reasoning", "security", "literary", "auxiliary"];

#[derive(Debug, Deserialize)]
pub struct StartRunRequest {
    pub model_key: String,
    /// Whole-axis batteries to run. Optional when `test_ids` is provided
    /// (modular segment run) — at least one of `axes`/`test_ids` is required.
    #[serde(default)]
    pub axes: Vec<String>,
    #[serde(default)]
    pub load_mode: Option<LoadMode>,
    #[serde(default)]
    pub draft_model_key: Option<String>,
    #[serde(default)]
    pub scaffold_supplement: Option<String>,
    /// Optional disambiguator for the 8 model keys that exist as BOTH a
    /// local (lmstudio) and a cloud (nous) row. Without it, `WHERE key = $1`
    /// picks an arbitrary twin — so a local pick could silently run in the
    /// cloud (and vice-versa), and the confirm dialog's Source label lies.
    /// The frontend now sends the exact provider of the card the user clicked.
    #[serde(default)]
    pub provider: Option<String>,
    /// Optional explicit test set. When provided, the run executes ONLY these
    /// test IDs (across any axes) instead of a whole axis — modular segments.
    /// Validated server-side: every ID must exist and be active.
    #[serde(default)]
    pub test_ids: Option<Vec<i32>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LoadMode {
    CleanRoom,
    SpeculativePair,
    Scaffolded,
}

#[derive(sqlx::FromRow)]
struct ModelRow {
    id: i32,
    key: String,
    provider: String,
    location: String,
    supports_vision: bool,
}

pub async fn start_runs(
    State(state): State<AppState>,
    Json(req): Json<StartRunRequest>,
) -> AppResult<Json<serde_json::Value>> {
    // A run needs SOMETHING to execute: either whole axes, or an explicit
    // modular test_ids set. Reject empty requests so we never spawn a
    // no-op run that would confuse the run history.
    if req.axes.is_empty() && req.test_ids.as_ref().map(|v| v.is_empty()).unwrap_or(true) {
        return Err(AppError::Executor(
            "provide at least one axis or a non-empty test_ids set".into(),
        ));
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

    // Disambiguate dual-location keys (local+cloud twins). When the caller
    // names a provider, match it exactly; otherwise fall back to key-only
    // (preserves old behaviour for the ~297 unique keys).
    let model = match req.provider.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(prov) => sqlx::query_as::<_, ModelRow>(
            "SELECT id, key, provider, location, supports_vision FROM models WHERE key = $1 AND provider = $2 AND active = true",
        )
        .bind(&req.model_key)
        .bind(prov)
        .fetch_optional(&state.db)
        .await?,
        None => sqlx::query_as::<_, ModelRow>(
            "SELECT id, key, provider, location, supports_vision FROM models WHERE key = $1 AND active = true",
        )
        .bind(&req.model_key)
        .fetch_optional(&state.db)
        .await?,
    }
    .ok_or_else(|| AppError::Executor(format!("Unknown model key: {}", req.model_key)))?;

    // Capability pre-flight, at the API boundary — the cheapest possible
    // place to refuse a job a model is already known to be unable to do.
    // Found live 2026-07-08 auditing historical data: every vision-axis run
    // against a supports_vision=false model came back 100%
    // infra-contaminated (LM Studio rejects the request outright; the model
    // never gets a chance to answer). Silently SKIP the incompatible axis
    // rather than reject the whole request — the model grid's "▶ Run"
    // button asks for all 4 core axes on every model, including non-vision
    // ones, so a hard rejection here would break Run for most of the fleet.
    // The executor also carries this same check (defense in depth against
    // any other caller of execute_run), but skipping it HERE means we never
    // even create a queued row, let alone spend a clean-room load cycle.
    // Vision-skip contract: a whole-axis run that ONLY requested vision on a
    // model with no vision support is rejected (not silently emptied). For a
    // modular test_ids run, `axes` is empty by design — skip this axis
    // validation entirely (the executor's per-test vision gate still applies).
    let mut skipped_axes: Vec<(&str, String)> = Vec::new();
    if req.test_ids.is_none() {
    axes.retain(|axis| {
        if *axis == "vision" && !model.supports_vision {
            skipped_axes.push((
                axis,
                format!("{} has no vision support (LM Studio capabilities metadata)", model.key),
            ));
            false
        } else {
            true
        }
    });
    if axes.is_empty() {
        return Err(AppError::Executor(format!(
            "Every requested axis was skipped as incompatible with {}: {}",
            model.key,
            skipped_axes.iter().map(|(_, r)| r.as_str()).collect::<Vec<_>>().join("; ")
        )));
    }
    }

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

    let load_mode = req.load_mode.unwrap_or(LoadMode::CleanRoom);
    if matches!(load_mode, LoadMode::SpeculativePair) {
        if req.draft_model_key.as_ref().map(|s| s.trim().is_empty()).unwrap_or(false) {
            return Err(AppError::Executor(
                "draft_model_key is required when load_mode is 'speculative-pair'".into(),
            ));
        }
    }

    let mut run_ids = Vec::new();
    // MODULAR SEGMENTS: an explicit test_ids set runs ONLY those tests
    // (across any axes) as one 'custom' run — instead of a whole axis.
    if let Some(ids) = &req.test_ids {
        if ids.is_empty() {
            return Err(AppError::Executor("test_ids was provided but empty — pass at least one test ID or omit the field to run a whole axis".into()));
        }
        // Validate every ID exists and is active; reject the whole request
        // (no partial run) if any ID is missing/inactive. This is the same
        // "objective verdict, no silent failure" discipline as the axis path.
        let mut valid: Vec<i32> = Vec::new();
        for &id in ids {
            let active: Option<bool> = sqlx::query_scalar(
                "SELECT active FROM tests WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
            match active {
                Some(true) => valid.push(id),
                _ => return Err(AppError::Executor(format!(
                    "Test id {} is not an active test — refusing the modular run (no partial execution)",
                    id
                ))),
            }
        }

        // One run row, axis='custom'. test_ids are persisted on the run row
        // so the executor (and any replay/audit) knows the exact set.
        let ids_json = serde_json::to_string(&valid).unwrap_or_else(|_| "[]".to_string());
        let (run_id,): (i32,) = sqlx::query_as(
            r#"INSERT INTO test_runs (model_id, test_id, axis, status, load_mode, draft_model_key, scaffold_supplement, test_ids)
               VALUES ($1, NULL, 'custom', 'queued', $2, $3, $4, $5::jsonb) RETURNING id"#,
        )
        .bind(model.id)
        .bind(match load_mode {
            LoadMode::CleanRoom => "clean-room",
            LoadMode::SpeculativePair => "speculative-pair",
            LoadMode::Scaffolded => "scaffolded",
        })
        .bind(req.draft_model_key.clone())
        .bind(req.scaffold_supplement.clone())
        .bind(ids_json)
        .fetch_one(&state.db)
        .await?;

        spawn_run_task(
            &state,
            run_id,
            &model,
            "custom".to_string(),
            load_mode.clone(),
            req.draft_model_key.clone(),
            req.scaffold_supplement.clone(),
            req.test_ids.clone(),
        );
        run_ids.push(run_id);
    }

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
            r#"INSERT INTO test_runs (model_id, test_id, axis, status, load_mode, draft_model_key, scaffold_supplement)
               VALUES ($1, NULL, $2, 'queued', $3, $4, $5) RETURNING id"#,
        )
        .bind(model.id)
        .bind(axis)
        .bind(match load_mode {
            LoadMode::CleanRoom => "clean-room",
            LoadMode::SpeculativePair => "speculative-pair",
            LoadMode::Scaffolded => "scaffolded",
        })
        .bind(req.draft_model_key.clone())
        // Persist the scaffold text ON the run row. The supplement used to
        // ride only the in-memory param into execute_run — runs 657-674
        // (the whole scaffold-framing arc) are sealed with NULL here, so
        // the exact prompt wrapper had to be recovered from session notes.
        // Provenance means the row itself must carry what the model saw.
        .bind(req.scaffold_supplement.clone())
        .fetch_one(&state.db)
        .await?;
        run_ids.push(run_id);

        spawn_run_task(
            &state,
            run_id,
            &model,
            axis.to_string(),
            load_mode.clone(),
            req.draft_model_key.clone(),
            req.scaffold_supplement.clone(),
            req.test_ids.clone(),
        );
    }

    Ok(Json(serde_json::json!({
        "run_id": run_ids[0],
        "run_ids": run_ids,
        "model_key": req.model_key,
        "axes": axes,
        "skipped_axes": skipped_axes.iter().map(|(a, reason)| serde_json::json!({"axis": a, "reason": reason})).collect::<Vec<_>>(),
    })))
}

/// Spawn the background task that executes ONE queued run. Shared by
/// `start_runs` and `start_baseline_scaffold` so the LM Studio serialization
/// gate and GPU-telemetry RAII guard are impossible to fork out of sync.
fn spawn_run_task(
    state: &AppState,
    run_id: i32,
    model: &ModelRow,
    axis: String,
    load_mode: LoadMode,
    draft_model_key: Option<String>,
    scaffold_supplement: Option<String>,
    test_ids: Option<Vec<i32>>,
) {
    let db = state.db.clone();
    let config = state.config.clone();
    let tx = state.events_tx.clone();
    let cancellations = state.cancellations.clone();
    let active_runs = state.active_runs.clone();
    let model_id = model.id;
    let model_key = model.key.clone();
    let location = model.location.clone();
    let provider = model.provider.clone();

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
        // RAII: while this guard lives, the 1Hz GPU telemetry sampler
        // (gpu_telemetry.rs) is active and streaming gpu_sample events.
        let _telemetry = active_runs.guard();
        crate::executor::execute_run(
            db, config, tx, cancellations, run_id, model_id, model_key, location, provider,
            axis, load_mode, draft_model_key, scaffold_supplement, test_ids,
        )
        .await;
    });
}

/// POST /api/runs/baseline-scaffold — queue a PAIRED experiment: one
/// clean-room baseline run and one scaffolded run per axis, same model,
/// same battery, atomically. This is the controlled-comparison primitive
/// the scaffold experiments (2026-07-13/14) ran by hand: baseline first,
/// scaffold second, so the pair lands adjacent in run history and the
/// scaffold_delta is computable from two sealed runs.
///
/// The frontend has called this endpoint since the scaffold work landed;
/// the route itself was never written (404, then 405 after the {id} route
/// grew a POST sibling). Found live 2026-07-14.
#[derive(Debug, Deserialize)]
pub struct BaselineScaffoldRequest {
    pub model_key: String,
    pub axes: Vec<String>,
    #[serde(default)]
    pub scaffold_supplement: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
}

pub async fn start_baseline_scaffold(
    State(state): State<AppState>,
    Json(req): Json<BaselineScaffoldRequest>,
) -> AppResult<Json<serde_json::Value>> {
    if req.axes.is_empty() {
        return Err(AppError::Executor("axes must be non-empty".into()));
    }
    let supplement = req
        .scaffold_supplement
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let Some(supplement) = supplement else {
        return Err(AppError::Executor(
            "scaffold_supplement is required for a baseline-vs-scaffold pair — without it the two runs would be identical".into(),
        ));
    };

    // Same axis validation/dedup discipline as start_runs.
    let mut axes: Vec<&str> = Vec::new();
    for axis in &req.axes {
        if !VALID_AXES.contains(&axis.as_str()) {
            return Err(AppError::Executor(format!("Invalid axis: {}", axis)));
        }
        if !axes.contains(&axis.as_str()) {
            axes.push(axis.as_str());
        }
    }

    let model = match req.provider.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(prov) => sqlx::query_as::<_, ModelRow>(
            "SELECT id, key, provider, location, supports_vision FROM models WHERE key = $1 AND provider = $2 AND active = true",
        )
        .bind(&req.model_key)
        .bind(prov)
        .fetch_optional(&state.db)
        .await?,
        None => sqlx::query_as::<_, ModelRow>(
            "SELECT id, key, provider, location, supports_vision FROM models WHERE key = $1 AND active = true",
        )
        .bind(&req.model_key)
        .fetch_optional(&state.db)
        .await?,
    }
    .ok_or_else(|| AppError::Executor(format!("Unknown model key: {}", req.model_key)))?;

    // Vision capability pre-flight — same skip-not-reject contract as start_runs.
    let mut skipped_axes: Vec<(&str, String)> = Vec::new();
    axes.retain(|axis| {
        if *axis == "vision" && !model.supports_vision {
            skipped_axes.push((
                axis,
                format!("{} has no vision support (LM Studio capabilities metadata)", model.key),
            ));
            false
        } else {
            true
        }
    });
    if axes.is_empty() {
        return Err(AppError::Executor(format!(
            "Every requested axis was skipped as incompatible with {}",
            model.key
        )));
    }

    // In-flight duplicate guard, one check per axis (covers both halves of
    // the pair: if EITHER a baseline or scaffold run is queued/running for
    // this (model, axis), refuse the whole pair rather than interleave).
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
                "A '{}' run for {} is already queued or running — the baseline/scaffold pair must start on an idle model",
                axis, model.key
            )));
        }
    }

    let mut baseline_run_ids = Vec::new();
    let mut scaffold_run_ids = Vec::new();
    for axis in &axes {
        let (test_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM tests WHERE active = true AND axis = $1")
                .bind(axis)
                .fetch_one(&state.db)
                .await?;
        if test_count == 0 {
            return Err(AppError::Executor(format!("No active tests for axis '{}'", axis)));
        }

        // Baseline half: clean-room, NO supplement.
        let (baseline_id,): (i32,) = sqlx::query_as(
            r#"INSERT INTO test_runs (model_id, test_id, axis, status, load_mode)
               VALUES ($1, NULL, $2, 'queued', 'clean-room') RETURNING id"#,
        )
        .bind(model.id)
        .bind(axis)
        .fetch_one(&state.db)
        .await?;
        baseline_run_ids.push(baseline_id);

        // Scaffold half: scaffolded, WITH supplement (persisted on the row
        // so the run detail view shows exactly what the model was given).
        let (scaffold_id,): (i32,) = sqlx::query_as(
            r#"INSERT INTO test_runs (model_id, test_id, axis, status, load_mode, scaffold_supplement)
               VALUES ($1, NULL, $2, 'queued', 'scaffolded', $3) RETURNING id"#,
        )
        .bind(model.id)
        .bind(axis)
        .bind(&supplement)
        .fetch_one(&state.db)
        .await?;
        scaffold_run_ids.push(scaffold_id);

        // Queue both halves. The lm_guard gate inside spawn_run_task
        // serializes local execution, so baseline runs to completion before
        // the scaffold half loads — adjacent, ordered, comparable.
        spawn_run_task(&state, baseline_id, &model, axis.to_string(), LoadMode::CleanRoom, None, None, None);
        spawn_run_task(
            &state,
            scaffold_id,
            &model,
            axis.to_string(),
            LoadMode::Scaffolded,
            None,
            Some(supplement.clone()),
            None,
        );
    }

    Ok(Json(serde_json::json!({
        "model_key": req.model_key,
        "axes": axes,
        "baseline_run_ids": baseline_run_ids,
        "scaffold_run_ids": scaffold_run_ids,
        "skipped_axes": skipped_axes.iter().map(|(a, reason)| serde_json::json!({"axis": a, "reason": reason})).collect::<Vec<_>>(),
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

/// Row shape for the per-run trial detail — the audit view. User request:
/// "put them into verbose mode... judge them against that too" — this is
/// where a stored reasoning trace is actually surfaced for historical runs,
/// not just live SSE telemetry (which is a compact one-line-per-trial
/// stream, not meant for after-the-fact judgment).
#[derive(sqlx::FromRow, serde::Serialize)]
struct TrialDetailRow {
    id: i32,
    trial_num: i32,
    raw_response: Option<String>,
    reasoning_content: Option<String>,
    latency_ms: Option<i64>,
    passed: bool,
    detail: Option<String>,
    is_infra_error: bool,
    speculative_draft_model: Option<String>,
    total_draft_tokens_count: Option<i64>,
    accepted_draft_tokens_count: Option<i64>,
    rejected_draft_tokens_count: Option<i64>,
    test_name: Option<String>,
    formal_spec: Option<String>,
}

#[derive(sqlx::FromRow)]
struct RunDetailHeader {
    id: i32,
    key: String,
    axis: String,
    status: String,
    pass_count: i32,
    total_count: i32,
    sha3_provenance: Option<String>,
    created_at: Option<chrono::NaiveDateTime>,
    finished_at: Option<chrono::NaiveDateTime>,
    load_mode: Option<String>,
    draft_model_key: Option<String>,
    lmstudio_runtime_config: Option<serde_json::Value>,
}

pub async fn get_run_detail(
    State(state): State<AppState>,
    axum::extract::Path(run_id): axum::extract::Path<i32>,
) -> AppResult<Json<serde_json::Value>> {
    let header: Option<RunDetailHeader> = sqlx::query_as(
        r#"SELECT r.id, m.key, r.axis, r.status, r.pass_count, r.total_count,
                  r.sha3_provenance, r.created_at, r.finished_at, r.load_mode, r.draft_model_key,
                  r.lmstudio_runtime_config
           FROM test_runs r JOIN models m ON m.id = r.model_id
           WHERE r.id = $1"#,
    )
    .bind(run_id)
    .fetch_optional(&state.db)
    .await?;

    let Some(header) = header else {
        return Err(AppError::Executor(format!("No run with id {}", run_id)));
    };

    let trials: Vec<TrialDetailRow> = sqlx::query_as(
        r#"SELECT tr.id, tr.trial_num, tr.raw_response, tr.reasoning_content, tr.latency_ms, tr.passed, tr.detail, tr.is_infra_error,
                  tr.speculative_draft_model, tr.total_draft_tokens_count, tr.accepted_draft_tokens_count, tr.rejected_draft_tokens_count,
                  t.name as test_name, t.formal_spec
           FROM trial_results tr
           LEFT JOIN tests t ON t.id = tr.test_id
           WHERE tr.run_id = $1 ORDER BY tr.id"#,
    )
    .bind(run_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(serde_json::json!({
        "id": header.id,
        "model_key": header.key,
        "axis": header.axis,
        "status": header.status,
        "pass_count": header.pass_count,
        "total_count": header.total_count,
        "load_mode": header.load_mode,
        "draft_model_key": header.draft_model_key,
        "lmstudio_runtime_config": header.lmstudio_runtime_config,
        "sha3_provenance": header.sha3_provenance,
        "created_at": header.created_at.map(|c| c.to_string()),
        "finished_at": header.finished_at.map(|c| c.to_string()),
        "trials": trials,
    })))
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

/// GET /api/runs/:id/export — the evidence bundle.
///
/// Data stewardship contract: everything the database knows about a run
/// leaves in ONE self-describing JSON document — header, runtime config,
/// every trial with its full raw response, reasoning trace, latency,
/// speculative-decode counters, the test text and formal spec it ran
/// against, and the SHA3-512 provenance seal. A scientist can archive it,
/// diff it, or re-verify the seal offline without this app existing.
/// Content-Disposition makes the browser download it as a named artifact.
pub async fn export_run(
    State(state): State<AppState>,
    axum::extract::Path(run_id): axum::extract::Path<i32>,
) -> AppResult<axum::response::Response> {
    use axum::response::IntoResponse;

    let header: Option<RunDetailHeader> = sqlx::query_as(
        r#"SELECT r.id, m.key, r.axis, r.status, r.pass_count, r.total_count,
                  r.sha3_provenance, r.created_at, r.finished_at, r.load_mode, r.draft_model_key,
                  r.lmstudio_runtime_config
           FROM test_runs r JOIN models m ON m.id = r.model_id
           WHERE r.id = $1"#,
    )
    .bind(run_id)
    .fetch_optional(&state.db)
    .await?;

    let Some(header) = header else {
        return Err(AppError::Executor(format!("No run with id {}", run_id)));
    };

    // Full trial evidence, including the test prompt text — the export must
    // stand alone: prompt + response + verdict + seal, nothing left behind.
    #[derive(sqlx::FromRow, serde::Serialize)]
    struct ExportTrial {
        trial_num: i32,
        test_name: Option<String>,
        prompt_text: Option<String>,
        expected_result: Option<String>,
        formal_spec: Option<String>,
        raw_response: Option<String>,
        reasoning_content: Option<String>,
        latency_ms: Option<i64>,
        passed: Option<bool>,
        detail: Option<String>,
        is_infra_error: Option<bool>,
        speculative_draft_model: Option<String>,
        total_draft_tokens_count: Option<i64>,
        accepted_draft_tokens_count: Option<i64>,
        rejected_draft_tokens_count: Option<i64>,
    }
    let trials: Vec<ExportTrial> = sqlx::query_as(
        r#"SELECT tr.trial_num, t.name as test_name, t.prompt_text, t.expected_result, t.formal_spec,
                  tr.raw_response, tr.reasoning_content, tr.latency_ms, tr.passed, tr.detail,
                  tr.is_infra_error, tr.speculative_draft_model, tr.total_draft_tokens_count,
                  tr.accepted_draft_tokens_count, tr.rejected_draft_tokens_count
           FROM trial_results tr
           LEFT JOIN tests t ON t.id = tr.test_id
           WHERE tr.run_id = $1 ORDER BY tr.id"#,
    )
    .bind(run_id)
    .fetch_all(&state.db)
    .await?;

    let bundle = serde_json::json!({
        "format": "calibration-scope-evidence-bundle",
        "format_version": 1,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "run": {
            "id": header.id,
            "model_key": header.key,
            "axis": header.axis,
            "status": header.status,
            "pass_count": header.pass_count,
            "total_count": header.total_count,
            "load_mode": header.load_mode,
            "draft_model_key": header.draft_model_key,
            "lmstudio_runtime_config": header.lmstudio_runtime_config,
            "sha3_provenance": header.sha3_provenance,
            "created_at": header.created_at.map(|c| c.to_string()),
            "finished_at": header.finished_at.map(|c| c.to_string()),
        },
        "trials": trials,
        "verification": {
            "seal_algorithm": "SHA3-512",
            "note": "sha3_provenance is computed over the ordered trial evidence; recompute offline to verify integrity without this application."
        }
    });

    let filename = format!("amb-run-{}-{}-evidence.json", header.id, header.axis);
    let body = serde_json::to_string_pretty(&bundle)
        .map_err(|e| AppError::Executor(format!("serialize export: {}", e)))?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "application/json; charset=utf-8".to_string()),
            (axum::http::header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename)),
        ],
        body,
    )
        .into_response())
}
