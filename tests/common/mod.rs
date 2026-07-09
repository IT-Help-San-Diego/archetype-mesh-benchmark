use axum::Router;
use sqlx::PgPool;

pub async fn test_app() -> Router {
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set to run integration tests (see .env.example)");

    let db = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    let config = archetype_mesh_dashboard::config::Config::from_env();
    let (events_tx, _) = tokio::sync::broadcast::channel(16);
    let state = archetype_mesh_dashboard::state::AppState {
        db,
        config,
        events_tx,
        cancellations: archetype_mesh_dashboard::lm_guard::CancellationRegistry::new(),
    };

    use tower_http::services::ServeDir;
    use tower_http::trace::TraceLayer;

    let static_files = ServeDir::new(state.config.assets_dir.clone());

    Router::new()
        .route("/", axum::routing::get(archetype_mesh_dashboard::routes::index::index_handler))
        .route("/api/status", axum::routing::get(archetype_mesh_dashboard::routes::status::status_handler))
        .route("/api/summary", axum::routing::get(archetype_mesh_dashboard::routes::summary::summary_handler))
        .route("/api/models", axum::routing::get(archetype_mesh_dashboard::routes::models::models_handler))
        .route("/api/events", axum::routing::get(archetype_mesh_dashboard::routes::events::sse_handler))
        .route(
            "/api/runs",
            axum::routing::get(archetype_mesh_dashboard::routes::runs::list_runs)
                .post(archetype_mesh_dashboard::routes::runs::start_runs),
        )
        .route(
            "/api/runs/{id}",
            axum::routing::get(archetype_mesh_dashboard::routes::runs::get_run_detail),
        )
        .route(
            "/api/runs/{id}/abort",
            axum::routing::post(archetype_mesh_dashboard::routes::runs::abort_run),
        )
        .route(
            "/api/tests",
            axum::routing::get(archetype_mesh_dashboard::routes::tests::list_tests)
                .post(archetype_mesh_dashboard::routes::tests::create_test),
        )
        .route(
            "/api/tests/{id}",
            axum::routing::put(archetype_mesh_dashboard::routes::tests::update_test),
        )
        .route("/api/loot", axum::routing::get(archetype_mesh_dashboard::routes::loot::loot_handler))
        .route(
            "/api/router/plan",
            axum::routing::get(archetype_mesh_dashboard::routes::router::router_plan),
        )
        .route(
            "/api/host/reality",
            axum::routing::get(archetype_mesh_dashboard::routes::host::host_reality),
        )
        .route(
            "/api/hermes/reality",
            axum::routing::get(archetype_mesh_dashboard::routes::hermes_check::hermes_reality),
        )
        .route(
            "/api/prompt-history",
            axum::routing::get(archetype_mesh_dashboard::routes::prompt_check::prompt_history),
        )
        .nest_service("/assets", static_files)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
