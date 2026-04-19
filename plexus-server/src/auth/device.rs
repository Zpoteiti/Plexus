//! Device token CRUD + policy/MCP management endpoints.

use crate::auth::extract_claims;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures_util::SinkExt;
use plexus_common::consts::DEVICE_TOKEN_PREFIX;
use plexus_common::errors::{ApiError, ErrorCode};
use plexus_common::protocol::{FsPolicy, McpServerEntry, ServerToClient};
use serde::Deserialize;
use std::sync::Arc;

fn claims(headers: &HeaderMap, state: &AppState) -> Result<crate::auth::Claims, ApiError> {
    extract_claims(headers, &state.config.jwt_secret)
}

// -- Token CRUD --

#[derive(Deserialize)]
struct CreateTokenRequest {
    device_name: String,
}

async fn create_token(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTokenRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let token = format!("{}{}", DEVICE_TOKEN_PREFIX, uuid::Uuid::new_v4().simple());
    crate::db::devices::create_token(&state.db, &token, &c.sub, &req.device_name)
        .await
        .map_err(|e| {
            if e.to_string().contains("unique") || e.to_string().contains("duplicate") {
                ApiError::new(ErrorCode::Conflict, "Device name already exists")
            } else {
                ApiError::new(ErrorCode::InternalError, format!("{e}"))
            }
        })?;
    Ok(Json(serde_json::json!({
        "token": token,
        "device_name": req.device_name,
    })))
}

async fn list_tokens(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<crate::db::devices::DeviceToken>>, ApiError> {
    let c = claims(&headers, &state)?;
    let tokens = crate::db::devices::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(tokens))
}

async fn delete_token(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let dt = crate::db::devices::find_by_token(&state.db, &token)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Token not found"))?;
    if dt.user_id != c.sub {
        return Err(ApiError::new(ErrorCode::Forbidden, "Not your token"));
    }
    crate::db::devices::delete_token(&state.db, &token)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    let key = AppState::device_key(&c.sub, &dt.device_name);
    state.devices.remove(&key);
    if let Some(mut keys) = state.devices_by_user.get_mut(&c.sub) {
        keys.retain(|k| k != &key);
    }
    state.tool_schema_cache.remove(&c.sub);
    Ok(Json(serde_json::json!({ "message": "Token deleted" })))
}

// -- Device Status --

async fn list_devices(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let tokens = crate::db::devices::list_by_user(&state.db, &c.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    let mut devices = Vec::new();
    for dt in tokens {
        let key = AppState::device_key(&c.sub, &dt.device_name);
        let (status, tools_count) = if let Some(conn) = state.devices.get(&key) {
            ("online", conn.tools.len())
        } else {
            ("offline", 0)
        };
        devices.push(serde_json::json!({
            "device_name": dt.device_name,
            "status": status,
            "tools_count": tools_count,
            "fs_policy": dt.fs_policy,
        }));
    }
    Ok(Json(serde_json::json!(devices)))
}

// -- Config --

async fn get_config(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(device_name): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let dt = crate::db::devices::find_by_user_and_device(&state.db, &c.sub, &device_name)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Device not found"))?;
    Ok(Json(serde_json::json!({
        "device_name": dt.device_name,
        "fs_policy": dt.fs_policy,
        "workspace_path": dt.workspace_path,
        "shell_timeout_max": dt.shell_timeout_max,
        "ssrf_whitelist": dt.ssrf_whitelist,
        "mcp_servers": dt.mcp_config,
    })))
}

#[derive(Deserialize, Default)]
struct PatchDeviceConfig {
    #[serde(default)]
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub shell_timeout_max: Option<i32>,
    #[serde(default)]
    pub ssrf_whitelist: Option<Vec<String>>,
    #[serde(default)]
    pub fs_policy: Option<serde_json::Value>,
}

fn validate_patch(req: &PatchDeviceConfig) -> Result<(), ApiError> {
    let mut errors: Vec<String> = Vec::new();

    if let Some(wp) = &req.workspace_path
        && (wp.is_empty() || !wp.starts_with('/'))
    {
        errors.push("workspace_path=not absolute".to_string());
    }
    if let Some(n) = req.shell_timeout_max
        && !(10..=1800).contains(&n)
    {
        errors.push("shell_timeout_max=out of range (10-1800)".to_string());
    }
    if let Some(whitelist) = &req.ssrf_whitelist {
        for entry in whitelist {
            if entry.parse::<ipnet::IpNet>().is_err() {
                errors.push(format!("ssrf_whitelist=invalid CIDR: {entry}"));
                break;
            }
        }
    }
    if let Some(fp) = &req.fs_policy
        && serde_json::from_value::<FsPolicy>(fp.clone()).is_err()
    {
        errors.push("fs_policy=must be \"sandbox\" or \"unrestricted\"".to_string());
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ApiError::new(
            ErrorCode::ValidationFailed,
            format!("field errors: {}", errors.join("; ")),
        ))
    }
}

