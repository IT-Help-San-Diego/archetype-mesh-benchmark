//! Test-run executor: the scientific core.
//!
//! One run = one (model, axis) execution of every active test on that axis.
//! Pipeline (each phase streamed live over the SSE broadcast — no spinners,
//! real telemetry): clean-room prep (local) → prompt assembly (server-side,
//! ground truth never sent to the model) → N trials → objective scoring →
//! verdict → SHA3-512 provenance → persist.
pub mod cloud;
pub mod lmstudio;
pub mod scoring;
pub mod provenance;

use base64::Engine;
use sqlx::PgPool;
use tokio::sync::broadcast;

use crate::config::Config;
use crate::error::{AppError, AppResult};
use crate::models::tests::TestDef;

/// Emit one telemetry envelope to every open SSE connection.
/// Best-effort: zero subscribers is not an error (runs still persist evidence).
fn emit(tx: &broadcast::Sender<String>, value: serde_json::Value) {
    if let Ok(json) = serde_json::to_string(&value) {
        let _ = tx.send(json);
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Load every active test for an axis.
pub async fn tests_for_axis(db: &PgPool, axis: &str) -> AppResult<Vec<TestDef>> {
    let rows = sqlx::query_as::<_, TestDef>(
        r#"SELECT id, name, axis, prompt_text, attachment_path, attachment_sha3,
                  expected_result, scoring_method, trials_per_run
           FROM tests WHERE active = true AND axis = $1 ORDER BY id"#,
    )
    .bind(axis)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// Build the OpenAI-shaped user message for a test.
/// Anti-cheat invariants enforced here:
///   1. expected_result is NEVER part of the payload.
///   2. If the test pins an attachment hash, the actual bytes on disk are
///      re-hashed and MUST match before anything is sent.
fn build_messages(
    test: &TestDef,
    project_root: &std::path::Path,
) -> AppResult<Vec<serde_json::Value>> {
    match &test.attachment_path {
        Some(rel_path) => {
            let full = project_root.join(rel_path);
            let bytes = std::fs::read(&full).map_err(|e| {
                AppError::Executor(format!("Attachment {} unreadable: {}", full.display(), e))
            })?;

            if let Some(pinned) = &test.attachment_sha3 {
                let actual = provenance::sha3_256_bytes(&bytes);
                if &actual != pinned {
                    return Err(AppError::Executor(format!(
                        "Attachment hash mismatch for test {} — pinned {} but disk has {}. \
                         Evidence integrity violated; refusing to run.",
                        test.name, pinned, actual
                    )));
                }
            }

            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            Ok(vec![serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": test.prompt_text},
                    {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", b64)}}
                ]
            })])
        }
        None => Ok(vec![serde_json::json!({
            "role": "user",
            "content": test.prompt_text
        })]),
    }
}

/// Execute one full run: all active tests on `axis` against `model_key`.
/// Persists test_runs + trial_results + verdict + SHA3-512 provenance.
#[allow(clippy::too_many_arguments)]
pub async fn execute_run(
    db: PgPool,
    config: Config,
    tx: broadcast::Sender<String>,
    run_id: i32,
    model_id: i32,
    model_key: String,
    location: String,
    provider: String,
    axis: String,
) {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(RUN_BUDGET_SECS),
        execute_run_inner(
            &db, &config, &tx, run_id, model_id, &model_key, &location, &provider, &axis,
        ),
    )
    .await
    // Budget expiry maps onto the same error path as any other failure:
    // status='error', finished, telemetry emitted. Completed trials are
    // already persisted row-by-row, so partial evidence survives.
    .unwrap_or_else(|_| {
        Err(AppError::Executor(format!(
            "Run exceeded the {}-minute wall-clock budget and was aborted to protect the machine. \
             Trials completed before the cutoff are preserved in trial_results.",
            RUN_BUDGET_SECS / 60
        )))
    });

    if let Err(e) = result {
        tracing::error!("Run {} failed: {}", run_id, e);
        let _ = sqlx::query("UPDATE test_runs SET status = 'error', finished_at = NOW() WHERE id = $1")
            .bind(run_id)
            .execute(&db)
            .await;
        emit(
            &tx,
            serde_json::json!({
                "type": "error", "run_id": run_id, "message": e.to_string(), "at": now_iso()
            }),
        );
    }
}

/// Hard wall-clock budget per run. This machine is someone's daily driver:
/// a pathological model (endless reasoning loops, thrashing swap) must never
/// silently grind the GPU for hours through a terminal the user can't see.
/// Worst case without this: 300s load + 33 trials x 300s timeout ≈ 3 hours
/// for ONE queued run. With it: the run aborts honestly at the budget,
/// records whatever trials completed, and frees the machine.
const RUN_BUDGET_SECS: u64 = 1800; // 30 minutes

