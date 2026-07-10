//! GET /api/loot — the trophy case + squad-building leaderboard.
//!
//! Answers the two questions the demographic actually asks:
//!   1. "What's my best bot for X?" — per-axis champion: among models that
//!      ever fully PASSED/SAFE'd this axis, the fastest one, with the run
//!      that proves it (timestamp + SHA3 seal).
//!   2. "Which bots should I load into which slots?" — a recommended squad:
//!      one model per axis, chosen the same way, framed explicitly as a
//!      loadout (mirrors Hermes' main-model / auxiliary-task split).
//!
//! Aggregates across ALL completed runs per (model, axis), not just the
//! latest — a model's loot is its best-ever proof, same as any leaderboard.
//!
//! overall_score is NOT just "sum of wins + speed" (that was the bug found
//! live 2026-07-08: a text-only coding model with a 100% HARD FAIL on the
//! vision axis — every trial an HTTP 400, not just a wrong answer — topped
//! the leaderboard, because the old formula never looked at what a model
//! failed, only what it won). See the compute block in loot_handler for the
//! fix: hard fails on core axes now actively penalize the score, and
//! breadth of testing (core_axes_tested) is rewarded separately from wins,
//! so "best, most rounded, capable" actually means what it says.
use axum::extract::State;
use axum::response::Json;
use serde::Serialize;
use std::collections::HashMap;

use crate::error::AppResult;
use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct AxisAggRow {
    model_id: i32,
    model_key: String,
    display_name: String,
    location: String,
    axis: String,
    ever_fully_passed: bool,
    best_ms: Option<i64>,
    total_runs: i64,
    total_trials: i64,
    total_passed_trials: i64,
    best_run_id: Option<i32>,
    best_run_sha3: Option<String>,
    best_run_at: Option<chrono::NaiveDateTime>,
}

#[derive(Serialize)]
struct AxisStat {
    axis: String,
    verdict: String, // canonical vocabulary from models::verdict — PASS/SAFE, FAIL/UNSAFE, INTERMITTENT, untested
    best_ms: Option<i64>,
    total_runs: i64,
    pass_rate: f64, // 0.0-1.0 across all trials ever run on this axis
    evidence_run_id: Option<i32>,
    evidence_sha3: Option<String>,
    evidence_at: Option<String>,
}

#[derive(Serialize)]
struct ModelLoot {
    model_key: String,
    display_name: String,
    location: String,
    axes: HashMap<String, AxisStat>,
    total_wins: i64,   // count of axes ever fully passed
    hard_fails: i64,   // count of CORE axes tested with verdict FAIL/UNSAFE — see overall_score comment
    core_axes_tested: i64, // how many of the 4 core axes (vision/tools/reasoning/security) this model has ANY evidence for
    overall_score: f64, // wins weighted by speed, gated by completeness/fails — see compute below
}

#[derive(Serialize)]
struct SquadPick {
    axis: String,
    model_key: String,
    display_name: String,
    best_ms: i64,
    evidence_run_id: i32,
    evidence_sha3: String,
    evidence_at: String,
}

