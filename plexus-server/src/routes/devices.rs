use crate::{
    app::AppState,
    auth::AuthUser,
    db::devices::{self, DevicePatch, DeviceRow, NewDevice},
    error::ApiError,
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use plexus_common::{ErrorCode, protocol::McpServerConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

const REDACTED: &str = "<redacted>";

#[derive(Debug, Deserialize)]
pub struct CreateDeviceRequest {
    name: String,
    workspace_path: Option<String>,
    fs_policy: Option<String>,
    shell_timeout_max: Option<i32>,
    ssrf_whitelist: Option<Value>,
    mcp_servers: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct PatchDeviceRequest {
    name: Option<String>,
    workspace_path: Option<String>,
    fs_policy: Option<String>,
    shell_timeout_max: Option<i32>,
    ssrf_whitelist: Option<Value>,
    mcp_servers: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct DeviceResponse {
    name: String,
    workspace_path: String,
    fs_policy: String,
    shell_timeout_max: i32,
    ssrf_whitelist: Value,
    mcp_servers: Value,
    created_at: time::OffsetDateTime,
    online: bool,
    token_hint: String,
}

#[derive(Debug, Serialize)]
pub struct DeviceWithTokenResponse {
    token: String,
    device: DeviceResponse,
}

pub async fn create_device(
    AuthUser { user }: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateDeviceRequest>,
) -> Result<(StatusCode, Json<DeviceWithTokenResponse>), ApiError> {
    let new = request_to_new_device(req)?;
    let row = devices::create(state.pool(), user.id, new)
        .await
        .map_err(map_write_error)?;
    let token = row.token.clone();
    let device = response_for(&row, state.devices().is_online(&row.token).await);
    Ok((
        StatusCode::CREATED,
        Json(DeviceWithTokenResponse { token, device }),
    ))
}

pub async fn list_devices(
    AuthUser { user }: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<DeviceResponse>>, ApiError> {
    let rows = devices::list_by_user(state.pool(), user.id)
        .await
        .map_err(ApiError::from_sqlx)?;
    let mut devices = Vec::with_capacity(rows.len());
    for row in rows {
        let online = state.devices().is_online(&row.token).await;
        devices.push(response_for(&row, online));
    }
    Ok(Json(devices))
}

pub async fn patch_device(
    AuthUser { user }: AuthUser,
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<PatchDeviceRequest>,
) -> Result<Json<DeviceResponse>, ApiError> {
    let patch = request_to_patch(req)?;
    let row = devices::patch(state.pool(), user.id, &name, patch)
        .await
        .map_err(map_write_error)?
        .ok_or_else(not_found)?;
    let online = state.devices().is_online(&row.token).await;
    if online {
        state.devices().send_config_update(&row).await;
    }
    Ok(Json(response_for(&row, online)))
}

pub async fn regenerate_token(
    AuthUser { user }: AuthUser,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<DeviceWithTokenResponse>, ApiError> {
    let (old_token, row) = devices::regenerate_token(state.pool(), user.id, &name)
        .await
        .map_err(ApiError::from_sqlx)?
        .ok_or_else(not_found)?;
    state
        .devices()
        .close(&old_token, crate::devices::CloseReason::Unauthorized)
        .await;
    let token = row.token.clone();
    let device = response_for(&row, state.devices().is_online(&row.token).await);
    Ok(Json(DeviceWithTokenResponse { token, device }))
}

pub async fn delete_device(
    AuthUser { user }: AuthUser,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let row = devices::delete_by_user_and_name(state.pool(), user.id, &name)
        .await
        .map_err(ApiError::from_sqlx)?
        .ok_or_else(not_found)?;
    state
        .devices()
        .close(&row.token, crate::devices::CloseReason::Unauthorized)
        .await;
    Ok(StatusCode::NO_CONTENT)
}

fn request_to_new_device(req: CreateDeviceRequest) -> Result<NewDevice, ApiError> {
    let mut new = devices::default_new_device(&req.name).map_err(ApiError::invalid_args)?;
    if let Some(path) = req.workspace_path {
        new.workspace_path = validate_workspace_path(path)?;
    }
    if let Some(policy) = req.fs_policy {
        new.fs_policy = devices::validate_fs_policy(&policy).map_err(ApiError::invalid_args)?;
    }
    if let Some(timeout) = req.shell_timeout_max {
        new.shell_timeout_max =
            devices::validate_shell_timeout(timeout).map_err(ApiError::invalid_args)?;
    }
    if let Some(value) = req.ssrf_whitelist {
        new.ssrf_whitelist = validate_ssrf_whitelist(value)?;
    }
    if let Some(value) = req.mcp_servers {
        new.mcp_servers = validate_mcp_servers(value)?;
    }
    Ok(new)
}

fn request_to_patch(req: PatchDeviceRequest) -> Result<DevicePatch, ApiError> {
    Ok(DevicePatch {
        name: req
            .name
            .map(|name| devices::normalize_device_name(&name))
            .transpose()
            .map_err(ApiError::invalid_args)?,
        workspace_path: req
            .workspace_path
            .map(validate_workspace_path)
            .transpose()?,
        fs_policy: req
            .fs_policy
            .map(|policy| devices::validate_fs_policy(&policy))
            .transpose()
            .map_err(ApiError::invalid_args)?,
        shell_timeout_max: req
            .shell_timeout_max
            .map(devices::validate_shell_timeout)
            .transpose()
            .map_err(ApiError::invalid_args)?,
        ssrf_whitelist: req
            .ssrf_whitelist
            .map(validate_ssrf_whitelist)
            .transpose()?,
        mcp_servers: req.mcp_servers.map(validate_mcp_servers).transpose()?,
    })
}

fn validate_workspace_path(path: String) -> Result<String, ApiError> {
    if path.trim().is_empty() {
        return Err(ApiError::invalid_args("workspace_path must not be empty"));
    }
    Ok(path)
}

fn validate_ssrf_whitelist(value: Value) -> Result<Value, ApiError> {
    serde_json::from_value::<Vec<String>>(value.clone())
        .map_err(|_| ApiError::invalid_args("ssrf_whitelist must be an array of strings"))?;
    Ok(value)
}

fn validate_mcp_servers(value: Value) -> Result<Value, ApiError> {
    let configs = serde_json::from_value::<HashMap<String, McpServerConfig>>(value.clone())
        .map_err(|_| {
            ApiError::invalid_args("mcp_servers must be an object of MCP server configs")
        })?;
    reject_redacted_mcp_env(&configs)?;
    Ok(value)
}

fn reject_redacted_mcp_env(configs: &HashMap<String, McpServerConfig>) -> Result<(), ApiError> {
    for config in configs.values() {
        if config.env.values().any(|value| value == REDACTED) {
            return Err(ApiError::invalid_args(
                "mcp_servers env values cannot be the redaction marker",
            ));
        }
    }
    Ok(())
}

fn response_for(row: &DeviceRow, online: bool) -> DeviceResponse {
    DeviceResponse {
        name: row.name.clone(),
        workspace_path: row.workspace_path.clone(),
        fs_policy: row.fs_policy.clone(),
        shell_timeout_max: row.shell_timeout_max,
        ssrf_whitelist: row.ssrf_whitelist.clone(),
        mcp_servers: redact_mcp_env(&row.mcp_servers),
        created_at: row.created_at,
        online,
        token_hint: devices::token_hint(&row.token),
    }
}

fn redact_mcp_env(value: &Value) -> Value {
    let mut redacted = value.clone();
    if let Some(servers) = redacted.as_object_mut() {
        for server in servers.values_mut() {
            if let Some(env) = server.get_mut("env").and_then(Value::as_object_mut) {
                redact_env_values(env);
            }
        }
    }
    redacted
}

fn redact_env_values(env: &mut Map<String, Value>) {
    for value in env.values_mut() {
        *value = Value::String(REDACTED.to_string());
    }
}

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
        "device not found",
    )
}

fn map_write_error(err: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(db_err) = &err
        && db_err.is_unique_violation()
    {
        return ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::InvalidArgs,
            "device name already exists",
        );
    }
    ApiError::from_sqlx(err)
}
