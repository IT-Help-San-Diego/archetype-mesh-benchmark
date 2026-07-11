use axum::response::Json;
use axum::extract::State;
use crate::state::AppState;
use crate::error::AppError;
use crate::db::queries;
use crate::routes::events::annotate_runnable;

pub async fn models_handler(State(state): State<AppState>) -> Result<Json<serde_json::Value>, AppError> {
    let rows = queries::fetch_unique_models(&state.db).await?;
    Ok(Json(serde_json::json!(annotate_runnable(rows))))
}