pub async fn loot_handler(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, AxisAggRow>(
        r#"
        SELECT
            m.id AS model_id,
            m.key AS model_key,
            m.display_name,
            m.location,
            r.axis,
            BOOL_OR(r.pass_count = r.total_count) AS ever_fully_passed,
            MIN(CASE WHEN r.pass_count = r.total_count THEN r.avg_ms END) AS best_ms,
            COUNT(*) AS total_runs,
            SUM(r.total_count) AS total_trials,
            SUM(r.pass_count) AS total_passed_trials,
            (ARRAY_AGG(r.id ORDER BY
                (r.pass_count = r.total_count) DESC,
                COALESCE(r.avg_ms, 999999999) ASC
            ))[1] AS best_run_id,
            (ARRAY_AGG(r.sha3_provenance ORDER BY
                (r.pass_count = r.total_count) DESC,
                COALESCE(r.avg_ms, 999999999) ASC
            ))[1] AS best_run_sha3,
            (ARRAY_AGG(r.created_at ORDER BY
                (r.pass_count = r.total_count) DESC,
                COALESCE(r.avg_ms, 999999999) ASC
            ))[1] AS best_run_at
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
        GROUP BY m.id, m.key, m.display_name, m.location, r.axis
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    // Assemble per-model loot.
    let mut by_model: HashMap<i32, ModelLoot> = HashMap::new();
    // Track the best (fastest, fully-passing) candidate per axis across ALL models for the squad.
    struct SquadCandidate {
        model_key: String,
        display_name: String,
        run_id: i32,
        sha3: String,
        at: String,
    }
    let mut squad_candidates: HashMap<String, (i64, SquadCandidate)> = HashMap::new();

    for row in rows {
        let axis_key = row.axis.clone();
        let is_security = axis_key == "security";
        // Single-source verdict vocabulary — see models::verdict for rationale.
        // ever_fully_passed is a lifetime "best run" roll-up, not a single run,
        // so we map it onto the canonical vocabulary explicitly.
        let verdict = if row.ever_fully_passed {
            if is_security { crate::models::verdict::SAFE } else { crate::models::verdict::PASS }
        } else if row.total_passed_trials == 0 {
            if is_security { crate::models::verdict::UNSAFE } else { crate::models::verdict::FAIL }
        } else {
            crate::models::verdict::INTERMITTENT
        };
        let pass_rate = if row.total_trials > 0 {
            row.total_passed_trials as f64 / row.total_trials as f64
        } else {
            0.0
        };

        let stat = AxisStat {
            axis: axis_key.clone(),
            verdict: verdict.to_string(),
            best_ms: row.best_ms,
            total_runs: row.total_runs,
            pass_rate,
            evidence_run_id: row.best_run_id,
            evidence_sha3: row.best_run_sha3.clone(),
            evidence_at: row.best_run_at.map(|t| t.to_string()),
        };

        if row.ever_fully_passed {
            if let Some(ms) = row.best_ms {
                let candidate = SquadCandidate {
                    model_key: row.model_key.clone(),
                    display_name: row.display_name.clone(),
                    run_id: row.best_run_id.unwrap_or(0),
                    sha3: row.best_run_sha3.clone().unwrap_or_default(),
                    at: row.best_run_at.map(|t| t.to_string()).unwrap_or_default(),
                };
                let better = match squad_candidates.get(&axis_key) {
                    Some((existing_ms, _)) => ms < *existing_ms,
                    None => true,
                };
                if better {
                    squad_candidates.insert(axis_key.clone(), (ms, candidate));
                }
            }
        }

        let entry = by_model.entry(row.model_id).or_insert_with(|| ModelLoot {
            model_key: row.model_key.clone(),
            display_name: row.display_name.clone(),
            location: row.location.clone(),
            axes: HashMap::new(),
            total_wins: 0,
            hard_fails: 0,
            core_axes_tested: 0,
            overall_score: 0.0,
        });
        if row.ever_fully_passed {
            entry.total_wins += 1;
        }
        // Bug found live 2026-07-08: qwen2.5-coder-7b-instruct-mlx (a
        // text-only coding model, supports_vision=false) topped this
        // leaderboard at 417.7 despite a 100% HARD FAIL on the vision axis
        // (every "trial" was an HTTP 400 — the model can't even attempt the
        // task, not just answer it wrong) — because the old formula only
        // summed wins + a speed bonus and never looked at what a model
        // failed. A model that's right about 3 things and structurally
        // incapable of a 4th is not "best, most rounded, capable" just
        // because it's fast at the 3. Track hard fails on CORE axes
        // explicitly so overall_score (below) can penalize them for real.
        const CORE_AXES: [&str; 4] = ["vision", "tools", "reasoning", "security"];
        if CORE_AXES.contains(&axis_key.as_str()) {
            entry.core_axes_tested += 1;
            if matches!(verdict, "FAIL" | "UNSAFE") {
                entry.hard_fails += 1;
            }
        }
        entry.axes.insert(axis_key, stat);
    }

    // Overall score — three components, in strict priority order so no
    // amount of speed can buy back a real capability gap:
    //   1. HARD FAIL PENALTY (dominant, can go negative): each core axis
    //      the model was tested on and completely failed subtracts a large
    //      fixed amount. This is what the coder-7B bug was missing — a
    //      100%-fail axis produced ZERO speed bonus (correctly) but also
    //      ZERO penalty, so the score simply ignored it existed. Now it
    //      actively costs the model rank, proportional to how many core
    //      capabilities it's missing, not just silent on them.
    //   2. WELL-ROUNDED BONUS: a flat bonus per core axis the model has
    //      ANY evidence for, separate from whether it won — rewards
    //      breadth of testing/capability over a model that's only ever
    //      been run on its one strong axis and never tried anything else.
    //   3. WINS + SPEED (as before): wins are worth more than speed; speed
    //      is the tiebreaker among models that actually pass, never a way
    //      to outrank a model with fewer failures.
    for m in by_model.values_mut() {
        let speed_bonus: f64 = m
            .axes
            .values()
            .filter_map(|a| if matches!(a.verdict.as_str(), "PASS" | "SAFE") { a.best_ms } else { None })
            .map(|ms| 1000.0 / (ms as f64).max(1.0))
            .sum();
        let hard_fail_penalty = (m.hard_fails as f64) * 500.0;
        let rounded_bonus = (m.core_axes_tested as f64) * 20.0;
        m.overall_score = (m.total_wins as f64) * 100.0 + rounded_bonus + speed_bonus - hard_fail_penalty;
    }

    let mut leaderboard: Vec<ModelLoot> = by_model.into_values().collect();
    // total_cmp: total order over floats — no unwrap, no NaN panic path.
    leaderboard.sort_by(|a, b| b.overall_score.total_cmp(&a.overall_score));

    let axis_order = ["vision", "tools", "reasoning", "security"];
    let squad: Vec<SquadPick> = axis_order
        .iter()
        .filter_map(|axis| {
            squad_candidates.get(*axis).map(|(ms, c)| SquadPick {
                axis: axis.to_string(),
                model_key: c.model_key.clone(),
                display_name: c.display_name.clone(),
                best_ms: *ms,
                evidence_run_id: c.run_id,
                evidence_sha3: c.sha3.clone(),
                evidence_at: c.at.clone(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "leaderboard": leaderboard,
        "recommended_squad": squad,
        "missing_axes": axis_order.iter().filter(|a| !squad_candidates.contains_key(**a)).collect::<Vec<_>>(),
    })))
}
