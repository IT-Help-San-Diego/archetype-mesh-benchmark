//! Test-run executor: the scientific core.
//!
//! One run = one (model, axis) execution of every active test on that axis.
//! Pipeline (each phase streamed live over the SSE broadcast — no spinners,
//! real telemetry): clean-room prep (local) → prompt assembly (server-side,
//! ground truth never sent to the model) → N trials → objective scoring →
//! verdict → SHA3-512 provenance → persist.
pub mod cloud;
pub mod lmstudio;
pub mod provenance;
pub mod scoring;

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
    pub speculative_decode: Option<SpeculativeDecodeStats>,
}

#[derive(Debug)]
pub struct SpeculativeDecodeStats {
    pub draft_model: Option<String>,
    pub total_draft_tokens_count: Option<i64>,
    pub accepted_draft_tokens_count: Option<i64>,
    pub rejected_draft_tokens_count: Option<i64>,
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
                  expected_result, scoring_method, trials_per_run, formal_spec, fallacy_tag, owl_type
           FROM tests WHERE active = true AND axis = $1 ORDER BY id"#,
    )
    .bind(axis)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// Load an explicit set of tests by ID (modular segments). We avoid a
/// runtime-built IN(...) query (sqlx 0.9 rejects non-literal query strings)
/// by loading all active tests with a static query and filtering in Rust.
/// The ID set is small (a modular slice) and already validated upstream, so
/// the extra in-memory filter is negligible and keeps the SQL path auditable.
pub async fn tests_for_ids(db: &PgPool, ids: &[i32]) -> AppResult<Vec<TestDef>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let all: Vec<TestDef> = sqlx::query_as::<_, TestDef>(
        r#"SELECT id, name, axis, prompt_text, attachment_path, attachment_sha3,
                  expected_result, scoring_method, trials_per_run, formal_spec, fallacy_tag, owl_type
           FROM tests WHERE active = true ORDER BY id"#,
    )
    .fetch_all(db)
    .await?;
    let _want: std::collections::HashSet<i32> = ids.iter().copied().collect();
    // Preserve the caller's selection order (top-to-bottom as chosen).
    let mut ordered: Vec<TestDef> = Vec::new();
    let mut seen: std::collections::HashSet<i32> = std::collections::HashSet::new();
    for id in ids {
        if let Some(t) = all.iter().find(|t| t.id == *id) {
            if seen.insert(t.id) {
                ordered.push(t.clone());
            }
        }
    }
    Ok(ordered)
}

/// Build the OpenAI-shaped user message for a test.
/// Anti-cheat invariants enforced here:
///   1. expected_result is NEVER part of the payload.
///   2. If the test pins an attachment hash, the actual bytes on disk are
///      re-hashed and MUST match before anything is sent.
fn build_messages(
    test: &TestDef,
    project_root: &std::path::Path,
    scaffold_supplement: Option<&str>,
) -> AppResult<Vec<serde_json::Value>> {
    let mut messages: Vec<serde_json::Value> = Vec::new();

    // Scaffold system prompt: the operator's general reasoning-guidance prompt.
    //
    // ANTI-CHEAT (I1): we DELIBERATELY do NOT append test.formal_spec here. The
    // formal spec IS the ground-truth answer for these tests — a reasoning
    // spec's ⊢/⊬ turnstile literally states VALID/INVALID, a vision spec names
    // the exact expected string ("menubar.app = Obsidian"), and a security spec
    // states the required refusal policy. Appending it handed the model an
    // answer key and confounded the entire scaffold-vs-cleanroom experiment
    // (baseline answered "OmniFocus"; scaffolded, with the spec, answered
    // "Obsidian"). Scaffolded mode now measures ONLY the effect of the
    // operator's general system prompt — never a per-test answer leak. If a
    // leak-free structural hint is ever wanted, it must be a separately stored,
    // answer-stripped field that passes the create_test leakage guard.
    if let Some(scaffold) = scaffold_supplement {
        if !scaffold.is_empty() {
            messages.push(serde_json::json!({
                "role": "system",
                "content": scaffold,
            }));
        }
    }

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
            messages.push(serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": test.prompt_text},
                    {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", b64)}}
                ]
            }));
            Ok(messages)
        }
        None => {
            messages.push(serde_json::json!({
                "role": "user",
                "content": test.prompt_text
            }));
            Ok(messages)
        }
    }
}

