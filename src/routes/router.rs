//! GET /api/router/plan — the capability router.
//!
//! Turns accumulated benchmark evidence into an explicit, auditable routing
//! plan: for each capability axis, which model should a job be dispatched to,
//! which models are acceptable fallbacks, and which are excluded — with a
//! stated reason and a sealed evidence pointer for every single placement.
//!
//! WHY THIS IS NOT JUST /api/loot's SQUAD
//! The loot squad answers "fastest model to EVER fully pass" — a trophy-case
//! question, correct for a leaderboard. A router must answer a stricter
//! question: "which model do I trust with real work?" One lucky 3/3 run is a
//! win; it is not a routing basis. This endpoint therefore classifies on the
//! AGGREGATE record (every trial ever run on the axis, infra errors already
//! excluded upstream by the executor) and demands a minimum evidence count
//! before promoting anything to primary.
//!
//! POLICY (deterministic, echoed in the response so every plan is
//! self-describing — a consumer can always tell WHICH policy produced it):
//!   PRIMARY   — 100% lifetime pass rate (integer compare, not float) AND
//!               total_trials >= min_trials (default 3). Among eligible,
//!               fastest best_ms wins; more trials breaks ties.
//!   FALLBACK  — either (a) 100% pass rate but not enough trials yet
//!               ("promising, under-evidenced"), or (b) pass rate >=
//!               fallback_threshold (default 0.8) but imperfect ("flaky").
//!               Perfect-but-slower primaries also land here, ranked first —
//!               they are genuinely the best fallbacks.
//!   EXCLUDED  — pass rate below the fallback threshold, or zero passes.
//!               Every exclusion carries a human-readable reason; silence is
//!               how the coder-7B leaderboard bug happened (2026-07-08) and
//!               we do not repeat it here.
//!   UNTESTED models simply do not appear — absence of evidence is absence
//!               from the plan, never a verdict.
//!
//! Read-only: no writes, no new tables. Pure decision function over
//! test_runs + models, the same substrate the leaderboard uses.
use axum::extract::{Query, State};
use axum::response::Json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Display / iteration order. Auxiliary is included — routing local models
/// onto Hermes' auxiliary tasks is the founding use case for this feature.
const AXIS_ORDER: [&str; 6] = ["vision", "tools", "reasoning", "security", "literary", "auxiliary"];

const DEFAULT_MIN_TRIALS: i64 = 3;
const DEFAULT_FALLBACK_THRESHOLD: f64 = 0.8;

#[derive(Debug, Deserialize)]
pub struct RouterParams {
    /// Minimum lifetime trials on an axis before a perfect record can be
    /// promoted to PRIMARY. Bounds-checked: 1..=1000.
    pub min_trials: Option<i64>,
    /// Minimum lifetime pass rate to stay routable as a FALLBACK.
    /// Bounds-checked: 0.0 < t <= 1.0.
    pub fallback_threshold: Option<f64>,
    /// Optional location filter ("local" | "cloud") — plan a local-only or
    /// cloud-only loadout. Omit for the full fleet.
    pub location: Option<String>,
}

#[derive(sqlx::FromRow)]
struct RouteAggRow {
    model_key: String,
    display_name: String,
    location: String,
    axis: String,
    total_runs: i64,
    total_trials: i64,
    total_passed: i64,
    best_ms: Option<i64>,
    latest_run_id: Option<i32>,
    latest_sha3: Option<String>,
    latest_at: Option<chrono::NaiveDateTime>,
}

#[derive(Serialize, Clone)]
struct Evidence {
    run_id: Option<i32>,
    sha3: Option<String>,
    at: Option<String>,
}

#[derive(Serialize, Clone)]
struct RoutedModel {
    model_key: String,
    display_name: String,
    location: String,
    pass_rate: f64,
    total_trials: i64,
    total_runs: i64,
    best_ms: Option<i64>,
    /// Why this model sits in this tier — always present, always specific.
    reason: String,
    /// Latest completed run on this axis: the freshest sealed justification.
    evidence: Evidence,
}

#[derive(Serialize)]
struct AxisPlan {
    axis: String,
    /// "routed" (primary exists) | "degraded" (fallbacks only) | "unrouted".
    status: String,
    primary: Option<RoutedModel>,
    fallbacks: Vec<RoutedModel>,
    excluded: Vec<RoutedModel>,
}

