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
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::error::{AppError, AppResult};
use crate::models::tests::TestDef;

/// One chat completion's full measurement record. Both executors (local
/// LM Studio, cloud providers) return this shape so the trial loop treats
/// them identically. Token counts come from the response's `usage` object —
/// the provider's own meter, read back verbatim; None when the provider
/// omitted usage. Cost is NEVER computed here: dollars = tokens × catalog
/// unit price, derived at read time (see migration 024).
#[derive(Debug)]
pub struct ChatOutcome {
    pub content: String,
    pub reasoning_content: Option<String>,
    pub latency_ms: u64,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
}

/// Parse usage.{prompt_tokens,completion_tokens} from an OpenAI-shaped
/// response body. Shared by both executors; absent fields stay None.
pub(crate) fn usage_tokens(json: &serde_json::Value) -> (Option<i64>, Option<i64>) {
    let get = |field: &str| {
        json.pointer(&format!("/usage/{}", field))
            .and_then(|v| v.as_i64())
            .filter(|n| *n >= 0)
    };
    (get("prompt_tokens"), get("completion_tokens"))
}

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
    cancellations: crate::lm_guard::CancellationRegistry,
    run_id: i32,
    model_id: i32,
    model_key: String,
    location: String,
    provider: String,
    axis: String,
) {
    let cancel_token = cancellations.register(run_id).await;

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(RUN_BUDGET_SECS),
        execute_run_inner(
            &db, &config, &tx, &cancel_token, run_id, model_id, &model_key, &location, &provider,
            &axis,
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

    // Always unregister on every exit path — otherwise the cancellation map
    // grows by one entry per run for the life of the process.
    cancellations.unregister(run_id).await;

    match result {
        Ok(()) => {}
        Err(AppError::Aborted) => {
            tracing::info!("Run {} aborted by operator request", run_id);
            let _ = sqlx::query(
                "UPDATE test_runs SET status = 'aborted', finished_at = NOW() WHERE id = $1",
            )
            .bind(run_id)
            .execute(&db)
            .await;
            emit(
                &tx,
                serde_json::json!({
                    "type": "aborted", "run_id": run_id,
                    "message": "Run stopped by operator request.", "at": now_iso()
                }),
            );
        }
        Err(e) => {
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
    cancel_token: &CancellationToken,
    run_id: i32,
    model_id: i32,
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

    // Pre-flight capability gate: refuse a vision-axis run against a model
    // LM Studio's own metadata already says has no vision support, BEFORE
    // spending a clean-room eject+load cycle and a real GPU inference
    // attempt on a request that's guaranteed to be rejected. Found live
    // 2026-07-08 by auditing historical data: EVERY vision-axis run against
    // a supports_vision=false model (10 of 10 checked — every harmonic-
    // hermes-9b quant, granite-3.2-8b, granite-4-h-tiny, llama-3.2-3b,
    // qwen2.5-coder-7b-instruct-mlx, hermes-4-14b) came back 100%
    // infra-contaminated (HTTP 400, the model never got to answer). This is
    // the simplest, most certain form of "never hand a model a job it
    // can't do": we already HAVE the ground truth (LM Studio told us this
    // model has no vision), so there's no need to spend a real load+
    // inference cycle to discover the same fact again every single run.
    if axis == "vision" {
        let supports_vision: Option<bool> =
            sqlx::query_scalar("SELECT supports_vision FROM models WHERE id = $1")
                .bind(model_id)
                .fetch_optional(db)
                .await?;
        if supports_vision == Some(false) {
            return Err(AppError::Executor(format!(
                "{} has no vision support (LM Studio capabilities metadata) — refusing to spend a \
                 clean-room load + inference attempt on a request guaranteed to fail. This is not a \
                 capability FAIL; the model was correctly never asked. If this model has gained vision \
                 support since the last LM Studio sync, run a sync and try again.",
                model_key
            )));
        }
    }

    // Every await point below that can take real wall-clock time (clean-room
    // ejection, model load, each chat call) races against cancel_token so an
    // operator-triggered abort takes effect within that single step instead
    // of only at trial boundaries. select! biases neither branch — whichever
    // resolves first wins, so a cancellation racing an already-completing
    // call still lets the call finish and simply stops before the NEXT step.
    macro_rules! cancellable {
        ($fut:expr) => {
            tokio::select! {
                res = $fut => res,
                _ = cancel_token.cancelled() => return Err(AppError::Aborted),
            }
        };
    }

/// Estimate the RAM cost of loading a model. For models with a speculative-
/// decoding draft model configured, BOTH the main and draft model must fit —
/// loading them together is what caused the 2026-07-09 kernel watchdog panic
/// (gemma-4-31b ~22GB + gemma-4-12b-qat draft ~6GB + background downloads +
/// Docker → 94-second hang → forced reboot). This guard ensures the benchmark
/// never crashes someone's machine, which is core to the mission: this tool
/// is designed to help people on constrained hardware.
async fn check_memory_safety(
    db: &PgPool,
    tx: &broadcast::Sender<String>,
    run_id: i32,
    model_id: i32,
    model_key: &str,
) -> AppResult<()> {
    // Get the model's size from the DB (size_gb, optional — some models lack it)
    let model_size_gb: Option<f64> =
        sqlx::query_scalar("SELECT size_gb FROM models WHERE id = $1")
            .bind(model_id)
            .fetch_optional(db)
            .await?;

    // Read available memory from macOS vm_stat.
    // free + inactive + purgeable pages are reclaimable; we need the model
    // to fit with a safety margin so the system doesn't thrash.
    let page_size = 16384usize; // macOS arm64 default
    let vm_stat = std::process::Command::new("vm_stat")
        .output()
        .map_err(|e| AppError::Executor(format!("Cannot read vm_stat: {}", e)))?;
    let vm_text = String::from_utf8_lossy(&vm_stat.stdout);

    let mut free_pages: u64 = 0;
    let mut inactive_pages: u64 = 0;
    let mut purgeable_pages: u64 = 0;
    for line in vm_text.lines() {
        if let Some(rest) = line.strip_prefix("Pages free:") {
            free_pages = rest.trim().trim_end_matches('.').parse().unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("Pages inactive:") {
            inactive_pages = rest.trim().trim_end_matches('.').parse().unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("Pages purgeable:") {
            purgeable_pages = rest.trim().trim_end_matches('.').parse().unwrap_or(0);
        }
    }

    let available_bytes = (free_pages + inactive_pages + purgeable_pages) as usize * page_size;
    let available_gb = available_bytes as f64 / 1_073_741_824.0;

    // If we know the model size from LM Studio sync, use it.
    // Otherwise estimate from the model's total bytes on disk if available.
    // As a fallback, refuse if free memory is very low regardless.
    if let Some(model_gb) = model_size_gb {
        // Speculative-decoding models load a draft model too — estimate 25%
        // overhead for the draft (conservative; the actual draft is usually
        // much smaller, but we'd rather over-provision than panic).
        let estimated_gb = model_gb * 1.25;
        let safety_margin_gb = 8.0; // leave headroom for OS + apps + inference
        let needed_gb = estimated_gb + safety_margin_gb;

        emit(tx, serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "memory_check",
            "message": format!(
                "Memory check: {} needs ~{:.1} GB (model {:.1} + 25% spec-decode overhead + 8 GB safety), available: {:.1} GB",
                model_key, needed_gb, model_gb, available_gb
            ),
            "at": now_iso()
        }));

        if available_gb < needed_gb {
            return Err(AppError::Executor(format!(
                "MEMORY GUARD: Refusing to load {} — estimated {:.1} GB needed (model {:.1} GB + draft overhead + 8 GB safety) \
                 but only {:.1} GB available. Loading this model now could destabilize the system. \
                 Close background applications, pause model downloads, or use a smaller quant.",
                model_key, needed_gb, model_gb, available_gb
            )));
        }
    } else {
        // No size data — still refuse if the system is critically low on memory.
        if available_gb < 12.0 {
            return Err(AppError::Executor(format!(
                "MEMORY GUARD: Only {:.1} GB available — refusing to load {} without sufficient free memory. \
                 Close background applications and try again.",
                available_gb, model_key
            )));
        }
        emit(tx, serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "memory_check",
            "message": format!("Memory check: {:.1} GB available (model size unknown, using minimum guard)", available_gb),
            "at": now_iso()
        }));
    }

    Ok(())
}

    // ── Clean-room prep (local models only) ────────────────────────────────
    if location == "local" {
        emit(tx, serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "ejecting",
            "message": "Clean room: ejecting all loaded models from LM Studio", "at": now_iso()
        }));
        let ejected = cancellable!(lmstudio::eject_all(&client, &config.lmstudio_base_url))?;
        emit(tx, serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "ejected",
            "message": format!("Ejected {} instance(s): {:?}", ejected.len(), ejected), "at": now_iso()
        }));

        // Memory guard: refuse to load if the model won't fit safely.
        // See check_memory_safety doc comment for the kernel-panic incident
        // that motivated this — speculative-decoding pairs can exceed RAM
        // and freeze the system, especially on smaller hardware.
        check_memory_safety(db, tx, run_id, model_id, model_key).await?;

        emit(tx, serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "loading",
            "message": format!("Loading {} — watch LM Studio's server tab", model_key), "at": now_iso()
        }));
        let load_start = std::time::Instant::now();
        let resident = cancellable!(lmstudio::ensure_loaded(
            &client,
            &config.lmstudio_base_url,
            model_key,
            300
        ))?;
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
    let mut infra_error_count: i32 = 0;
    let mut evidence_lines: Vec<String> = Vec::new();

    for test in &tests {
        let n_trials = test.trials_per_run.unwrap_or(3).max(1);
        emit(tx, serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "trial",
            "message": format!("Test '{}' — {} trial(s)", test.name, n_trials), "at": now_iso()
        }));

        let messages = build_messages(test, &config.project_root)?;

        for trial_num in 1..=n_trials {
            // Also checked here (not just inside cancellable! around the
            // network call) so a cancel between trials doesn't have to wait
            // for one more full trial to start before taking effect.
            if cancel_token.is_cancelled() {
                return Err(AppError::Aborted);
            }
            let outcome = match location {
                "local" => {
                    cancellable!(lmstudio::chat(
                        &client,
                        &config.lmstudio_base_url,
                        model_key,
                        &messages,
                        512,
                        0.0
                    ))
                }
                _ => {
                    let config_key = match provider {
                        "nous" => &config.nous_api_key,
                        "openrouter" => &config.openrouter_api_key,
                        "openai" => &config.openai_api_key,
                        other => {
                            return Err(AppError::Executor(format!("Unknown provider: {}", other)))
                        }
                    };
                    // Resolved per run (not at process start): Nous OAuth agent
                    // keys rotate on the order of hours.
                    let key = cloud::resolve_api_key(provider, config_key)?;
                    cancellable!(cloud::chat(&client, provider, &key, model_key, &messages, 1024))
                }
            };

            total_count += 1;
            let (passed, latency_ms, raw, reasoning, detail, is_infra_error, ptok, ctok) = match outcome {
                Ok(o) => {
                    let expected = test.expected_result.as_deref().unwrap_or("");
                    let score = scoring::score_response(&o.content, expected, &test.scoring_method);
                    (
                        score.passed,
                        o.latency_ms as i64,
                        o.content,
                        o.reasoning_content,
                        score.detail.unwrap_or_default(),
                        false,
                        o.prompt_tokens,
                        o.completion_tokens,
                    )
                }
                // Infra failure (LM Studio rejected the request, connection
                // dropped, provider timeout) — the model never got a chance
                // to answer. This must NOT be scored as a capability FAIL;
                // is_infra_error flags it so aggregation (loot.rs) can
                // exclude it from the verdict instead of silently treating
                // "we never asked" the same as "it answered wrong." Found
                // live 2026-07-08: without this, a config bug that blocks
                // every request to a model made that model look like it
                // fails every capability, when the truth was infrastructure.
                Err(e) => (false, -1, String::new(), None, format!("execution error: {}", e), true, None, None),
            };
            if passed {
                pass_count += 1;
            }
            if is_infra_error {
                infra_error_count += 1;
            }

            sqlx::query(
                r#"INSERT INTO trial_results (run_id, trial_num, raw_response, latency_ms, passed, detail, is_infra_error, reasoning_content, test_id, prompt_tokens, completion_tokens)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
            )
            .bind(run_id)
            .bind(trial_num)
            .bind(&raw)
            .bind(latency_ms)
            .bind(passed)
            .bind(&detail)
            .bind(is_infra_error)
            .bind(&reasoning)
            .bind(test.id)
            .bind(ptok)
            .bind(ctok)
            .execute(db)
            .await?;

            // Evidence record includes the reasoning trace when present —
            // this is sealed into the run's SHA3 provenance, so a model's
            // chain-of-thought is part of the auditable evidence, not just
            // a live-only UI convenience. User request: "put them into
            // verbose mode... judge them against that too."
            evidence_lines.push(match &reasoning {
                Some(r) => format!(
                    "test={} trial={} passed={} latency_ms={} reasoning={} response={}",
                    test.name, trial_num, passed, latency_ms, r, raw
                ),
                None => format!(
                    "test={} trial={} passed={} latency_ms={} response={}",
                    test.name, trial_num, passed, latency_ms, raw
                ),
            });

            emit(tx, serde_json::json!({
                "type": "trial_result", "run_id": run_id, "test": test.name,
                "axis": test.axis,
                "trial_num": trial_num, "passed": passed, "latency_ms": latency_ms,
                "detail": detail, "reasoning_content": reasoning, "at": now_iso()
            }));
        }
    }

    // ── Verdict + provenance ───────────────────────────────────────────────
    // Infra-contaminated trials (LM Studio/provider rejected the request —
    // the model never got a chance to answer) must NOT count toward the
    // capability denominator. Found live 2026-07-08: hermes-4-14b showed
    // FAIL/UNSAFE on every core axis and looked like a genuinely terrible
    // model — every single trial had actually died to the exact
    // speculative-decoding config bug found earlier in this session (draft
    // model + batched load = LM Studio rejects the request outright). The
    // harness never reached the model once. Excluding infra trials from
    // the denominator here (not just at query time in loot.rs) means the
    // fix applies everywhere the run's totals are read, including future
    // capability-routing features — a router trained on "fails everything"
    // when the truth is "never asked" would be actively wrong, not just
    // imprecise.
    let real_total_count = total_count - infra_error_count;
    if real_total_count == 0 {
        // Every single trial was infrastructure noise — this axis was
        // never actually tested. Return an error (not a FAIL/UNSAFE
        // verdict): loot.rs only aggregates status='done' runs, so this
        // correctly disappears from the capability leaderboard instead of
        // reporting a false 100% failure.
        return Err(AppError::Executor(format!(
            "All {} trial(s) for {} on axis '{}' failed at the infrastructure level \
             (LM Studio/provider rejected every request before the model could answer) — \
             this is NOT a capability failure, the model was never actually tested. \
             Check connectivity/model load config (see LM Studio's server log) and re-run.",
            total_count, model_key, axis
        )));
    }
    if infra_error_count > 0 {
        emit(tx, serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "scoring",
            "message": format!(
                "{} of {} trials were infrastructure errors (excluded from the capability score, not counted as failures)",
                infra_error_count, total_count
            ),
            "at": now_iso()
        }));
    }
    // Shadow total_count with the corrected (infra-excluded) denominator —
    // everything downstream (verdict, pass_rate stored on test_runs,
    // evidence record) must agree on what was actually tested.
    let total_count = real_total_count;

    emit(tx, serde_json::json!({
        "type": "phase", "run_id": run_id, "phase": "scoring",
        "message": format!("Scoring: {}/{} trials passed", pass_count, total_count), "at": now_iso()
    }));

    // Verdict vocabulary lives in ONE place: models::verdict. Partial passes
    // are INTERMITTENT (IEEE reliability term) — "flaky" blames the harness,
    // and this harness is deterministic (temp 0, pinned stimuli, sealed).
    let verdict = crate::models::verdict::compute(axis, pass_count.into(), total_count.into());

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
