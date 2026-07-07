mod config;
mod error;
mod state;
mod models;
mod db;
mod routes;
mod executor;

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

    let static_files = ServeDir::new(&config.assets_dir);

    let app = Router::new()
        .route("/", get(routes::index::index_handler))
        .route("/api/status", get(routes::status::status_handler))
        .route("/api/summary", get(routes::summary::summary_handler))
        .route("/api/models", get(routes::models::models_handler))
        .route("/api/events", get(routes::events::sse_handler))
        .route("/api/runs", get(routes::runs::list_runs).post(routes::runs::start_runs))
        .route("/api/prompt-check", get(routes::prompt_check::prompt_check))
        .nest_service("/assets", static_files)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind_addr())
        .await
        .expect("Failed to bind listener");

    tracing::info!("Listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app)
        .await
        .expect("Server error");
}