#[allow(clippy::too_many_arguments)]
async fn execute_run_inner(
    db: &PgPool,
    config: &Config,
    tx: &broadcast::Sender<String>,
    run_id: i32,
    _model_id: i32,
    model_key: &str,
    location: &str,
    provider: &str,
    axis: &str,
) -> AppResult<()> {
    let client = reqwest::Client::new();

    sqlx::query("UPDATE test_runs SET status = 'loading', started_at = NOW() WHERE id = $1")
        .bind(run_id)
        .execute(db)
        .await?;
    emit(tx, serde_json::json!({
        "type": "run_started", "run_id": run_id, "model_key": model_key,
        "axis": axis, "location": location, "at": now_iso()
    }));

    // ── Clean-room prep (local models only) ────────────────────────────────
    if location == "local" {
        emit(tx, serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "ejecting",
            "message": "Clean room: ejecting all loaded models from LM Studio", "at": now_iso()
        }));
        let ejected = lmstudio::eject_all(&client, &config.lmstudio_base_url).await?;
        emit(tx, serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "ejected",
            "message": format!("Ejected {} instance(s): {:?}", ejected.len(), ejected), "at": now_iso()
        }));

        emit(tx, serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "loading",
            "message": format!("Loading {} — watch LM Studio's server tab", model_key), "at": now_iso()
        }));
        let load_start = std::time::Instant::now();
        let resident =
            lmstudio::ensure_loaded(&client, &config.lmstudio_base_url, model_key, 300).await?;
        if !resident {
            return Err(AppError::Executor(format!(
                "{} did not become resident within 300s",
                model_key
            )));
        }
        emit(tx, serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "resident",
            "message": format!("{} verified resident in RAM ({}s load)", model_key, load_start.elapsed().as_secs()),
            "at": now_iso()
        }));
    }

    // ── Trials ─────────────────────────────────────────────────────────────
    let tests = tests_for_axis(db, axis).await?;
    if tests.is_empty() {
        return Err(AppError::Executor(format!("No active tests for axis '{}'", axis)));
    }

    sqlx::query("UPDATE test_runs SET status = 'running' WHERE id = $1")
        .bind(run_id)
        .execute(db)
        .await?;

    let mut pass_count: i32 = 0;
    let mut total_count: i32 = 0;
    let mut evidence_lines: Vec<String> = Vec::new();

    for test in &tests {
        let n_trials = test.trials_per_run.unwrap_or(3).max(1);
        emit(tx, serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "trial",
            "message": format!("Test '{}' — {} trial(s)", test.name, n_trials), "at": now_iso()
        }));

        let messages = build_messages(test, &config.project_root)?;

        for trial_num in 1..=n_trials {
            let outcome = match location {
                "local" => {
                    lmstudio::chat(&client, &config.lmstudio_base_url, model_key, &messages, 512, 0.0)
                        .await
                }
                _ => {
                    let config_key = match provider {
                        "nous" => &config.nous_api_key,
                        "openrouter" => &config.openrouter_api_key,
                        other => {
                            return Err(AppError::Executor(format!("Unknown provider: {}", other)))
                        }
                    };
                    // Resolved per run (not at process start): Nous OAuth agent
                    // keys rotate on the order of hours.
                    let key = cloud::resolve_api_key(provider, config_key)?;
                    cloud::chat(&client, provider, &key, model_key, &messages, 512).await
                }
            };

            total_count += 1;
            let (passed, latency_ms, raw, detail) = match outcome {
                Ok((response, latency)) => {
                    let expected = test.expected_result.as_deref().unwrap_or("");
                    let score = scoring::score_response(&response, expected, &test.scoring_method);
                    (score.passed, latency as i64, response, score.detail.unwrap_or_default())
                }
                Err(e) => (false, -1, String::new(), format!("execution error: {}", e)),
            };
            if passed {
                pass_count += 1;
            }

            sqlx::query(
                r#"INSERT INTO trial_results (run_id, trial_num, raw_response, latency_ms, passed, detail)
                   VALUES ($1, $2, $3, $4, $5, $6)"#,
            )
            .bind(run_id)
            .bind(trial_num)
            .bind(&raw)
            .bind(latency_ms)
            .bind(passed)
            .bind(&detail)
            .execute(db)
            .await?;

            evidence_lines.push(format!(
                "test={} trial={} passed={} latency_ms={} response={}",
                test.name, trial_num, passed, latency_ms, raw
            ));

            emit(tx, serde_json::json!({
                "type": "trial_result", "run_id": run_id, "test": test.name,
                "trial_num": trial_num, "passed": passed, "latency_ms": latency_ms,
                "detail": detail, "at": now_iso()
            }));
        }
    }

    // ── Verdict + provenance ───────────────────────────────────────────────
    emit(tx, serde_json::json!({
        "type": "phase", "run_id": run_id, "phase": "scoring",
        "message": format!("Scoring: {}/{} trials passed", pass_count, total_count), "at": now_iso()
    }));

    // Lean language: "unsafe" is a security claim, not a capability claim.
    // Security axis keeps SAFE/UNSAFE; capability axes report PASS/FAIL.
    let verdict = if pass_count == total_count {
        if axis == "security" { "SAFE" } else { "PASS" }
    } else if pass_count == 0 {
        if axis == "security" { "UNSAFE" } else { "FAIL" }
    } else {
        "FLAKY"
    };

    let evidence_record = format!(
        "run_id={} model={} axis={} pass={}/{}\n{}",
        run_id, model_key, axis, pass_count, total_count,
        evidence_lines.join("\n")
    );
    let sha3 = provenance::sha3_hex(&evidence_record);

    sqlx::query(
        r#"UPDATE test_runs
           SET status = 'done', finished_at = NOW(),
               pass_count = $2, total_count = $3, sha3_provenance = $4
           WHERE id = $1"#,
    )
    .bind(run_id)
    .bind(pass_count)
    .bind(total_count)
    .bind(&sha3)
    .execute(db)
    .await?;

    emit(tx, serde_json::json!({
        "type": "verdict", "run_id": run_id, "overall": verdict,
        "pass_count": pass_count, "total_count": total_count, "at": now_iso()
    }));
    emit(tx, serde_json::json!({
        "type": "run_complete", "run_id": run_id, "overall": verdict,
        "sha3": sha3, "at": now_iso()
    }));

    Ok(())
}

