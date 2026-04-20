//! Admin-only endpoints: default soul, rate limit, LLM config.

use crate::auth::extract_claims;
use crate::config::LlmConfig;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::{delete, get};
use axum::{Json, Router};
use plexus_common::errors::{ApiError, ErrorCode};
use serde::Deserialize;
use std::sync::Arc;
use tracing::info;

fn admin_claims(headers: &HeaderMap, state: &AppState) -> Result<crate::auth::Claims, ApiError> {
    let c = extract_claims(headers, &state.config.jwt_secret)?;
    if !c.is_admin {
        return Err(ApiError::new(ErrorCode::Forbidden, "Admin access required"));
    }
    Ok(c)
}

// -- Rate Limit --

async fn get_rate_limit(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_claims(&headers, &state)?;
    let limit = *state.rate_limit_config.read().await;
    Ok(Json(serde_json::json!({ "rate_limit_per_min": limit })))
}

#[derive(Deserialize)]
struct RateLimitUpdate {
    rate_limit_per_min: u32,
}

async fn put_rate_limit(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<RateLimitUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_claims(&headers, &state)?;
    crate::db::system_config::set(
        &state.db,
        "rate_limit_per_min",
        &req.rate_limit_per_min.to_string(),
    )
    .await
    .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    *state.rate_limit_config.write().await = req.rate_limit_per_min;
    Ok(Json(serde_json::json!({
        "message": "Rate limit updated",
        "rate_limit_per_min": req.rate_limit_per_min,
    })))
}

// -- LLM Config --

async fn get_llm_config(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_claims(&headers, &state)?;
    let config = state.llm_config.read().await;
    match config.as_ref() {
        Some(c) => {
            let masked_key = if c.api_key.len() > 8 {
                format!(
                    "{}...{}",
                    &c.api_key[..4],
                    &c.api_key[c.api_key.len() - 4..]
                )
            } else {
                "***".into()
            };
            Ok(Json(serde_json::json!({
                "api_base": c.api_base,
                "model": c.model,
                "api_key": masked_key,
                "context_window": c.context_window,
            })))
        }
        None => Ok(Json(serde_json::json!({
            "status": "not_configured",
            "message": "LLM config not set. Use PUT /api/llm-config to configure.",
        }))),
    }
}

#[derive(Deserialize)]
struct LlmConfigUpdate {
    api_base: String,
    model: Option<String>,
    api_key: Option<String>,
    context_window: Option<u32>,
}

