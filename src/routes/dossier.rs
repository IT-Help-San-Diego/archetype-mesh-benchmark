//! GET /api/models/{key}/dossier — every factual thing we know about a model.
//!
//! User request (2026-07-09): "when they click on a model, it should give
//! them every single factual thing we know about it. Not just the testing."
//! One endpoint, three evidence classes, each labeled with its source:
//!   1. REGISTRY  — our models row (key, provider, location, context, vision
//!      flag, tags, notes, first-seen/last-seen timestamps)
//!   2. LIVE      — LM Studio's /api/v0/models entry RIGHT NOW (state,
//!      quantization, arch, type, max context) for local models; reported
//!      honestly as unreachable/absent when it isn't there
//!   3. EVIDENCE  — full per-axis lifetime record (every run, pass rates,
//!      latencies, seals) + per-test breakdown with the exact test names
//!
//! (Router placement lives at /api/router/plan — the UI links there rather
//! than duplicating the policy computation here.)
//!
//! Same discipline as everywhere else: measured or null, never invented.
use axum::extract::{Path, State};
use axum::response::Json;
use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(sqlx::FromRow, Serialize)]
struct RegistryRow {
    id: i32,
    key: String,
    display_name: String,
    provider: String,
    location: String,
    context_length: i32,
    supports_vision: bool,
    size_gb: f64,
    notes: Option<String>,
    tags: Option<Vec<String>>,
    active: bool,
    created_at: Option<chrono::NaiveDateTime>,
    updated_at: Option<chrono::NaiveDateTime>,
}

#[derive(sqlx::FromRow)]
struct AxisEvidenceRow {
    axis: String,
    total_runs: i64,
    total_trials: i64,
    total_passed: i64,
    infra_errors: i64,
    best_ms: Option<i64>,
    avg_ms: Option<i64>,
    first_tested: Option<chrono::NaiveDateTime>,
    last_tested: Option<chrono::NaiveDateTime>,
    latest_run_id: Option<i32>,
    latest_sha3: Option<String>,
}

#[derive(sqlx::FromRow)]
struct TestEvidenceRow {
    test_id: Option<i32>,
    test_name: Option<String>,
    axis: String,
    trials: i64,
    passed: i64,
    avg_ms: Option<i64>,
}

