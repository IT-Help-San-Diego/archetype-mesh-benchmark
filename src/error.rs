use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP client error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Executor error: {0}")]
    Executor(String),

    #[error("Run aborted by operator request")]
    Aborted,
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            AppError::Database(e) => {
                tracing::error!("Database error: {}", e);
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Database error")
            }
            AppError::Migration(e) => {
                tracing::error!("Migration error: {}", e);
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Migration error")
            }
            AppError::FileNotFound(_path) => {
                (axum::http::StatusCode::NOT_FOUND, "File not found")
            }
            AppError::Io(e) => {
                tracing::error!("IO error: {}", e);
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "IO error")
            }
            AppError::Json(e) => {
                tracing::error!("JSON error: {}", e);
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "JSON error")
            }
            AppError::Http(e) => {
                tracing::error!("HTTP client error: {}", e);
                (axum::http::StatusCode::BAD_GATEWAY, "Upstream HTTP error")
            }
            AppError::Executor(msg) => {
                tracing::error!("Executor error: {}", msg);
                // Executor errors are overwhelmingly caller-fixable (invalid
                // axis, unknown model, duplicate in-flight run, budget refusal)
                // — surface the actual message with a 400 so the client can
                // act on it, instead of a swallowed generic 500. Messages are
                // built server-side from validated data; nothing sensitive.
                return (axum::http::StatusCode::BAD_REQUEST, msg.clone()).into_response();
            }
            AppError::Aborted => {
                (axum::http::StatusCode::OK, "Run aborted by operator request")
            }
        };

        (status, message).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