pub async fn router_plan(
    State(state): State<AppState>,
    Query(params): Query<RouterParams>,
) -> AppResult<Json<serde_json::Value>> {
    // ── Validate policy knobs up front; actionable 400s, never silent clamps ──
    let min_trials = params.min_trials.unwrap_or(DEFAULT_MIN_TRIALS);
    if !(1..=1000).contains(&min_trials) {
        return Err(AppError::Executor(format!(
            "min_trials must be between 1 and 1000, got {min_trials}"
        )));
    }
    let threshold = params.fallback_threshold.unwrap_or(DEFAULT_FALLBACK_THRESHOLD);
    if !(threshold > 0.0 && threshold <= 1.0) {
        return Err(AppError::Executor(format!(
            "fallback_threshold must be in (0.0, 1.0], got {threshold}"
        )));
    }
    if let Some(loc) = &params.location {
        if loc != "local" && loc != "cloud" {
            return Err(AppError::Executor(format!(
                "location must be 'local' or 'cloud', got '{loc}'"
            )));
        }
    }

    // Aggregate the full lifetime record per (model, axis). Inner subquery
    // matches loot.rs exactly: completed runs with at least one real trial;
    // infra-errored trials were already excluded from total_count by the
    // executor, so these numbers are capability evidence, not noise.
    let rows = sqlx::query_as::<_, RouteAggRow>(
        r#"
        SELECT
            m.key AS model_key,
            m.display_name,
            m.location,
            r.axis,
            COUNT(*) AS total_runs,
            SUM(r.total_count) AS total_trials,
            SUM(r.pass_count) AS total_passed,
            MIN(CASE WHEN r.pass_count = r.total_count THEN r.avg_ms END) AS best_ms,
            (ARRAY_AGG(r.id ORDER BY r.created_at DESC))[1] AS latest_run_id,
            (ARRAY_AGG(r.sha3_provenance ORDER BY r.created_at DESC))[1] AS latest_sha3,
            (ARRAY_AGG(r.created_at ORDER BY r.created_at DESC))[1] AS latest_at
        FROM (
            SELECT tr.*,
                   (SELECT ROUND(AVG(t.latency_ms))::bigint
                    FROM trial_results t
                    WHERE t.run_id = tr.id AND t.latency_ms >= 0) AS avg_ms
            FROM test_runs tr
            WHERE tr.status = 'done' AND tr.total_count > 0
        ) r
        JOIN models m ON m.id = r.model_id
        WHERE m.active = true
          AND ($1::text IS NULL OR m.location = $1)
        GROUP BY m.key, m.display_name, m.location, r.axis
        "#,
    )
    .bind(params.location.as_deref())
    .fetch_all(&state.db)
    .await?;

    // ── Classify every (model, axis) record into a tier ──────────────────
    enum Tier {
        PrimaryEligible,
        Fallback,
        Excluded,
    }

    /// Per-axis working buckets, filled during classification.
    #[derive(Default)]
    struct AxisBuckets {
        eligible: Vec<RoutedModel>,
        fallbacks: Vec<RoutedModel>,
        excluded: Vec<RoutedModel>,
    }

    let mut per_axis: BTreeMap<String, AxisBuckets> = BTreeMap::new();

    for row in rows {
        // total_trials > 0 is guaranteed by the WHERE clause (total_count > 0).
        let pass_rate = row.total_passed as f64 / row.total_trials as f64;
        let perfect = row.total_passed == row.total_trials; // integer compare — no float-equality trap
        let is_security = row.axis == "security";
        let verb = if is_security { "resisted" } else { "passed" };

        let (tier, reason) = if perfect && row.total_trials >= min_trials {
            (
                Tier::PrimaryEligible,
                format!(
                    "verified: {verb} {}/{} trials across {} run(s)",
                    row.total_passed, row.total_trials, row.total_runs
                ),
            )
        } else if perfect {
            (
                Tier::Fallback,
                format!(
                    "under-evidenced: {verb} {}/{} trials, but policy requires ≥{} before primary",
                    row.total_passed, row.total_trials, min_trials
                ),
            )
        } else if pass_rate >= threshold {
            (
                Tier::Fallback,
                format!(
                    "unstable: {verb} {}/{} trials ({:.0}%) — meets the {:.0}% fallback floor, not routable as primary",
                    row.total_passed,
                    row.total_trials,
                    pass_rate * 100.0,
                    threshold * 100.0
                ),
            )
        } else if row.total_passed == 0 {
            (
                Tier::Excluded,
                format!(
                    "structural fail: 0/{} trials ever {verb} across {} run(s)",
                    row.total_trials, row.total_runs
                ),
            )
        } else {
            (
                Tier::Excluded,
                format!(
                    "below floor: {verb} {}/{} trials ({:.0}%) — under the {:.0}% fallback threshold",
                    row.total_passed,
                    row.total_trials,
                    pass_rate * 100.0,
                    threshold * 100.0
                ),
            )
        };

        let routed = RoutedModel {
            model_key: row.model_key,
            display_name: row.display_name,
            location: row.location,
            pass_rate,
            total_trials: row.total_trials,
            total_runs: row.total_runs,
            best_ms: row.best_ms,
            reason,
            evidence: Evidence {
                run_id: row.latest_run_id,
                sha3: row.latest_sha3,
                at: row.latest_at.map(|t| t.to_string()),
            },
        };

        let entry = per_axis.entry(row.axis).or_default();
        match tier {
            Tier::PrimaryEligible => entry.eligible.push(routed),
            Tier::Fallback => entry.fallbacks.push(routed),
            Tier::Excluded => entry.excluded.push(routed),
        }
    }

    // ── Rank and assemble per-axis plans in the canonical axis order ─────
    let plans: Vec<AxisPlan> = AXIS_ORDER
        .iter()
        .map(|axis| {
            let AxisBuckets { mut eligible, mut fallbacks, mut excluded } =
                per_axis.remove(*axis).unwrap_or_default();

            // Primary ranking: fastest verified wins; more evidence breaks
            // ties; key is the deterministic final tiebreaker (stable plans
            // across identical data — a router that reorders on refresh with
            // unchanged evidence is broken).
            eligible.sort_by(|a, b| {
                a.best_ms
                    .unwrap_or(i64::MAX)
                    .cmp(&b.best_ms.unwrap_or(i64::MAX))
                    .then(b.total_trials.cmp(&a.total_trials))
                    .then(a.model_key.cmp(&b.model_key))
            });

            let mut iter = eligible.into_iter();
            let primary = iter.next();

            // Remaining verified models are the best possible fallbacks —
            // they outrank flaky ones by construction.
            let mut verified_rest: Vec<RoutedModel> = iter
                .map(|mut m| {
                    m.reason = format!("{} — slower than primary", m.reason);
                    m
                })
                .collect();
            // Flaky/under-evidenced fallbacks: highest pass rate first, then
            // evidence volume, then key for determinism. Vocabulary note
            // (user mandate 2026-07-08): reason strings use "unstable", not
            // "flaky" — the router is a science surface, and every claim on
            // it carries its measurement (n, pass rate) in the same breath.
            // "Flaky" stays in the Loot tab's fun register only.
            fallbacks.sort_by(|a, b| {
                b.pass_rate
                    .total_cmp(&a.pass_rate)
                    .then(b.total_trials.cmp(&a.total_trials))
                    .then(a.model_key.cmp(&b.model_key))
            });
            verified_rest.append(&mut fallbacks);

            excluded.sort_by(|a, b| {
                b.pass_rate
                    .total_cmp(&a.pass_rate)
                    .then(a.model_key.cmp(&b.model_key))
            });

            let status = if primary.is_some() {
                "routed"
            } else if !verified_rest.is_empty() {
                "degraded"
            } else {
                "unrouted"
            };

            AxisPlan {
                axis: axis.to_string(),
                status: status.to_string(),
                primary,
                fallbacks: verified_rest,
                excluded,
            }
        })
        .collect();

    Ok(Json(serde_json::json!({
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "policy": {
            "min_trials": min_trials,
            "fallback_threshold": threshold,
            "location_filter": params.location,
            "rules": [
                "PRIMARY: 100% lifetime pass rate AND total_trials >= min_trials; fastest wins, evidence volume breaks ties",
                "FALLBACK: 100% but under-evidenced, or pass_rate >= fallback_threshold; verified-but-slower rank above unstable",
                "EXCLUDED: pass_rate below fallback_threshold; every exclusion states its reason",
                "UNTESTED: absent from the plan entirely — absence of evidence is not a verdict"
            ],
        },
        "axes": plans,
    })))
}
