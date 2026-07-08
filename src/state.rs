use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::broadcast;

use crate::config::Config;
use crate::error::AppError;
use crate::error::AppResult;
use crate::lm_guard::CancellationRegistry;

/// Capacity of the run-event broadcast channel. Slow SSE subscribers that lag
/// more than this many events behind simply skip ahead (documented tokio behavior);
/// grid state is self-healing because periodic `refresh` snapshots follow.
const EVENT_CHANNEL_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Config,
    /// Run telemetry fan-out: executors publish serialized event JSON
    /// (run_started / phase / trial_result / verdict / run_complete / error),
    /// every open SSE connection receives it. See routes::events.
    pub events_tx: broadcast::Sender<String>,
    /// Per-run cancellation handles — lets POST /api/runs/:id/abort signal a
    /// specific in-flight run's executor task to stop. See lm_guard.rs for
    /// the full rationale (verified live: dropping the LM Studio connection
    /// genuinely halts GPU work, so cancellation here is a real abort, not
    /// a cosmetic one).
    pub cancellations: CancellationRegistry,
}

impl AppState {
    pub async fn new(config: Config) -> AppResult<Self> {
        // Retry connection for up to 60 seconds — Colima/Docker may still be booting after a reboot
        let db = Self::connect_with_retry(&config.database_url, 60).await?;

        // Run migrations on startup
        sqlx::migrate!("./migrations").run(&db).await?;

        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);

        tracing::info!("Database connected and migrations applied");
        Ok(AppState { db, config, events_tx, cancellations: CancellationRegistry::new() })
    }

    async fn connect_with_retry(url: &str, max_seconds: u64) -> AppResult<PgPool> {
        let mut elapsed = 0u64;
        loop {
            match PgPool::connect(url).await {
                Ok(pool) => return Ok(pool),
                Err(e) => {
                    if elapsed >= max_seconds {
                        tracing::error!("Failed to connect to database after {}s: {}", max_seconds, e);
                        return Err(AppError::Database(e));
                    }
                    tracing::warn!("Database not ready ({}s elapsed), retrying in 2s...", elapsed);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    elapsed += 2;
                }
            }
        }
    }
}