pub async fn model_dossier(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    // ── 1. REGISTRY ──────────────────────────────────────────────────────
    let registry: RegistryRow = sqlx::query_as(
        r#"SELECT id, key, display_name, provider, location, context_length,
                  supports_vision, size_gb, notes, tags, active, created_at, updated_at
           FROM models WHERE key = $1"#,
    )
    .bind(&key)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Executor(format!("Unknown model key: {key}")))?;
    let model_id = registry.id;

    // ── 2. LIVE (local models: ask LM Studio right now) ─────────────────
    let live = if registry.location == "local" {
        match reqwest::Client::new()
            .get(format!("{}/api/v0/models", state.config.lmstudio_base_url))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
                Ok(json) => {
                    let entry = json["data"]
                        .as_array()
                        .and_then(|a| a.iter().find(|m| m["id"].as_str() == Some(key.as_str())));
                    match entry {
                        Some(m) => serde_json::json!({
                            "reachable": true, "present": true,
                            "state": m["state"], "type": m["type"],
                            "arch": m["arch"], "quantization": m["quantization"],
                            "compatibility_type": m["compatibility_type"],
                            "max_context_length": m["max_context_length"],
                            "loaded_context_length": m["loaded_context_length"],
                            "source": "GET /api/v0/models (LM Studio, live)",
                        }),
                        None => serde_json::json!({
                            "reachable": true, "present": false,
                            "note": "LM Studio is up but this key is not in its current library",
                        }),
                    }
                }
                Err(_) => serde_json::json!({ "reachable": false }),
            },
            _ => serde_json::json!({ "reachable": false, "note": "LM Studio not reachable right now" }),
        }
    } else {
        serde_json::json!({ "applicable": false, "note": "cloud model — no local live state" })
    };

    // ── 3. EVIDENCE: per-axis lifetime aggregate ─────────────────────────
    let axes: Vec<AxisEvidenceRow> = sqlx::query_as(
        r#"
        SELECT r.axis,
               COUNT(DISTINCT r.id) AS total_runs,
               COUNT(tr.id) AS total_trials,
               COUNT(tr.id) FILTER (WHERE tr.passed) AS total_passed,
               COUNT(tr.id) FILTER (WHERE tr.is_infra_error) AS infra_errors,
               MIN(tr.latency_ms) FILTER (WHERE tr.latency_ms >= 0) AS best_ms,
               ROUND(AVG(tr.latency_ms) FILTER (WHERE tr.latency_ms >= 0))::bigint AS avg_ms,
               MIN(r.created_at) AS first_tested,
               MAX(r.created_at) AS last_tested,
               (ARRAY_AGG(r.id ORDER BY r.created_at DESC))[1] AS latest_run_id,
               (ARRAY_AGG(r.sha3_provenance ORDER BY r.created_at DESC))[1] AS latest_sha3
        FROM test_runs r
        LEFT JOIN trial_results tr ON tr.run_id = r.id
        WHERE r.model_id = $1 AND r.status = 'done'
        GROUP BY r.axis ORDER BY r.axis
        "#,
    )
    .bind(model_id)
    .fetch_all(&state.db)
    .await?;

    // ── 3b. EVIDENCE: per-test breakdown (via the provenance link) ──────
    let per_test: Vec<TestEvidenceRow> = sqlx::query_as(
        r#"
        SELECT tr.test_id, t.name AS test_name, r.axis,
               COUNT(*) AS trials,
               COUNT(*) FILTER (WHERE tr.passed) AS passed,
               ROUND(AVG(tr.latency_ms) FILTER (WHERE tr.latency_ms >= 0))::bigint AS avg_ms
        FROM trial_results tr
        JOIN test_runs r ON r.id = tr.run_id
        LEFT JOIN tests t ON t.id = tr.test_id
        WHERE r.model_id = $1 AND r.status = 'done'
        GROUP BY tr.test_id, t.name, r.axis
        ORDER BY r.axis, t.name NULLS LAST
        "#,
    )
    .bind(model_id)
    .fetch_all(&state.db)
    .await?;

    let axes_json: Vec<serde_json::Value> = axes
        .iter()
        .map(|a| {
            let capability_trials = a.total_trials - a.infra_errors;
            serde_json::json!({
                "axis": a.axis,
                "total_runs": a.total_runs,
                "total_trials": a.total_trials,
                "total_passed": a.total_passed,
                "infra_errors": a.infra_errors,
                "pass_rate": if capability_trials > 0 { a.total_passed as f64 / capability_trials as f64 } else { 0.0 },
                "best_ms": a.best_ms,
                "avg_ms": a.avg_ms,
                "first_tested": a.first_tested.map(|t| t.to_string()),
                "last_tested": a.last_tested.map(|t| t.to_string()),
                "latest_run_id": a.latest_run_id,
                "latest_sha3": a.latest_sha3,
            })
        })
        .collect();

    let per_test_json: Vec<serde_json::Value> = per_test
        .iter()
        .map(|t| {
            serde_json::json!({
                "test_id": t.test_id,
                // NULL test_id = trials recorded before the provenance link
                // existed (migration 021) — labeled, not hidden.
                "test_name": t.test_name.clone().unwrap_or_else(|| "(pre-linkage evidence)".into()),
                "axis": t.axis,
                "trials": t.trials,
                "passed": t.passed,
                "avg_ms": t.avg_ms,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "key": key,
        "registry": registry,
        "live": live,
        "evidence": { "axes": axes_json, "per_test": per_test_json },
        "sources": {
            "registry": "models table (synced from LM Studio / seeded config)",
            "live": "LM Studio REST /api/v0/models at request time",
            "evidence": "test_runs + trial_results, completed runs only, SHA3-sealed",
        },
    })))
}
