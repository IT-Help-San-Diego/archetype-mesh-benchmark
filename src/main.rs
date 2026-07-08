mod config;
mod error;
mod state;
mod models;
mod db;
mod routes;
mod executor;
mod lm_guard;

use config::Config;
use state::AppState;
use axum::routing::{get, post};
use axum::Router;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "archetype_mesh_dashboard=debug,tower_http=debug".into()),
        )
        .init();

    let config = Config::from_env();
    tracing::info!("Starting Archetype Mesh Dashboard on {}:{}", config.listen_addr, config.listen_port);

    let state = AppState::new(config.clone())
        .await
        .expect("Failed to initialize application state");

    // Startup reaper: runs stuck in a non-terminal state belong to a previous
    // process (crash, launchd restart, reboot mid-run) — no executor task
    // exists for them anymore, so without this they'd read "running" forever.
    // Marking them 'error' is honest: their execution genuinely did not finish.
    // ('aborted' is excluded from the reap target — it's already terminal.)
    match sqlx::query(
        "UPDATE test_runs SET status = 'error', finished_at = NOW()
         WHERE status NOT IN ('done', 'error', 'aborted')",
    )
    .execute(&state.db)
    .await
    {
        Ok(r) if r.rows_affected() > 0 => {
            tracing::warn!("Reaped {} orphaned run(s) from a previous process", r.rows_affected())
        }
        Ok(_) => {}
        Err(e) => tracing::error!("Orphan-run reaper failed: {}", e),
    }

    let static_files = ServeDir::new(&config.assets_dir);

    let app = Router::new()
        .route("/", get(routes::index::index_handler))
        .route("/api/status", get(routes::status::status_handler))
        .route("/api/summary", get(routes::summary::summary_handler))
        .route("/api/models", get(routes::models::models_handler))
        .route("/api/events", get(routes::events::sse_handler))
        .route("/api/runs", get(routes::runs::list_runs).post(routes::runs::start_runs))
        .route("/api/runs/{id}/abort", post(routes::runs::abort_run))
        .route("/api/prompt-check", get(routes::prompt_check::prompt_check).post(routes::prompt_check::prompt_check_post))
        .route("/api/loot", get(routes::loot::loot_handler))
        .route("/api/lmstudio/status", get(routes::lmstudio::lmstudio_status))
        .route("/api/lmstudio/sync", post(routes::lmstudio::lmstudio_sync))
        .route("/api/tests", get(routes::tests::list_tests).post(routes::tests::create_test))
        .route("/api/tests/{id}", axum::routing::put(routes::tests::update_test))
        .nest_service("/assets", static_files)
        // 16MB body cap: a 10MB image (Prompt Builder max) is ~13.7MB as base64.
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind_addr())
        .await
        .expect("Failed to bind listener");

    tracing::info!("Listening on {}", listener.local_addr().expect("listener has a local addr"));
    // Graceful shutdown: launchd sends SIGTERM on unload/kickstart. Draining
    // in-flight HTTP (incl. open SSE streams) instead of dropping mid-write;
    // the startup reaper covers any executor tasks cut off by the exit.
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server error");
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => tracing::info!("SIGINT received — shutting down"),
        _ = sigterm.recv() => tracing::info!("SIGTERM received — shutting down"),
    }
}
