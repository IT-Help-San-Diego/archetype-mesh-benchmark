use crate::error::AppError;
use crate::security::{stamp_nonce, Nonce};
use crate::state::AppState;
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, EXPIRES, PRAGMA};
use axum::response::{Html, IntoResponse};
use axum::Extension;

pub async fn index_handler(
    State(state): State<AppState>,
    Extension(nonce): Extension<Nonce>,
) -> Result<impl IntoResponse, AppError> {
    let content = tokio::fs::read_to_string(&state.config.dashboard_path)
        .await
        .map_err(|e| {
            AppError::FileNotFound(format!("{}: {}", state.config.dashboard_path.display(), e))
        })?;
    // Stamp the per-request CSP nonce onto every inline <script> so the
    // nonce-based CSP (set by the security middleware) actually permits
    // our own code without 'unsafe-inline'.
    let content = stamp_nonce(&content, &nonce.0);
    let headers = [
        (
            CACHE_CONTROL,
            "no-store, no-cache, must-revalidate, max-age=0",
        ),
        (PRAGMA, "no-cache"),
        (EXPIRES, "0"),
    ];
    Ok((headers, Html(content)))
}
