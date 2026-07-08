//! The single choke point every LM Studio-touching request MUST go through.
//!
//! ── WHY THIS FILE EXISTS (self-harm audit, 2026-07-08) ──────────────────
//! Prior to this file, `LOCAL_RUN_LOCK` in routes::runs serialized ONLY the
//! benchmark-run path. POST /api/prompt-check and POST /api/tests (the
//! Prompt Builder, and the "live verify" checkbox) called LM Studio
//! DIRECTLY — zero lock, zero concurrency cap. A user mashing "Run Test"
//! repeatedly, a stuck browser retry loop, or literally anyone finding the
//! endpoint with curl could fire unlimited concurrent LM Studio load/chat
//! calls. On a machine that is someone's daily driver, unbounded concurrent
//! model loads is exactly the "runaway loop heats up the laptop" scenario —
//! this was the real, confirmed mechanism, not a hypothetical.
//!
//! FIX: one process-wide semaphore (permits=1, matching the old mutex's
//! all-or-nothing serialization — LM Studio can only usefully run one clean
//! room at a time on shared hardware) gates every call site. Acquiring it is
//! a queue, not a rejection: a second request waits its turn instead of
//! firing concurrently. This turns "N concurrent loads" into "N queued
//! loads, one at a time" — the difference between a laptop fan spinning up
//! predictably and a thermal event.
//!
//! ── ABORT SUPPORT ────────────────────────────────────────────────────────
//! Verified live 2026-07-08 (real process, real kill, not a code-reading
//! assumption): dropping the HTTP connection to LM Studio's
//! /api/v0/chat/completions genuinely stops GPU work — the llmworker
//! process's CPU dropped from 11.2% to 0.1% within 1 second of killing the
//! client mid-stream, and stayed at 0.0% after. LM Studio detects the
//! dropped connection and cancels inference; it does not orphan GPU work.
//! This means a real, working abort is just "cancel our own outbound
//! reqwest call" — no LM Studio-side kill API is needed.
//! `CancellationRegistry` tracks one CancellationToken per active run_id so
//! POST /api/runs/:id/abort can trigger that drop from a separate HTTP
//! request.
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore, SemaphorePermit};
use tokio_util::sync::CancellationToken;

/// Only one LOCAL LM Studio operation (run trial, prompt-check, prompt
/// builder test) may be in flight at a time — matches the machine's actual
/// hardware constraint (one GPU, one model resident at a time in the
/// clean-room model). Cloud calls never touch this.
static LM_STUDIO_GATE: Semaphore = Semaphore::const_new(1);

/// Acquire the gate. Waits in line if another local LM Studio call is in
/// flight — this is what turns "unbounded concurrent loads" into "queued,
/// one at a time," closing the self-harm gap described above.
pub async fn acquire() -> SemaphorePermit<'static> {
    LM_STUDIO_GATE
        .acquire()
        .await
        .expect("LM_STUDIO_GATE semaphore never closes — acquire() cannot fail")
}

/// Registry of cancellation handles for in-flight runs, keyed by run_id.
/// Cloned cheaply (Arc inside); one instance lives in AppState.
#[derive(Clone, Default)]
pub struct CancellationRegistry {
    inner: Arc<Mutex<HashMap<i32, CancellationToken>>>,
}

impl CancellationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a fresh token for a run and return it. Call this once at the
    /// start of execute_run_inner; the token is Ok to clone/hold for the
    /// whole run and must be unregistered on completion (any path) to avoid
    /// leaking a HashMap entry per run forever.
    pub async fn register(&self, run_id: i32) -> CancellationToken {
        let token = CancellationToken::new();
        self.inner.lock().await.insert(run_id, token.clone());
        token
    }

    /// Remove a run's token — call on every exit path (done/error/aborted)
    /// so the map doesn't grow unbounded over the life of the process.
    pub async fn unregister(&self, run_id: i32) {
        self.inner.lock().await.remove(&run_id);
    }

    /// Signal cancellation for a run. Returns true if a token was found (the
    /// run was actually in flight) — false means the run already finished
    /// or never existed, which the caller should report as a clean no-op,
    /// not an error (aborting an already-done run isn't a mistake to punish).
    pub async fn cancel(&self, run_id: i32) -> bool {
        if let Some(token) = self.inner.lock().await.get(&run_id) {
            token.cancel();
            true
        } else {
            false
        }
    }
}
