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
    Ok(Json(system_config::redact_for_response(values)))
}

pub async fn patch_config(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(input): Json<BTreeMap<String, Value>>,
) -> Result<Json<BTreeMap<String, Value>>, ApiError> {
    let values = system_config::validate_patch(input)?;
    let limit = system_config::concurrency_limit(&values);
    let _guard = state.admin_config_lock().lock().await;

    let current = system_config::get_all(state.pool())
        .await
        .map_err(ApiError::from_sqlx)?;

    if system_config::identity_changed(&values) {
        let cfg = system_config::merged_llm_config(&current, &values)?;
        state.openai().client().validate_config(&cfg).await?;
    }

    if let Some(limit) = limit {
        crate::openai::OpenAiRuntime::new(limit)?;
    }

    let mut tx = state.pool().begin().await.map_err(ApiError::from_sqlx)?;
    system_config::set_many(&mut tx, &values)
        .await
        .map_err(ApiError::from_sqlx)?;
    tx.commit().await.map_err(ApiError::from_sqlx)?;

    if let Some(limit) = limit {
        state.openai().set_concurrency_limit(limit).await?;
    }

    let current = system_config::get_all(state.pool())
        .await
        .map_err(ApiError::from_sqlx)?;
    Ok(Json(system_config::redact_for_response(current)))
}
