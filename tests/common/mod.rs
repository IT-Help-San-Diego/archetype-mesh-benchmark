use axum::Router;
use sqlx::PgPool;

// Integration tests run in a fresh process that does not inherit shell dotfiles,
// so we load the project .env here before any DATABASE_URL lookup.
fn load_project_dotenv() {
    use std::{env, fs, path::PathBuf};
    let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let candidates = [
        home.join(".env"),
        env::current_dir().unwrap_or_default().join(".env"),
    ];
    for path in candidates {
        if let Ok(raw) = fs::read_to_string(path) {
            for line in raw.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some(sep) = line.find('=') {
                    let key = line[..sep].trim();
                    let val = line[sep + 1..].trim().trim_matches('"').trim_matches('\'');
                    env::set_var(key, val);
                }
            }
            break;
        }
    }
}

pub async fn test_app() -> Router {
    load_project_dotenv();
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set to run integration tests (see .env.example)");

    let db = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    let config = calibration_scope_dashboard::config::Config::from_env();
    let (events_tx, _) = tokio::sync::broadcast::channel(16);
    let state = calibration_scope_dashboard::state::AppState {
        db,
        config,
        events_tx,
        registry_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        cancellations: calibration_scope_dashboard::lm_guard::CancellationRegistry::new(),
        active_runs: calibration_scope_dashboard::gpu_telemetry::ActiveRuns::new(),
    };

    use tower_http::services::ServeDir;
    use tower_http::trace::TraceLayer;

    let static_files = ServeDir::new(state.config.assets_dir.clone());

    Router::new()
        .route(
            "/",
            axum::routing::get(calibration_scope_dashboard::routes::index::index_handler),
        )
        .route(
            "/api/status",
            axum::routing::get(calibration_scope_dashboard::routes::status::status_handler),
        )
        .route(
            "/api/summary",
            axum::routing::get(calibration_scope_dashboard::routes::summary::summary_handler),
        )
        .route(
            "/api/models",
            axum::routing::get(calibration_scope_dashboard::routes::models::models_handler),
        )
        .route(
            "/api/events",
            axum::routing::get(calibration_scope_dashboard::routes::events::sse_handler),
        )
        .route(
            "/api/runs",
            axum::routing::get(calibration_scope_dashboard::routes::runs::list_runs)
                .post(calibration_scope_dashboard::routes::runs::start_runs),
        )
        .route(
            "/api/runs/{id}",
            axum::routing::get(calibration_scope_dashboard::routes::runs::get_run_detail),
        )
        .route(
            "/api/runs/{id}/abort",
            axum::routing::post(calibration_scope_dashboard::routes::runs::abort_run),
        )
        .route(
            "/api/tests",
            axum::routing::get(calibration_scope_dashboard::routes::tests::list_tests)
                .post(calibration_scope_dashboard::routes::tests::create_test),
        )
        .route(
            "/api/tests/{id}",
            axum::routing::put(calibration_scope_dashboard::routes::tests::update_test),
        )
        .route(
            "/api/loot",
            axum::routing::get(calibration_scope_dashboard::routes::loot::loot_handler),
        )
        .route(
            "/api/router/plan",
            axum::routing::get(calibration_scope_dashboard::routes::router::router_plan),
        )
        .route(
            "/api/host/reality",
            axum::routing::get(calibration_scope_dashboard::routes::host::host_reality),
        )
        .route(
            "/api/hermes/reality",
            axum::routing::get(calibration_scope_dashboard::routes::hermes_check::hermes_reality),
        )
        .route(
            "/api/prompt-history",
            axum::routing::get(calibration_scope_dashboard::routes::prompt_check::prompt_history),
        )
        .route(
            "/api/models/{key}/dossier",
            axum::routing::get(calibration_scope_dashboard::routes::dossier::model_dossier),
        )
        .route(
            "/api/fountain",
            axum::routing::get(calibration_scope_dashboard::routes::fountain::list_probes)
                .post(calibration_scope_dashboard::routes::fountain::start_probe),
        )
        .route(
            "/api/fountain/{id}",
            axum::routing::get(calibration_scope_dashboard::routes::fountain::probe_detail),
        )
        .route(
            "/api/quarantine",
            axum::routing::get(calibration_scope_dashboard::routes::quarantine::list_quarantined),
        )
        .route(
            "/api/quarantine/{id}/release",
            axum::routing::post(
                calibration_scope_dashboard::routes::quarantine::release_quarantined,
            ),
        )
        .route(
            "/api/quarantine/{id}/notes",
            axum::routing::post(calibration_scope_dashboard::routes::quarantine::append_notes),
        )
        .nest_service("/assets", static_files)
        .layer(axum::middleware::from_fn(
            calibration_scope_dashboard::security::security_headers,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