/// Regenerate SHA3 provenance for a completed/failed run from persisted
/// `trial_results`. This is used when the inner run fails after partial
/// trials have already been written, so the audit ledger can still seal
/// exactly what happened instead of leaving `sha3_provenance = NULL`.
/// Regenerate SHA3 provenance for a completed/failed run from persisted
/// `trial_results`. This is used when the inner run fails after partial
/// completion — the error handler recomputes provenance from whatever
/// trials did complete so partial evidence is still sealed.
/// Currently unused — kept for future partial-run recovery scenarios.
#[allow(dead_code)]
async fn recompute_run_sha3(
    db: &PgPool,
    run_id: i32,
    model_key: &str,
    axis: &str,
) -> Option<String> {
    let rows = sqlx::query_as::<_, (String,)>(
        r#"SELECT COALESCE(reasoning_content, '') || ' ' || COALESCE(raw_response, '')
           FROM trial_results
           WHERE run_id = $1
           ORDER BY test_id, trial_num"#,
    )
    .bind(run_id)
    .fetch_all(db)
    .await
    .ok()?;

    let mut evidence_lines: Vec<String> = Vec::new();
    let mut pass_count: i32 = 0;
    let mut total_count: i32 = 0;
    for (idx, (payload,)) in rows.iter().enumerate() {
        total_count += 1;
        if payload.contains("infrastructure error") {
            continue;
        }
        if !payload.trim().is_empty() {
            pass_count += 1;
        }
        evidence_lines.push(format!("trial={} response={}", idx + 1, payload));
    }
    let real_total_count = total_count;
    let evidence_record = format!(
        "run_id={} model={} axis={} pass={}/{}\n{}",
        run_id,
        model_key,
        axis,
        pass_count,
        real_total_count,
        evidence_lines.join("\n")
    );
    Some(provenance::sha3_hex(&evidence_record))
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
    load_mode: crate::routes::runs::LoadMode,
    draft_model_key: Option<String>,
    scaffold_supplement: Option<String>,
    test_ids: Option<Vec<i32>>,
    load_preset: Option<String>,
) {
    let cancel_token = cancellations.register(run_id).await;

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(RUN_BUDGET_SECS),
        execute_run_inner(
            &db,
            &config,
            &tx,
            &cancel_token,
            run_id,
            model_id,
            &model_key,
            &location,
            &provider,
            &axis,
            load_mode,
            draft_model_key,
            scaffold_supplement,
            test_ids,
            load_preset,
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
            let sha3: Option<(Option<String>,)> = sqlx::query_as::<_, (Option<String>,)>(
                "SELECT sha3_provenance FROM test_runs WHERE id = $1",
            )
            .bind(run_id)
            .fetch_optional(&db)
            .await
            .ok()
            .flatten();
            let sha3 = sha3.and_then(|t| t.0);
            // Always mark the run as terminal on error so it cannot hang in
            // `running` and block re-runs (the re-run guard only exempts
            // 'done'/'error'/'aborted'). Previously this used a CASE that
            // could leave the run as 'completed_with_errors' (also blocking
            // re-runs) or — if the UPDATE itself failed — as 'running'
            // forever. Set 'error' unconditionally and surface any DB error.
            if let Err(db_err) = sqlx::query(
                r#"UPDATE test_runs
                   SET status = 'error', finished_at = NOW(),
                       sha3_provenance = COALESCE(sha3_provenance, $2)
                   WHERE id = $1"#,
            )
            .bind(run_id)
            .bind(&sha3)
            .execute(&db)
            .await
            {
                tracing::error!("Failed to mark run {} as error in DB: {}", run_id, db_err);
            }
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
const RUN_BUDGET_SECS: u64 = 3600; // 60 minutes — raised from 30 when reasoning battery expanded from 60→90 tests

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
    load_mode: crate::routes::runs::LoadMode,
    draft_model_key: Option<String>,
    scaffold_supplement: Option<String>,
    test_ids: Option<Vec<i32>>,
    load_preset: Option<String>,
) -> AppResult<()> {
    let client = reqwest::Client::new();

    sqlx::query("UPDATE test_runs SET status = 'loading', started_at = NOW() WHERE id = $1")
        .bind(run_id)
        .execute(db)
        .await?;
    emit(
        tx,
        serde_json::json!({
            "type": "run_started", "run_id": run_id, "model_key": model_key,
            "axis": axis, "location": location, "at": now_iso()
        }),
    );

    // ── Trials ─────────────────────────────────────────────────────────────
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
        client: &reqwest::Client,
        base_url: &str,
    ) -> AppResult<()> {
        // PATIENCE PRINCIPLE: if the model is already resident, the RAM is
        // already committed — re-loading won't consume more, so skip the
        // guard entirely. This lets a user who pre-loaded a large model
        // (e.g. a 120B on a slow machine) benchmark it without the guard
        // refusing based on "available" memory that's already spoken for.
        if let Ok(models) = lmstudio::list_ls_models(client, base_url).await {
            if models
                .iter()
                .any(|m| m.id == model_key && m.load_state == "loaded")
            {
                emit(
                    tx,
                    serde_json::json!({
                        "type": "phase", "run_id": run_id, "phase": "memory_check",
                        "message": format!("Memory check: {} already resident — guard skipped", model_key),
                        "at": now_iso()
                    }),
                );
                return Ok(());
            }
        }

        // Get the model's size from the DB (size_gb, optional — some models lack it).
        // size_gb is NULL for models we synced from LM Studio (which does not
        // report size) or synced before a download captured it. sqlx decodes a
        // NULL scalar as the inner type, so we must declare the column nullable
        // (Option<f64>) and flatten the double-Option. Without this, every model
        // with size_gb IS NULL crashes the run at the memory check with
        // "decoding column 0: unexpected null" — a silent benchmark break for
        // most locally-synced models.
        let model_size_gb: Option<f64> =
            sqlx::query_scalar::<_, Option<f64>>("SELECT size_gb FROM models WHERE id = $1")
                .bind(model_id)
                .fetch_optional(db)
                .await?
                .flatten();

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

            emit(
                tx,
                serde_json::json!({
                    "type": "phase", "run_id": run_id, "phase": "memory_check",
                    "message": format!(
                        "Memory check: {} needs ~{:.1} GB (model {:.1} + 25% spec-decode overhead + 8 GB safety), available: {:.1} GB",
                        model_key, needed_gb, model_gb, available_gb
                    ),
                    "at": now_iso()
                }),
            );

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
            emit(
                tx,
                serde_json::json!({
                    "type": "phase", "run_id": run_id, "phase": "memory_check",
                    "message": format!("Memory check: {:.1} GB available (model size unknown, using minimum guard)", available_gb),
                    "at": now_iso()
                }),
            );
        }

        Ok(())
    }

    // ── Local-model prep ────────────────────────────────────────────────
    if location == "local" {
        match load_mode {
            crate::routes::runs::LoadMode::CleanRoom
            | crate::routes::runs::LoadMode::Scaffolded => {
                emit(
                    tx,
                    serde_json::json!({
                        "type": "phase", "run_id": run_id, "phase": "ejecting",
                        "model_key": model_key,
                        "message": "Clean room: ejecting all loaded models from LM Studio", "at": now_iso()
                    }),
                );
                // PATIENCE PRINCIPLE: if the target model is already resident,
                // do NOT eject it — tearing down a working 120B just to
                // immediately re-load it thrashes RAM and can abort the engine
                // on slow / memory-constrained hardware. Eject only the
                // *other* instances, keeping the target in place.
                let target_resident = {
                    if let Ok(models) =
                        lmstudio::list_ls_models(&client, &config.lmstudio_base_url).await
                    {
                        models.iter().any(|m| {
                            m.id == model_key
                                && (m.load_state == "loaded" || !m.loaded_instances.is_empty())
                        })
                    } else {
                        false
                    }
                };
                let ejected = if target_resident {
                    emit(
                        tx,
                        serde_json::json!({
                            "type": "phase", "run_id": run_id, "phase": "ejected",
                            "model_key": model_key,
                            "message": format!("{} already resident — skipping eject (patience principle)", model_key), "at": now_iso()
                        }),
                    );
                    lmstudio::eject_others(&client, &config.lmstudio_base_url, model_key).await?
                } else {
                    lmstudio::eject_all(&client, &config.lmstudio_base_url).await?
                };
                emit(
                    tx,
                    serde_json::json!({
                        "type": "phase", "run_id": run_id, "phase": "ejected",
                        "model_key": model_key,
                        "message": format!("Ejected {} instance(s): {:?}", ejected.len(), ejected), "at": now_iso()
                    }),
                );

                check_memory_safety(
                    db,
                    tx,
                    run_id,
                    model_id,
                    model_key,
                    &client,
                    &config.lmstudio_base_url,
                )
                .await?;

                emit(
                    tx,
                    serde_json::json!({
                        "type": "phase", "run_id": run_id, "phase": "loading",
                        "model_key": model_key,
                        "message": format!("Loading {} — watch LM Studio's server tab", model_key), "at": now_iso()
                    }),
                );
                let load_start = std::time::Instant::now();
                // Resolve the engine load preset the run was requested under.
                // `load_preset` (e.g. "lightweight") selects the LM Studio
                // load config; a draft model (when provided) enables
                // speculative decoding on top of that preset.
                let preset_name = load_preset.clone().unwrap_or_else(|| "performance".into());
                let preset = crate::routes::runs::load_preset_by_name(&preset_name);
                let resident = cancellable!(lmstudio::ensure_loaded(
                    &client,
                    &config.lmstudio_base_url,
                    model_key,
                    &preset,
                    draft_model_key.as_deref(),
                    300
                ))?;
                if !resident {
                    return Err(AppError::Executor(format!(
                        "{} did not become resident within 300s",
                        model_key
                    )));
                }
                emit(
                    tx,
                    serde_json::json!({
                        "type": "phase", "run_id": run_id, "phase": "resident",
                        "model_key": model_key,
                        "message": format!("{} verified resident in RAM ({}s load)", model_key, load_start.elapsed().as_secs()),
                        "at": now_iso()
                    }),
                );
                // Persist the exact engine load config this run measured under,
                // so the result is reproducible + the UI can show what tuning
                // was applied. This is the "what we have control over" record.
                let runtime_cfg = preset.to_load_json(model_key, draft_model_key.as_deref());
                sqlx::query("UPDATE test_runs SET lmstudio_runtime_config = $1 WHERE id = $2")
                    .bind(runtime_cfg)
                    .bind(run_id)
                    .execute(db)
                    .await?;
            }
            crate::routes::runs::LoadMode::SpeculativePair => {
                let draft_key = draft_model_key.as_ref().ok_or_else(|| {
                    AppError::Executor("speculative-pair mode requires draft_model_key".into())
                })?;

                emit(
                    tx,
                    serde_json::json!({
                        "type": "phase", "run_id": run_id, "phase": "pair_loading",
                        "model_key": model_key,
                        "draft_key": draft_key,
                        "message": format!("Speculative pair: loading {} + {}", model_key, draft_key), "at": now_iso()
                    }),
                );

                let draft_model_id = sqlx::query_scalar::<_, Option<i32>>(
                    "SELECT id FROM models WHERE key = $1 AND active = true",
                )
                .bind(draft_key)
                .fetch_optional(db)
                .await?
                .ok_or_else(|| {
                    AppError::Executor(format!("Unknown draft model key: {}", draft_key))
                })?;

                check_memory_safety(
                    db,
                    tx,
                    run_id,
                    model_id,
                    model_key,
                    &client,
                    &config.lmstudio_base_url,
                )
                .await?;
                if let Some(draft_id) = draft_model_id {
                    check_memory_safety(
                        db,
                        tx,
                        run_id,
                        draft_id,
                        draft_key,
                        &client,
                        &config.lmstudio_base_url,
                    )
                    .await?;
                }

                let pair_load_start = std::time::Instant::now();
                progress(run_id, "pair_load_start");
                let (_primary_inst, _draft_inst) = cancellable!(lmstudio::ensure_pair_loaded(
                    &client,
                    &config.lmstudio_base_url,
                    model_key,
                    draft_key,
                    300
                ))?;
                progress(run_id, "pair_load_done");

                let lmstudio_config_json =
                    lmstudio::fetch_instance_config(&client, &config.lmstudio_base_url, model_key)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|v| serde_json::to_string(&v).ok());

                let _ = sqlx::query(
                    "UPDATE test_runs SET draft_model_key = $1, lmstudio_runtime_config = COALESCE($2, lmstudio_runtime_config) WHERE id = $3"
                )
                .bind(draft_key)
                .bind(lmstudio_config_json)
                .bind(run_id)
                .execute(db)
                .await;

                emit(
                    tx,
                    serde_json::json!({
                        "type": "phase", "run_id": run_id, "phase": "pair_resident",
                        "message": format!("Pair verified resident in RAM ({}s load): {} + {}",
                            pair_load_start.elapsed().as_secs(), model_key, draft_key),
                        "at": now_iso()
                    }),
                );
                progress(run_id, "pair_resident_emitted");
            }
        }
    }

    // ── Progress log helper ──────────────────────────────────────────────
    fn progress(run_id: i32, msg: &str) {
        let line = format!(
            "{} run={} {}\n",
            chrono::Utc::now().to_rfc3339(),
            run_id,
            msg
        );
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/progress.log")
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    }

    // ── Trials ─────────────────────────────────────────────────────────────
    // Modular segments: a 'custom' run loads exactly the requested test IDs;
    // a whole-axis run loads by axis. Same trial loop either way — the only
    // difference is which tests are in the set.
    let tests = match &test_ids {
        Some(ids) => tests_for_ids(db, ids).await?,
        None => tests_for_axis(db, axis).await?,
    };
    if tests.is_empty() {
        return Err(AppError::Executor(format!(
            "No active tests for {}",
            if test_ids.is_some() {
                "the requested test_ids set".to_string()
            } else {
                format!("axis '{}'", axis)
            }
        )));
    }

    // Pre-flight capability gate: refuse a vision test against a model LM
    // Studio's own metadata says has no vision support, BEFORE spending a
    // clean-room eject+load cycle and a real GPU inference attempt. For a
    // whole-axis 'vision' run this is axis==vision; for a modular 'custom'
    // run it fires if ANY requested test is a vision test. We determine
    // vision membership from the already-loaded test set (covers both the
    // axis and custom paths uniformly).
    let run_has_vision = tests.iter().any(|t| t.axis == "vision");
    if run_has_vision {
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

    // Emit a run_plan with the total trial count for THIS axis-execution so
    // the dashboard can show real progress ("trial 14 of 29") instead of a
    // spinner. One run_id may span multiple axis-executions; each emits its
    // own plan with its own total, and the frontend tracks per-axis progress.
    let total_trials: i32 = tests
        .iter()
        .map(|t| t.trials_per_run.unwrap_or(3).max(1))
        .sum();
    emit(
        tx,
        serde_json::json!({
            "type": "run_plan", "run_id": run_id, "axis": axis,
            "total_tests": tests.len() as i32,
            "total_trials": total_trials, "at": now_iso()
        }),
    );

    sqlx::query("UPDATE test_runs SET status = 'running' WHERE id = $1")
        .bind(run_id)
        .execute(db)
        .await?;
    progress(run_id, "tests_loaded");

    let mut pass_count: i32 = 0;
    let mut total_count: i32 = 0;
    let mut infra_error_count: i32 = 0;
    let mut evidence_lines: Vec<String> = Vec::new();
    let mut completed_trials: i32 = 0;

    for test in &tests {
        let n_trials = test.trials_per_run.unwrap_or(3).max(1);
        emit(
            tx,
            serde_json::json!({
                "type": "phase", "run_id": run_id, "phase": "trial",
                "message": format!("Test '{}' — {} trial(s)", test.name, n_trials), "at": now_iso()
            }),
        );

        let messages = build_messages(test, &config.project_root, scaffold_supplement.as_deref())?;

        for trial_num in 1..=n_trials {
            // Also checked here (not just inside cancellable! around the
            // network call) so a cancel between trials doesn't have to wait
            // for one more full trial to start before taking effect.
            if cancel_token.is_cancelled() {
                return Err(AppError::Aborted);
            }
            // Emit trial_start so the brain visualization lights up the
            // corresponding region for the ENTIRE trial duration — not just
            // a blip at the end. The dashboard uses this to start a
            // sustained glow that stays lit until trial_result arrives.
            emit(
                tx,
                serde_json::json!({
                    "type": "trial_start", "run_id": run_id, "test": test.name,
                    "axis": test.axis, "trial_num": trial_num,
                    "formal_spec": test.formal_spec,
                    "test_name": test.name,
                    "at": now_iso()
                }),
            );
            let outcome = match location {
                "local" => {
                    cancellable!(lmstudio::chat(
                        &client,
                        &config.lmstudio_base_url,
                        model_key,
                        &messages,
                        4096,
                        0.0
                    ))
                }
                _ => {
                    let config_key = match provider {
                        "nous" => &config.nous_api_key,
                        "openrouter" => &config.openrouter_api_key,
                        "openai" => &config.openai_api_key,
                        "gemini" => &config.gemini_api_key,
                        other => {
                            return Err(AppError::Executor(format!("Unknown provider: {}", other)))
                        }
                    };
                    // Resolved per run (not at process start): Nous OAuth agent
                    // keys rotate on the order of hours.
                    let key = cloud::resolve_api_key(provider, config_key)?;
                    if provider == "gemini" {
                        cancellable!(cloud::gemini_chat(
                            &client, &key, model_key, &messages, 1024
                        ))
                    } else {
                        cancellable!(cloud::chat(
                            &client, provider, &key, model_key, &messages, 1024
                        ))
                    }
                }
            };

            total_count += 1;
            let (
                passed,
                latency_ms,
                raw,
                reasoning,
                mut detail,
                is_infra_error,
                ptok,
                ctok,
                spec_decode,
            ) = match outcome {
                Ok(o) => {
                    let expected = test.expected_result.as_deref().unwrap_or("");
                    match scoring::score_response(&o.content, expected, &test.scoring_method) {
                        Ok(score) => (
                            score.passed,
                            o.latency_ms as i64,
                            o.content,
                            o.reasoning_content,
                            score.detail.unwrap_or_default(),
                            false,
                            o.prompt_tokens,
                            o.completion_tokens,
                            o.speculative_decode,
                        ),
                        // Unknown scoring_method: the model DID answer — keep
                        // its response, latency and tokens as evidence — but
                        // the trial cannot be graded. Record as infra/config
                        // error so it never reads as a capability failure.
                        Err(e) => (
                            false,
                            o.latency_ms as i64,
                            o.content,
                            o.reasoning_content,
                            e,
                            true,
                            o.prompt_tokens,
                            o.completion_tokens,
                            o.speculative_decode,
                        ),
                    }
                }
                Err(e) => (
                    false,
                    -1,
                    String::new(),
                    None,
                    format!("execution error: {}", e),
                    true,
                    None,
                    None,
                    None,
                ),
            };
            let mut is_infra_error = is_infra_error;
            if !is_infra_error && raw.trim().is_empty() {
                is_infra_error = true;
            }
            if !is_infra_error && latency_ms == -1 && raw.is_empty() {
                is_infra_error = true;
            }
            if is_infra_error {
                detail = format!("infrastructure error: {}", detail);
            }
            if passed {
                pass_count += 1;
            }
            if is_infra_error {
                infra_error_count += 1;
            }

            let (trial_result_id,): (i32,) = sqlx::query_as(
                r#"INSERT INTO trial_results (run_id, trial_num, raw_response, latency_ms, passed, detail, is_infra_error, reasoning_content, test_id, prompt_tokens, completion_tokens, speculative_draft_model, total_draft_tokens_count, accepted_draft_tokens_count, rejected_draft_tokens_count)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) RETURNING id"#,
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
            .bind(spec_decode.as_ref().and_then(|s| s.draft_model.clone()))
            .bind(spec_decode.as_ref().and_then(|s| s.total_draft_tokens_count))
            .bind(spec_decode.as_ref().and_then(|s| s.accepted_draft_tokens_count))
            .bind(spec_decode.as_ref().and_then(|s| s.rejected_draft_tokens_count))
            .fetch_one(db)
            .await?;

            // Owl Semaphore σₕ (metacognitive) pass — score the explanation
            // the model ALREADY gave on this trial and persist it beside the
            // trial row. Deterministic keyword check, never a second model
            // grading the first (migration 036). One row per trial; rows
            // where nothing could be checked carry honest NULLs + notes.
            let meta = scoring::score_metacognition(reasoning.as_deref(), &test.name);
            sqlx::query(
                r#"INSERT INTO metacognitive_scores (trial_result_id, cites_correct_rule, acknowledges_uncertainty, explains_distractor, rubric_notes)
                   VALUES ($1, $2, $3, $4, $5)
                   ON CONFLICT (trial_result_id) DO NOTHING"#,
            )
            .bind(trial_result_id)
            .bind(meta.cites_correct_rule)
            .bind(meta.acknowledges_uncertainty)
            .bind(meta.explains_distractor)
            .bind(&meta.rubric_notes)
            .execute(db)
            .await?;

            // Evidence record includes the reasoning trace when present —
            // this is sealed into the run's SHA3 provenance, so a model's
            // chain-of-thought is part of the auditable evidence, not just
            // a live-only UI convenience. User request: "put them into
            // verbose mode... judge them against that too."
            if let Some(sd) = &spec_decode {
                evidence_lines.push(format!(
                    "speculative_decode draft={} total={} accepted={} rejected={}",
                    sd.draft_model.as_deref().unwrap_or("?"),
                    sd.total_draft_tokens_count.unwrap_or(-1),
                    sd.accepted_draft_tokens_count.unwrap_or(-1),
                    sd.rejected_draft_tokens_count.unwrap_or(-1),
                ));
            }
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

            // NO mid-run circuit breaker — by design, after three live
            // miscalibrations (runs 904, 910, 911). Every heuristic tried
            // ("N consecutive infra", "majority infra", "infra with zero
            // scored") false-aborted a healthy run, because a gap/completion
            // run can legitimately OPEN with tests a model deterministically
            // nulls on (Fable 5: content=null on safety-classifier prompts)
            // — indistinguishable mid-run from a dead model. The bounds that
            // actually protect the machine are structural: the 90s per-trial
            // timeout (a dead model fails fast per trial, no more 300s
            // hangs) and the wall-clock run budget. A truly dead model now
            // produces an honest all-infra 0-scored verdict within budget
            // instead of a guessed abort.

            completed_trials += 1;
            emit(
                tx,
                serde_json::json!({
                    "type": "trial_result", "run_id": run_id, "test": test.name,
                    "axis": test.axis,
                    "trial_num": trial_num, "passed": passed, "latency_ms": latency_ms,
                    "detail": detail, "reasoning_content": reasoning,
                    "owl_cites_rule": meta.cites_correct_rule, "at": now_iso(),
                    "owl_type": test.owl_type,
                    "completed_trials": completed_trials
                }),
            );
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
        emit(
            tx,
            serde_json::json!({
                "type": "phase", "run_id": run_id, "phase": "scoring",
                "message": format!(
                    "{} of {} trials were infrastructure errors (excluded from the capability score, not counted as failures)",
                    infra_error_count, total_count
                ),
                "at": now_iso()
            }),
        );
    }
    // Shadow total_count with the corrected (infra-excluded) denominator —
    // everything downstream (verdict, pass_rate stored on test_runs,
    // evidence record) must agree on what was actually tested.
    let total_count = real_total_count;

    progress(
        run_id,
        &format!("scoring_start pass={} total={}", pass_count, total_count),
    );
    emit(
        tx,
        serde_json::json!({
            "type": "phase", "run_id": run_id, "phase": "scoring",
            "message": format!("Scoring: {}/{} trials passed", pass_count, total_count), "at": now_iso()
        }),
    );
    progress(run_id, "scoring_emitted");

    // Verdict vocabulary lives in ONE place: models::verdict. Partial passes
    // are INTERMITTENT (IEEE reliability term) — "flaky" blames the harness,
    // and this harness is deterministic (temp 0, pinned stimuli, sealed).
    let verdict = crate::models::verdict::compute(axis, pass_count.into(), total_count.into());

    let evidence_record = format!(
        "run_id={} model={} axis={} pass={}/{}\n{}",
        run_id,
        model_key,
        axis,
        pass_count,
        total_count,
        evidence_lines.join("\n")
    );
    let sha3 = provenance::sha3_hex(&evidence_record);

    // Auto-quarantine ONLY runs whose evidence is untrustworthy — i.e.
    // infrastructure noise DOMINATED the run (more trials died before the model
    // could answer than actually survived to be scored). Infra trials are
    // already excluded from the denominator above, so a run with substantial
    // clean evidence plus a few infra blips is a VALID measurement and MUST stay
    // on the leaderboard. (Previously ANY single infra trial quarantined the
    // whole run: a perfect 78/78 reasoning run vanished from every aggregate for
    // one rejected request — the fleet's best evidence made invisible.)
    //
    // A run that genuinely answered its trials and passed none is an HONEST
    // failure verdict (INTERMITTENT/FAIL/UNSAFE) — it is NEVER quarantined;
    // hiding a real bad result is dishonesty in the opposite direction. The old
    // `blank_responses` branch was dead (blank trials are reclassified as infra
    // upstream) and the `all_failed` branch suppressed exactly those honest
    // failures. Trial-granular contamination is the deeper fix (see the
    // quarantine-redesign refactor); this run-granular rule stops the bleeding.
    //
    // `total_count` here is the infra-excluded real count (shadowed above), so
    // `infra_error_count > total_count` means "more attempts died than survived".
    let quarantine_reason = if infra_error_count > total_count {
        Some("infrastructure_error")
    } else {
        None
    };

    sqlx::query(
        r#"UPDATE test_runs
           SET status = 'done', finished_at = NOW(),
               pass_count = $2, total_count = $3, sha3_provenance = $4,
               quarantined = COALESCE($5, FALSE),
               quarantine_reason = $6
           WHERE id = $1"#,
    )
    .bind(run_id)
    .bind(pass_count)
    .bind(total_count)
    .bind(&sha3)
    .bind(quarantine_reason.is_some())
    .bind(quarantine_reason)
    .execute(db)
    .await?;

    emit(
        tx,
        serde_json::json!({
            "type": "verdict", "run_id": run_id, "overall": verdict,
            "pass_count": pass_count, "total_count": total_count, "at": now_iso()
        }),
    );
    emit(
        tx,
        serde_json::json!({
            "type": "run_complete", "run_id": run_id, "overall": verdict,
            "sha3": sha3, "at": now_iso()
        }),
    );

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
        .ok_or_else(|| {
            AppError::Executor("LM Studio response had no usage.prompt_tokens".to_string())
        })?;

    let fits = exact <= context_limit;
    let pct = if context_limit > 0 {
        (exact as f64 / context_limit as f64 * 100.0).round() as i64
    } else {
        0
    };
    let note = format!(
        "{} tokens EXACT (live LM Studio count) / {} ctx window ({}%) — {}",
        exact,
        context_limit,
        pct,
        if fits { "FITS" } else { "OVERFLOW" }
    );
    Ok((exact, context_limit, fits, note))
}