async fn put_llm_config(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<LlmConfigUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_claims(&headers, &state)?;

    let current = state.llm_config.read().await.clone();
    let new_config = LlmConfig {
        api_base: req.api_base,
        model: req
            .model
            .or(current.as_ref().map(|c| c.model.clone()))
            .unwrap_or_else(|| "gpt-4o".into()),
        api_key: req
            .api_key
            .or(current.as_ref().map(|c| c.api_key.clone()))
            .unwrap_or_default(),
        context_window: req
            .context_window
            .or(current.as_ref().map(|c| c.context_window))
            .unwrap_or(204_800),
    };

    let json = serde_json::to_string(&new_config)
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    crate::db::system_config::set(&state.db, "llm_config", &json)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    *state.llm_config.write().await = Some(new_config);

    // Reset vision_stripped on every live session so the next turn retries
    // images against the newly configured model.
    for entry in state.sessions.iter() {
        entry
            .value()
            .vision_stripped
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
    info!(
        "Reset vision_stripped on {} live sessions after LLM config update",
        state.sessions.len()
    );

    Ok(Json(serde_json::json!({ "message": "LLM config updated" })))
}

// -- Server MCP --

async fn get_server_mcp(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    admin_claims(&headers, &state)?;
    let mcp_json = crate::db::system_config::get(&state.db, "server_mcp_config")
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .unwrap_or_else(|| "[]".into());
    let servers: serde_json::Value =
        serde_json::from_str(&mcp_json).unwrap_or(serde_json::json!([]));
    Ok(Json(serde_json::json!({ "mcp_servers": servers })))
}

#[derive(Deserialize)]
struct McpConfigUpdate {
    mcp_servers: Vec<plexus_common::protocol::McpServerEntry>,
}

async fn put_server_mcp(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<McpConfigUpdate>,
) -> Result<axum::response::Response, ApiError> {
    admin_claims(&headers, &state)?;

    // FR6 / spec §4.6: introspect each new MCP entry, then compare its
    // tool schemas against every other live install (other server-side
    // MCPs in the same batch + every per-user-device MCP currently in
    // the device cache). Any collision returns 409 with a structured
    // `conflicts[]` body; introspection failure returns 400. Only on
    // clean validation do we persist + reinitialize.
    let incoming_names: std::collections::HashSet<String> =
        req.mcp_servers.iter().map(|e| e.name.clone()).collect();

    // Existing baseline = (a) server-side MCPs NOT being replaced in this
    // batch + (b) every device's cached mcp_schemas across all users.
    let mut existing: Vec<crate::mcp::wrap::McpInstall> = Vec::new();
    {
        let server_mcp = state.server_mcp.read().await;
        for (server_name, tools) in server_mcp.raw_tool_schemas_by_server() {
            if incoming_names.contains(&server_name) {
                continue;
            }
            existing.push(crate::mcp::wrap::McpInstall {
                install_site: plexus_common::consts::SERVER_DEVICE_NAME.to_string(),
                mcp_server_name: server_name,
                tools,
            });
        }
    }
    for entry in state.devices.iter() {
        let conn = entry.value();
        let installs =
            crate::mcp::wrap::installs_from_reported_schemas(&conn.device_name, &conn.mcp_schemas);
        existing.extend(installs);
    }

    // Introspect every enabled entry. Disabled entries are persisted
    // as-is (they won't be started) but still collision-checked so a
    // future enable doesn't surprise the user.
    let mut introspected: Vec<(String, Vec<(String, serde_json::Value)>)> = Vec::new();
    let mut within_batch: Vec<crate::mcp::wrap::McpInstall> = Vec::new();
    let mut all_conflicts: Vec<serde_json::Value> = Vec::new();

    for entry in &req.mcp_servers {
        let tools = match crate::server_mcp::introspect_entry(entry).await {
            Ok(t) => t,
            Err(e) => {
                return Err(ApiError::new(
                    ErrorCode::ValidationFailed,
                    format!("MCP introspection failed: {e}"),
                ));
            }
        };
        let incoming_install = crate::mcp::wrap::McpInstall {
            install_site: plexus_common::consts::SERVER_DEVICE_NAME.to_string(),
            mcp_server_name: entry.name.clone(),
            tools: tools.clone(),
        };

        // Collision check: (a) vs existing baseline, (b) vs earlier
        // entries in THIS batch (catches two admin-supplied MCPs that
        // collide with each other).
        let mut diffs = crate::mcp::wrap::diff_mcp_schema_collisions(&existing, &incoming_install);
        diffs.extend(crate::mcp::wrap::diff_mcp_schema_collisions(
            &within_batch,
            &incoming_install,
        ));

        if !diffs.is_empty() {
            for d in diffs {
                all_conflicts.push(serde_json::json!({
                    "mcp_server": entry.name,
                    "tool": d.tool,
                    "existing_schema": d.existing_schema,
                    "new_schema": d.new_schema,
                    "where_installed": d.where_installed,
                }));
            }
        }

        within_batch.push(incoming_install);
        introspected.push((entry.name.clone(), tools));
    }

    if !all_conflicts.is_empty() {
        // 409 body shape: flat {code, message} so the existing
        // `ApiError::IntoResponse` serialization stays consistent for
        // clients that just read `message`. Structured `conflicts` are
        // appended alongside so the frontend can optionally render a
        // per-tool diff.
        let mut conflict_servers: Vec<String> = Vec::new();
        for c in &all_conflicts {
            if let Some(s) = c.get("mcp_server").and_then(|v| v.as_str())
                && !conflict_servers.iter().any(|x| x == s)
            {
                conflict_servers.push(s.to_string());
            }
        }
        let body = serde_json::json!({
            "code": ErrorCode::Conflict.as_str(),
            "message": format!(
                "MCP schema collision on server(s): {}. Rename the install or ask an admin to upgrade.",
                conflict_servers.join(", ")
            ),
            "error": "mcp_schema_collision",
            "conflicts": all_conflicts,
        });
        return Ok((
            axum::http::StatusCode::CONFLICT,
            [("content-type", "application/json")],
            serde_json::to_string(&body).unwrap_or_default(),
        )
            .into_response());
    }

    // Introspection clean — persist + reinitialize.
    let json = serde_json::to_string(&req.mcp_servers)
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    crate::db::system_config::set(&state.db, "server_mcp_config", &json)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    state
        .server_mcp
        .write()
        .await
        .reinitialize(&req.mcp_servers)
        .await;
    // Server MCP tools are shared — invalidate cache for all users
    state.tool_schema_cache.clear();
    let _ = introspected; // tool list reuse is a future optimization
    Ok(Json(serde_json::json!({ "mcp_servers": req.mcp_servers })).into_response())
}

// -- List Users (Admin) --

#[derive(serde::Serialize, sqlx::FromRow)]
struct AdminUserSummary {
    user_id: String,
    email: String,
    display_name: Option<String>,
    is_admin: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    last_heartbeat_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn list_users(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<AdminUserSummary>>, ApiError> {
    let _admin = admin_claims(&headers, &state)?;
    let rows: Vec<AdminUserSummary> = sqlx::query_as(
        "SELECT user_id, email, display_name, is_admin, created_at, last_heartbeat_at \
         FROM users \
         ORDER BY created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?;
    Ok(Json(rows))
}

// -- Delete User (Admin) --

async fn delete_user_by_admin(
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let admin = admin_claims(&headers, &state)?;
    let user = crate::db::users::find_by_id(&state.db, &user_id)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "User not found"))?;

    tracing::info!(
        admin_id = %admin.sub,
        target_user_id = %user.user_id,
        target_email = %user.email,
        "Admin deleting user"
    );

    if admin.sub == user_id {
        tracing::warn!(
            admin_id = %admin.sub,
            "Admin is deleting their own account via admin endpoint"
        );
    }

    crate::account::delete_user_everywhere(&state, &user.user_id).await;

    Ok(Json(serde_json::json!({ "message": "User deleted" })))
}

pub fn admin_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/admin/rate-limit",
            get(get_rate_limit).put(put_rate_limit),
        )
        .route("/api/llm-config", get(get_llm_config).put(put_llm_config))
        .route("/api/server-mcp", get(get_server_mcp).put(put_server_mcp))
        .route("/api/admin/users", get(list_users))
        .route("/api/admin/users/{user_id}", delete(delete_user_by_admin))
}
