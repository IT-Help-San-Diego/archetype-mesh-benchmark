use axum::Router;
use sqlx::PgPool;

pub async fn test_app() -> Router {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| {
            "postgres://REDACTED:REDACTED@localhost:5432/archetype_mesh".to_string()
        });

    let db = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    let config = archetype_mesh_dashboard::config::Config::from_env();
    let state = archetype_mesh_dashboard::state::AppState { db, config };

    use tower_http::services::ServeDir;
    use tower_http::trace::TraceLayer;

    let static_files = ServeDir::new(state.config.assets_dir.clone());

    Router::new()
        .route("/", axum::routing::get(archetype_mesh_dashboard::routes::index::index_handler))
        .route("/api/status", axum::routing::get(archetype_mesh_dashboard::routes::status::status_handler))
        .route("/api/summary", axum::routing::get(archetype_mesh_dashboard::routes::summary::summary_handler))
        .route("/api/models", axum::routing::get(archetype_mesh_dashboard::routes::models::models_handler))
        .route("/api/events", axum::routing::get(archetype_mesh_dashboard::routes::events::sse_handler))
        .nest_service("/assets", static_files)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