/// Prompt length validation — heuristic by default, zero inference cost.
///
/// IMPORTANT: LM Studio's REST API has NO standalone tokenizer endpoint
/// (verified empirically 2026-07-07: /api/tokenize, /api/v0/tokenize, and
/// every OpenAI-compat variant all 404 with "Unexpected endpoint"). The only
/// way to get an EXACT count is to actually call chat/completions and read
/// `usage.prompt_tokens` back — which loads the model and burns a sliver of
/// real inference (max_tokens=1). That's a genuine trade-off, not a free
/// lunch, so it's exposed as an explicit opt-in (see `verify_prompt_length_live`)
/// rather than silently attempted here.
/// Returns (tokens, context_limit, fits, note).
pub fn validate_prompt_length(prompt_text: &str, context_limit: i64) -> (i64, i64, bool, String) {
    let char_count = prompt_text.chars().count() as i64;
    // Rough: 1 token ≈ 3.5 chars for English/markdown; pad 20% for safety margin
    // since this estimate has no ground truth to check itself against.
    let estimated = ((char_count as f64 / 3.5) * 1.2).ceil() as i64;
    let fits = estimated <= context_limit;
    let note = format!(
        "~{} tokens (estimated from {} chars, 20% safety margin) / {} ctx — heuristic only, no live tokenizer exists on LM Studio's REST API",
        estimated, char_count, context_limit
    );
    (estimated, context_limit, fits, note)
}

/// Optional LIVE verification: fires one real max_tokens=1 chat completion
/// at the target model and reads the EXACT prompt token count back from
/// `usage.prompt_tokens`. This is real inference — it loads the model if not
/// resident and costs a sliver of compute/time. Use only when the user
/// explicitly asks for exact numbers, never as the default check.
pub async fn verify_prompt_length_live(
    client: &reqwest::Client,
    lmstudio_base_url: &str,
    model_key: &str,
    prompt_text: &str,
    context_limit: i64,
) -> AppResult<(i64, i64, bool, String)> {
    let body = serde_json::json!({
        "model": model_key,
        "messages": [{"role": "user", "content": prompt_text}],
        "max_tokens": 1,
    });

    let resp = client
        .post(format!("{}/api/v0/chat/completions", lmstudio_base_url))
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(AppError::Executor(format!(
            "Live check rejected by LM Studio (HTTP {}): {}. This itself is informative — it likely means the prompt overflowed the context window.",
            status, body_text.chars().take(200).collect::<String>()
        )));
    }

    let json: serde_json::Value = resp.json().await?;
    let exact = json
        .get("usage")
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|t| t.as_i64())
        .ok_or_else(|| AppError::Executor("LM Studio response had no usage.prompt_tokens".to_string()))?;

    let fits = exact <= context_limit;
    let pct = if context_limit > 0 { (exact as f64 / context_limit as f64 * 100.0).round() as i64 } else { 0 };
    let note = format!(
        "{} tokens EXACT (live LM Studio count) / {} ctx window ({}%) — {}",
        exact, context_limit, pct, if fits { "FITS" } else { "OVERFLOW" }
    );
    Ok((exact, context_limit, fits, note))
}
