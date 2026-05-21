use crate::{app::AppState, error::ApiError};
use axum::response::Response;

pub async fn device_ws(_state: AppState) -> Result<Response, ApiError> {
    Err(ApiError::invalid_args("device websocket not implemented"))
}
