use axum::Json;
use serde_json::json;

/// Liveness + identity. The version field lets the dashboard footer (and any
/// operator or script) see exactly which build is answering — deliberate
/// self-identification, NOT an update check: the instrument never phones
/// home, so "is there a newer version" stays a human question answered by
/// `brew upgrade` / the GitHub releases page.
pub async fn status_handler() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}