async fn patch_config(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(device_name): Path<String>,
    Json(req): Json<PatchDeviceConfig>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;

    validate_patch(&req)?;

    // Verify the device exists
    let _ = crate::db::devices::find_by_user_and_device(&state.db, &c.sub, &device_name)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Device not found"))?;

    // Apply each Some field to DB
    if let Some(wp) = &req.workspace_path {
        let updated =
            crate::db::devices::update_workspace_path(&state.db, &c.sub, &device_name, wp)
                .await
                .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
        if !updated {
            return Err(ApiError::new(ErrorCode::NotFound, "Device not found"));
        }
    }
    if let Some(st) = req.shell_timeout_max {
        crate::db::devices::update_shell_timeout_max(&state.db, &c.sub, &device_name, st)
            .await
            .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    }
    if let Some(whitelist) = &req.ssrf_whitelist {
        let val = serde_json::to_value(whitelist)
            .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
        crate::db::devices::update_ssrf_whitelist(&state.db, &c.sub, &device_name, &val)
            .await
            .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    }
    if let Some(fp) = &req.fs_policy {
        crate::db::devices::update_fs_policy(&state.db, &c.sub, &device_name, fp)
            .await
            .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    }

    // Build ConfigUpdate with all changed fields
    let new_fs_policy = req
        .fs_policy
        .as_ref()
        .and_then(|v| serde_json::from_value::<FsPolicy>(v.clone()).ok());
    let new_workspace_path = req.workspace_path.clone();
    let new_shell_timeout_max = req
        .shell_timeout_max
        .map(|n| u64::try_from(n).unwrap_or(n.unsigned_abs() as u64));
    let new_ssrf_whitelist = req.ssrf_whitelist.clone();

    if new_fs_policy.is_some()
        || new_workspace_path.is_some()
        || new_shell_timeout_max.is_some()
        || new_ssrf_whitelist.is_some()
    {
        push_config_update(&state, &c.sub, &device_name, |msg| {
            *msg = ServerToClient::ConfigUpdate {
                fs_policy: new_fs_policy,
                mcp_servers: None,
                workspace_path: new_workspace_path,
                shell_timeout_max: new_shell_timeout_max,
                ssrf_whitelist: new_ssrf_whitelist,
            };
        })
        .await;
    }

    // Re-read from DB for canonical response
    let dt = crate::db::devices::find_by_user_and_device(&state.db, &c.sub, &device_name)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Device not found"))?;
    Ok(Json(serde_json::json!({
        "device_name": dt.device_name,
        "fs_policy": dt.fs_policy,
        "workspace_path": dt.workspace_path,
        "shell_timeout_max": dt.shell_timeout_max,
        "ssrf_whitelist": dt.ssrf_whitelist,
        "mcp_servers": dt.mcp_config,
    })))
}

// -- MCP Config --

async fn get_mcp(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(device_name): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let dt = crate::db::devices::find_by_user_and_device(&state.db, &c.sub, &device_name)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Device not found"))?;
    Ok(Json(serde_json::json!({
        "device_name": dt.device_name,
        "mcp_servers": dt.mcp_config,
    })))
}

#[derive(Deserialize)]
struct McpUpdate {
    mcp_servers: serde_json::Value,
}

async fn put_mcp(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(device_name): Path<String>,
    Json(req): Json<McpUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let c = claims(&headers, &state)?;
    let updated =
        crate::db::devices::update_mcp_config(&state.db, &c.sub, &device_name, &req.mcp_servers)
            .await
            .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    if !updated {
        return Err(ApiError::new(ErrorCode::NotFound, "Device not found"));
    }
    push_config_update(&state, &c.sub, &device_name, |msg| {
        let mcp_servers: Vec<McpServerEntry> =
            serde_json::from_value(req.mcp_servers.clone()).unwrap_or_default();
        *msg = ServerToClient::ConfigUpdate {
            fs_policy: None,
            mcp_servers: Some(mcp_servers),
            workspace_path: None,
            shell_timeout_max: None,
            ssrf_whitelist: None,
        };
    })
    .await;
    Ok(Json(serde_json::json!({
        "device_name": device_name,
        "mcp_servers": req.mcp_servers,
    })))
}

/// Push a ConfigUpdate to a connected client device.
async fn push_config_update(
    state: &AppState,
    user_id: &str,
    device_name: &str,
    build: impl FnOnce(&mut ServerToClient),
) {
    let key = AppState::device_key(user_id, device_name);
    let sink = state.devices.get(&key).map(|conn| Arc::clone(&conn.sink));
    if let Some(sink) = sink {
        let mut msg = ServerToClient::HeartbeatAck; // placeholder
        build(&mut msg);
        let json = serde_json::to_string(&msg).unwrap();
        let mut s = sink.lock().await;
        let _ = s.send(axum::extract::ws::Message::Text(json.into())).await;
    }
}

pub fn device_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/device-tokens", post(create_token).get(list_tokens))
        .route("/api/device-tokens/{token}", delete(delete_token))
        .route("/api/devices", get(list_devices))
        .route(
            "/api/devices/{device_name}/config",
            get(get_config).patch(patch_config),
        )
        .route("/api/devices/{device_name}/mcp", get(get_mcp).put(put_mcp))
}
