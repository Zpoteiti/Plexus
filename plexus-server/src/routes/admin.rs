use crate::{app::AppState, auth::AdminUser, db::system_config, error::ApiError};
use axum::{Json, extract::State};
use serde_json::Value;
use std::collections::BTreeMap;

pub async fn get_config(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<BTreeMap<String, Value>>, ApiError> {
    let values = system_config::get_all(state.pool())
        .await
        .map_err(ApiError::from_sqlx)?;
    Ok(Json(values))
}

pub async fn patch_config(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(input): Json<BTreeMap<String, Value>>,
) -> Result<Json<BTreeMap<String, Value>>, ApiError> {
    let values = system_config::validate_patch(input)?;
    let mut tx = state.pool().begin().await.map_err(ApiError::from_sqlx)?;
    system_config::set_many(&mut tx, &values)
        .await
        .map_err(ApiError::from_sqlx)?;
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    let current = system_config::get_all(state.pool())
        .await
        .map_err(ApiError::from_sqlx)?;
    Ok(Json(current))
}
