# Plexus M1e Device Connectivity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the server-side device lifecycle and WebSocket connectivity foundation for M1e.

**Architecture:** Persist device rows in the existing `devices` table, expose authenticated REST routes for device lifecycle, and keep online state in an in-memory device runtime. The WebSocket path authenticates by device token, completes `hello` / `hello_ack`, registers a connection handle, and uses that handle for heartbeat, duplicate replacement, revocation closes, and `config_update` sends.

**Tech Stack:** Rust 2024, Axum 0.8 REST + WebSocket, Tokio, SQLx/Postgres, `plexus-common::protocol::WsFrame`, integration tests with local Axum listener and `tokio-tungstenite`.

---

## File Structure

- `Cargo.toml` - enable Axum WebSocket support and add shared websocket test utilities.
- `plexus-server/Cargo.toml` - add `futures-util`; add `tokio-tungstenite` as dev dependency.
- `plexus-server/src/app.rs` - store `DeviceRuntime` in `AppState`.
- `plexus-server/src/db/mod.rs` - export `devices`.
- `plexus-server/src/db/devices.rs` - device row model, slug normalization, token generation/hinting, CRUD helpers.
- `plexus-server/src/devices/mod.rs` - runtime exports and heartbeat config.
- `plexus-server/src/devices/registry.rs` - in-memory connection registry with stale-cleanup protection.
- `plexus-server/src/devices/ws.rs` - `/ws/device` upgrade, handshake, read/write loop, heartbeat.
- `plexus-server/src/routes/mod.rs` - mount device REST routes and WebSocket route.
- `plexus-server/src/routes/devices.rs` - REST request/response shape and lifecycle handlers.
- `plexus-server/tests/support/mod.rs` - add real local-server helper for WS tests.
- `plexus-server/tests/support/device_client.rs` - in-process fake WebSocket device client.
- `plexus-server/tests/m1e_devices_rest.rs` - REST lifecycle tests.
- `plexus-server/tests/m1e_device_ws.rs` - WebSocket handshake, heartbeat, duplicate, revoke tests.
- `docs/SCHEMA.md` - update device API/schema notes after implementation.

Boundaries:

- `db::devices` owns persistence and pure helper logic.
- `routes::devices` owns browser JWT auth, HTTP request validation, response shaping.
- `devices::registry` owns online state and connection commands.
- `devices::ws` owns WebSocket state transitions and close codes.

---

### Task 1: Device DB Helpers, Slug Names, and Token Hints

**Files:**
- Create: `plexus-server/src/db/devices.rs`
- Modify: `plexus-server/src/db/mod.rs`

- [ ] **Step 1: Write failing helper tests**

Create `plexus-server/src/db/devices.rs` with only this test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_name_normalizes_to_slug() {
        assert_eq!(normalize_device_name("MacBook Pro").unwrap(), "macbook-pro");
        assert_eq!(normalize_device_name("John's iPhone 6S").unwrap(), "johns-iphone-6s");
        assert_eq!(normalize_device_name("lab_pc_01").unwrap(), "lab-pc-01");
        assert_eq!(normalize_device_name("lab--machine").unwrap(), "lab-machine");
    }

    #[test]
    fn device_name_rejects_reserved_empty_and_non_ascii() {
        assert!(normalize_device_name("server").is_err());
        assert!(normalize_device_name("Server").is_err());
        assert!(normalize_device_name("  ---  ").is_err());
        assert!(normalize_device_name("办公室电脑").is_err());
        assert!(normalize_device_name("bad/name").is_err());
    }

    #[test]
    fn token_hint_keeps_prefix_and_last_four_only() {
        assert_eq!(token_hint("plexus_dev_abcdefghijklmnopqrstuvwxyz"), "plexus_dev_...wxyz");
    }

    #[test]
    fn generated_token_has_device_prefix_and_entropy() {
        let token = generate_device_token();
        assert!(token.starts_with(plexus_common::consts::DEVICE_TOKEN_PREFIX));
        assert!(token.len() > plexus_common::consts::DEVICE_TOKEN_PREFIX.len() + 32);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p plexus-server db::devices::tests -- --nocapture
```

Expected: compile failure because `db::devices` is not exported and helper functions do not exist.

- [ ] **Step 3: Export the DB module**

Modify `plexus-server/src/db/mod.rs`:

```rust
pub mod devices;
pub mod messages;
pub mod pending_messages;
pub mod sessions;
pub mod system_config;
pub mod users;
```

- [ ] **Step 4: Implement helpers and row types**

Implement the top of `plexus-server/src/db/devices.rs`:

```rust
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use plexus_common::consts::DEVICE_TOKEN_PREFIX;
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

pub const DEFAULT_WORKSPACE_PATH: &str = "~/plexus/workspace";
pub const DEFAULT_FS_POLICY: &str = "sandbox";
pub const DEFAULT_SHELL_TIMEOUT_MAX: i32 = 300;
pub const MAX_DEVICE_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DeviceRow {
    pub token: String,
    pub user_id: Uuid,
    pub name: String,
    pub workspace_path: String,
    pub fs_policy: String,
    pub shell_timeout_max: i32,
    pub ssrf_whitelist: Value,
    pub mcp_servers: Value,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct NewDevice {
    pub name: String,
    pub workspace_path: String,
    pub fs_policy: String,
    pub shell_timeout_max: i32,
    pub ssrf_whitelist: Value,
    pub mcp_servers: Value,
}

#[derive(Debug, Clone, Default)]
pub struct DevicePatch {
    pub name: Option<String>,
    pub workspace_path: Option<String>,
    pub fs_policy: Option<String>,
    pub shell_timeout_max: Option<i32>,
    pub ssrf_whitelist: Option<Value>,
    pub mcp_servers: Option<Value>,
}

pub fn generate_device_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{}{}", DEVICE_TOKEN_PREFIX, URL_SAFE_NO_PAD.encode(bytes))
}

pub fn token_hint(token: &str) -> String {
    let suffix = token.get(token.len().saturating_sub(4)..).unwrap_or(token);
    format!("{}...{}", DEVICE_TOKEN_PREFIX, suffix)
}

pub fn normalize_device_name(raw: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if ch.is_ascii_whitespace() || ch == '_' || ch == '-' {
            if !out.is_empty() && !last_was_sep {
                out.push('-');
                last_was_sep = true;
            }
        } else if ch == '\'' {
            continue;
        } else {
            return Err("device name may contain only ASCII letters, digits, spaces, underscores, apostrophes, and hyphens".to_string());
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        return Err("device name must not be empty".to_string());
    }
    if out == "server" {
        return Err("device name 'server' is reserved".to_string());
    }
    if out.chars().count() > MAX_DEVICE_NAME_CHARS {
        return Err(format!("device name must be at most {MAX_DEVICE_NAME_CHARS} characters"));
    }
    Ok(out)
}
```

- [ ] **Step 5: Add CRUD helpers**

Append these functions to `db/devices.rs`:

```rust
pub async fn create(pool: &PgPool, user_id: Uuid, new: NewDevice) -> Result<DeviceRow, sqlx::Error> {
    let token = generate_device_token();
    sqlx::query_as::<_, DeviceRow>(
        r#"
        INSERT INTO devices (token, user_id, name, workspace_path, fs_policy,
                             shell_timeout_max, ssrf_whitelist, mcp_servers)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING token, user_id, name, workspace_path, fs_policy,
                  shell_timeout_max, ssrf_whitelist, mcp_servers, created_at
        "#,
    )
    .bind(token)
    .bind(user_id)
    .bind(new.name)
    .bind(new.workspace_path)
    .bind(new.fs_policy)
    .bind(new.shell_timeout_max)
    .bind(new.ssrf_whitelist)
    .bind(new.mcp_servers)
    .fetch_one(pool)
    .await
}

pub async fn list_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<DeviceRow>, sqlx::Error> {
    sqlx::query_as::<_, DeviceRow>(
        r#"
        SELECT token, user_id, name, workspace_path, fs_policy,
               shell_timeout_max, ssrf_whitelist, mcp_servers, created_at
        FROM devices
        WHERE user_id = $1
        ORDER BY created_at ASC, name ASC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn find_by_token(pool: &PgPool, token: &str) -> Result<Option<DeviceRow>, sqlx::Error> {
    sqlx::query_as::<_, DeviceRow>(
        r#"
        SELECT token, user_id, name, workspace_path, fs_policy,
               shell_timeout_max, ssrf_whitelist, mcp_servers, created_at
        FROM devices
        WHERE token = $1
        "#,
    )
    .bind(token)
    .fetch_optional(pool)
    .await
}

pub async fn find_owned_by_name(pool: &PgPool, user_id: Uuid, name: &str) -> Result<Option<DeviceRow>, sqlx::Error> {
    sqlx::query_as::<_, DeviceRow>(
        r#"
        SELECT token, user_id, name, workspace_path, fs_policy,
               shell_timeout_max, ssrf_whitelist, mcp_servers, created_at
        FROM devices
        WHERE user_id = $1 AND name = $2
        "#,
    )
    .bind(user_id)
    .bind(name)
    .fetch_optional(pool)
    .await
}
```

- [ ] **Step 6: Keep slug validation in runtime code**

Do not change `plexus-server/src/db/schema.sql` in this task. M1 has no migration framework, and runtime normalization plus the existing `server` reserved-name check is the authoritative M1e validation boundary.

- [ ] **Step 7: Run helper tests**

Run:

```bash
cargo test -p plexus-server db::devices::tests -- --nocapture
```

Expected: helper tests pass.

- [ ] **Step 8: Commit**

```bash
git add plexus-server/src/db/mod.rs plexus-server/src/db/devices.rs
git commit -m "feat: add device persistence helpers"
```

---

### Task 2: Device REST Create and List

**Files:**
- Create: `plexus-server/tests/m1e_devices_rest.rs`
- Create: `plexus-server/src/routes/devices.rs`
- Modify: `plexus-server/src/routes/mod.rs`

- [ ] **Step 1: Write failing REST tests**

Create `plexus-server/tests/m1e_devices_rest.rs`:

```rust
mod support;

use axum::http::{Method, StatusCode};
use serde_json::json;
use support::{TestApp, json_request, register_user};

#[tokio::test]
async fn create_device_returns_token_once_and_list_returns_details_with_hint() {
    let app = TestApp::spawn().await;
    let (jwt, _) = register_user(&app, "device-owner@example.com").await;

    let (status, created) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices",
        json!({ "name": "MacBook Pro" }),
        Some(&jwt),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    let token = created["token"].as_str().unwrap();
    assert!(token.starts_with("plexus_dev_"));
    assert_eq!(created["device"]["name"], "macbook-pro");
    assert_eq!(created["device"]["workspace_path"], "~/plexus/workspace");
    assert_eq!(created["device"]["fs_policy"], "sandbox");
    assert_eq!(created["device"]["shell_timeout_max"], 300);
    assert_eq!(created["device"]["ssrf_whitelist"], json!([]));
    assert_eq!(created["device"]["mcp_servers"], json!({}));
    assert_eq!(created["device"]["online"], false);
    assert_eq!(created["device"]["token_hint"], format!("plexus_dev_...{}", &token[token.len() - 4..]));

    let (status, listed) = json_request(app.router.clone(), Method::GET, "/api/devices", json!(null), Some(&jwt)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["name"], "macbook-pro");
    assert_eq!(listed[0]["token_hint"], created["device"]["token_hint"]);
    assert!(listed[0].get("token").is_none());
}

#[tokio::test]
async fn create_device_accepts_workspace_override_verbatim() {
    let app = TestApp::spawn().await;
    let (jwt, _) = register_user(&app, "path-owner@example.com").await;
    let (status, created) = json_request(
        app.router.clone(),
        Method::POST,
        "/api/devices",
        json!({ "name": "lab pc 01", "workspace_path": "/tmp/plexus-testing-path" }),
        Some(&jwt),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["device"]["name"], "lab-pc-01");
    assert_eq!(created["device"]["workspace_path"], "/tmp/plexus-testing-path");
}
```

- [ ] **Step 2: Run tests and verify they fail**

```bash
cargo test -p plexus-server --test m1e_devices_rest -- --nocapture
```

Expected: 404 for `/api/devices`.

- [ ] **Step 3: Implement route response shaping**

Create `plexus-server/src/routes/devices.rs` with request/response structs and helper functions:

```rust
use crate::{auth::AuthUser, db::devices as device_db, error::ApiError};
use axum::{Json, extract::State, http::StatusCode};
use plexus_common::{DeviceConfig, ErrorCode, FsPolicy, McpServerConfig};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use time::OffsetDateTime;

#[derive(Deserialize)]
pub struct CreateDeviceRequest {
    name: String,
    workspace_path: Option<String>,
    fs_policy: Option<FsPolicy>,
    shell_timeout_max: Option<u32>,
    ssrf_whitelist: Option<Vec<String>>,
    mcp_servers: Option<HashMap<String, McpServerConfig>>,
}

#[derive(Serialize)]
pub struct DeviceResponse {
    name: String,
    token_hint: String,
    online: bool,
    workspace_path: String,
    fs_policy: FsPolicy,
    shell_timeout_max: u32,
    ssrf_whitelist: Vec<String>,
    mcp_servers: HashMap<String, McpServerConfig>,
    created_at: OffsetDateTime,
}

#[derive(Serialize)]
pub struct CreateDeviceResponse {
    device: DeviceResponse,
    token: String,
}

pub fn row_to_config(row: &device_db::DeviceRow) -> Result<DeviceConfig, ApiError> {
    let fs_policy = match row.fs_policy.as_str() {
        "sandbox" => FsPolicy::Sandbox,
        "unrestricted" => FsPolicy::Unrestricted,
        _ => return Err(ApiError::invalid_args("invalid device fs_policy stored in database")),
    };
    Ok(DeviceConfig {
        workspace_path: row.workspace_path.clone(),
        fs_policy,
        shell_timeout_max: row.shell_timeout_max as u32,
        ssrf_whitelist: serde_json::from_value(row.ssrf_whitelist.clone()).map_err(|_| ApiError::invalid_args("invalid device ssrf_whitelist stored in database"))?,
        mcp_servers: serde_json::from_value(row.mcp_servers.clone()).map_err(|_| ApiError::invalid_args("invalid device mcp_servers stored in database"))?,
    })
}

pub fn row_to_response(row: &device_db::DeviceRow, online: bool) -> Result<DeviceResponse, ApiError> {
    let config = row_to_config(row)?;
    Ok(DeviceResponse {
        name: row.name.clone(),
        token_hint: device_db::token_hint(&row.token),
        online,
        workspace_path: config.workspace_path,
        fs_policy: config.fs_policy,
        shell_timeout_max: config.shell_timeout_max,
        ssrf_whitelist: config.ssrf_whitelist,
        mcp_servers: config.mcp_servers,
        created_at: row.created_at,
    })
}

fn fs_policy_string(policy: FsPolicy) -> &'static str {
    match policy {
        FsPolicy::Sandbox => "sandbox",
        FsPolicy::Unrestricted => "unrestricted",
    }
}
```

- [ ] **Step 4: Implement create/list handlers**

Append:

```rust
pub async fn create_device(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Json(req): Json<CreateDeviceRequest>,
) -> Result<(StatusCode, Json<CreateDeviceResponse>), ApiError> {
    let name = device_db::normalize_device_name(&req.name).map_err(ApiError::invalid_args)?;
    let shell_timeout_max = req.shell_timeout_max.unwrap_or(device_db::DEFAULT_SHELL_TIMEOUT_MAX as u32);
    if shell_timeout_max == 0 || shell_timeout_max > 86_400 {
        return Err(ApiError::invalid_args("shell_timeout_max must be between 1 and 86400 seconds"));
    }
    let new = device_db::NewDevice {
        name,
        workspace_path: req.workspace_path.unwrap_or_else(|| device_db::DEFAULT_WORKSPACE_PATH.to_string()),
        fs_policy: fs_policy_string(req.fs_policy.unwrap_or(FsPolicy::Sandbox)).to_string(),
        shell_timeout_max: shell_timeout_max as i32,
        ssrf_whitelist: serde_json::to_value(req.ssrf_whitelist.unwrap_or_default()).unwrap_or_else(|_| json!([])),
        mcp_servers: serde_json::to_value(req.mcp_servers.unwrap_or_default()).unwrap_or_else(|_| json!({})),
    };
    let row = device_db::create(state.pool(), auth.user.id, new).await.map_err(map_device_write_error)?;
    Ok((StatusCode::CREATED, Json(CreateDeviceResponse {
        token: row.token.clone(),
        device: row_to_response(&row, false)?,
    })))
}

pub async fn list_devices(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
) -> Result<Json<Vec<DeviceResponse>>, ApiError> {
    let rows = device_db::list_for_user(state.pool(), auth.user.id).await.map_err(ApiError::from_sqlx)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_response(&row, false)?);
    }
    Ok(Json(out))
}

fn map_device_write_error(err: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(db_err) = &err {
        if db_err.is_unique_violation() {
            return ApiError::new(StatusCode::CONFLICT, ErrorCode::InvalidArgs, "device name already exists");
        }
    }
    ApiError::from_sqlx(err)
}
```

- [ ] **Step 5: Mount list/create routes**

Modify `plexus-server/src/routes/mod.rs`:

```rust
pub mod devices;
```

Add to `router()`:

```rust
.route(
    "/api/devices",
    get(devices::list_devices).post(devices::create_device),
)
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p plexus-server --test m1e_devices_rest -- --nocapture
```

Expected: create/list tests pass.

- [ ] **Step 7: Commit**

```bash
git add plexus-server/src/db/devices.rs plexus-server/src/routes/mod.rs plexus-server/src/routes/devices.rs plexus-server/tests/m1e_devices_rest.rs
git commit -m "feat: add device create and list APIs"
```

---

### Task 3: Patch, Regenerate, and Delete REST Routes

**Files:**
- Modify: `plexus-server/src/db/devices.rs`
- Modify: `plexus-server/src/routes/devices.rs`
- Modify: `plexus-server/src/routes/mod.rs`
- Modify: `plexus-server/tests/m1e_devices_rest.rs`

- [ ] **Step 1: Add failing lifecycle tests**

Append to `m1e_devices_rest.rs`:

```rust
#[tokio::test]
async fn patch_renames_and_updates_config_without_returning_token() {
    let app = TestApp::spawn().await;
    let (jwt, _) = register_user(&app, "patch-owner@example.com").await;
    let (_, created) = json_request(app.router.clone(), Method::POST, "/api/devices", json!({ "name": "old laptop" }), Some(&jwt)).await;
    let original_token = created["token"].as_str().unwrap().to_string();

    let (status, patched) = json_request(app.router.clone(), Method::PATCH, "/api/devices/old-laptop/config", json!({
        "name": "new laptop",
        "workspace_path": "/tmp/plexus-testing-path",
        "fs_policy": "unrestricted",
        "shell_timeout_max": 120,
        "ssrf_whitelist": ["10.0.0.1:8080"],
        "mcp_servers": {}
    }), Some(&jwt)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(patched["name"], "new-laptop");
    assert_eq!(patched["workspace_path"], "/tmp/plexus-testing-path");
    assert_eq!(patched["fs_policy"], "unrestricted");
    assert!(patched.get("token").is_none());

    let (_, listed) = json_request(app.router.clone(), Method::GET, "/api/devices", json!(null), Some(&jwt)).await;
    assert_eq!(listed[0]["token_hint"], format!("plexus_dev_...{}", &original_token[original_token.len() - 4..]));
}

#[tokio::test]
async fn regenerate_preserves_config_and_returns_new_token_once() {
    let app = TestApp::spawn().await;
    let (jwt, _) = register_user(&app, "regen-owner@example.com").await;
    let (_, created) = json_request(app.router.clone(), Method::POST, "/api/devices", json!({ "name": "devbox", "workspace_path": "/tmp/keep-me" }), Some(&jwt)).await;
    let old_token = created["token"].as_str().unwrap().to_string();

    let (status, regenerated) = json_request(app.router.clone(), Method::POST, "/api/devices/devbox/regenerate-token", json!(null), Some(&jwt)).await;
    assert_eq!(status, StatusCode::OK);
    let new_token = regenerated["token"].as_str().unwrap();
    assert_ne!(new_token, old_token);
    assert_eq!(regenerated["device"]["workspace_path"], "/tmp/keep-me");

    let (_, listed) = json_request(app.router.clone(), Method::GET, "/api/devices", json!(null), Some(&jwt)).await;
    assert!(listed[0].get("token").is_none());
    assert_eq!(listed[0]["token_hint"], format!("plexus_dev_...{}", &new_token[new_token.len() - 4..]));
}

#[tokio::test]
async fn delete_removes_device() {
    let app = TestApp::spawn().await;
    let (jwt, _) = register_user(&app, "delete-owner@example.com").await;
    let _ = json_request(app.router.clone(), Method::POST, "/api/devices", json!({ "name": "trash pc" }), Some(&jwt)).await;

    let (status, _) = support::empty_request(app.router.clone(), Method::DELETE, "/api/devices/trash-pc", Some(&jwt)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, listed) = json_request(app.router.clone(), Method::GET, "/api/devices", json!(null), Some(&jwt)).await;
    assert_eq!(listed.as_array().unwrap().len(), 0);
}
```

- [ ] **Step 2: Run tests and verify they fail**

```bash
cargo test -p plexus-server --test m1e_devices_rest -- --nocapture
```

Expected: patch/regenerate/delete routes return 404.

- [ ] **Step 3: Add DB mutation helpers**

Append to `db/devices.rs`:

```rust
pub async fn update_owned_by_name(pool: &PgPool, user_id: Uuid, current_name: &str, patch: DevicePatch) -> Result<Option<DeviceRow>, sqlx::Error> {
    sqlx::query_as::<_, DeviceRow>(
        r#"
        UPDATE devices
        SET name = COALESCE($3, name),
            workspace_path = COALESCE($4, workspace_path),
            fs_policy = COALESCE($5, fs_policy),
            shell_timeout_max = COALESCE($6, shell_timeout_max),
            ssrf_whitelist = COALESCE($7, ssrf_whitelist),
            mcp_servers = COALESCE($8, mcp_servers)
        WHERE user_id = $1 AND name = $2
        RETURNING token, user_id, name, workspace_path, fs_policy,
                  shell_timeout_max, ssrf_whitelist, mcp_servers, created_at
        "#,
    )
    .bind(user_id)
    .bind(current_name)
    .bind(patch.name)
    .bind(patch.workspace_path)
    .bind(patch.fs_policy)
    .bind(patch.shell_timeout_max)
    .bind(patch.ssrf_whitelist)
    .bind(patch.mcp_servers)
    .fetch_optional(pool)
    .await
}

pub async fn regenerate_token_owned_by_name(pool: &PgPool, user_id: Uuid, name: &str) -> Result<Option<(String, String, DeviceRow)>, sqlx::Error> {
    let old = find_owned_by_name(pool, user_id, name).await?;
    let Some(old) = old else { return Ok(None); };
    let old_token = old.token.clone();
    let new_token = generate_device_token();
    let row = sqlx::query_as::<_, DeviceRow>(
        r#"
        UPDATE devices
        SET token = $3
        WHERE user_id = $1 AND name = $2
        RETURNING token, user_id, name, workspace_path, fs_policy,
                  shell_timeout_max, ssrf_whitelist, mcp_servers, created_at
        "#,
    )
    .bind(user_id)
    .bind(name)
    .bind(&new_token)
    .fetch_one(pool)
    .await?;
    Ok(Some((old_token, new_token, row)))
}

pub async fn delete_owned_by_name(pool: &PgPool, user_id: Uuid, name: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>("DELETE FROM devices WHERE user_id = $1 AND name = $2 RETURNING token")
        .bind(user_id)
        .bind(name)
        .fetch_optional(pool)
        .await
}
```

- [ ] **Step 4: Implement REST handlers**

Append to `routes/devices.rs`:

```rust
use axum::extract::Path;

#[derive(Deserialize)]
pub struct PatchDeviceRequest {
    name: Option<String>,
    workspace_path: Option<String>,
    fs_policy: Option<FsPolicy>,
    shell_timeout_max: Option<u32>,
    ssrf_whitelist: Option<Vec<String>>,
    mcp_servers: Option<HashMap<String, McpServerConfig>>,
}

#[derive(Serialize)]
pub struct RegenerateTokenResponse {
    device: DeviceResponse,
    token: String,
}

pub async fn patch_device_config(auth: AuthUser, State(state): State<crate::app::AppState>, Path(name): Path<String>, Json(req): Json<PatchDeviceRequest>) -> Result<Json<DeviceResponse>, ApiError> {
    let timeout = match req.shell_timeout_max {
        Some(0) => return Err(ApiError::invalid_args("shell_timeout_max must be between 1 and 86400 seconds")),
        Some(value) if value > 86_400 => return Err(ApiError::invalid_args("shell_timeout_max must be between 1 and 86400 seconds")),
        Some(value) => Some(value as i32),
        None => None,
    };
    let patch = device_db::DevicePatch {
        name: req.name.as_deref().map(device_db::normalize_device_name).transpose().map_err(ApiError::invalid_args)?,
        workspace_path: req.workspace_path,
        fs_policy: req.fs_policy.map(fs_policy_string).map(str::to_string),
        shell_timeout_max: timeout,
        ssrf_whitelist: req.ssrf_whitelist.map(|v| serde_json::to_value(v).unwrap()),
        mcp_servers: req.mcp_servers.map(|v| serde_json::to_value(v).unwrap()),
    };
    let row = device_db::update_owned_by_name(state.pool(), auth.user.id, &name, patch).await.map_err(map_device_write_error)?.ok_or_else(not_found)?;
    Ok(Json(row_to_response(&row, false)?))
}

pub async fn regenerate_token(auth: AuthUser, State(state): State<crate::app::AppState>, Path(name): Path<String>) -> Result<Json<RegenerateTokenResponse>, ApiError> {
    let (_old_token, new_token, row) = device_db::regenerate_token_owned_by_name(state.pool(), auth.user.id, &name).await.map_err(ApiError::from_sqlx)?.ok_or_else(not_found)?;
    Ok(Json(RegenerateTokenResponse { token: new_token, device: row_to_response(&row, false)? }))
}

pub async fn delete_device(auth: AuthUser, State(state): State<crate::app::AppState>, Path(name): Path<String>) -> Result<StatusCode, ApiError> {
    let deleted = device_db::delete_owned_by_name(state.pool(), auth.user.id, &name).await.map_err(ApiError::from_sqlx)?;
    if deleted.is_none() {
        return Err(not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "device not found")
}
```

- [ ] **Step 5: Mount mutating routes**

Modify `routes/mod.rs`:

```rust
.route("/api/devices/{name}/config", axum::routing::patch(devices::patch_device_config))
.route("/api/devices/{name}/regenerate-token", post(devices::regenerate_token))
.route("/api/devices/{name}", delete(devices::delete_device))
```

- [ ] **Step 6: Run REST tests**

```bash
cargo test -p plexus-server --test m1e_devices_rest -- --nocapture
```

Expected: all REST lifecycle tests pass.

- [ ] **Step 7: Commit**

```bash
git add plexus-server/src/db/devices.rs plexus-server/src/routes/devices.rs plexus-server/src/routes/mod.rs plexus-server/tests/m1e_devices_rest.rs
git commit -m "feat: add device update regenerate and delete APIs"
```

---

### Task 4: Device Runtime and Connection Registry

**Files:**
- Create: `plexus-server/src/devices/mod.rs`
- Create: `plexus-server/src/devices/registry.rs`
- Modify: `plexus-server/src/lib.rs`
- Modify: `plexus-server/src/app.rs`
- Modify: `plexus-server/src/routes/devices.rs`

- [ ] **Step 1: Write failing registry tests**

Create `plexus-server/src/devices/registry.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    fn handle(token: &str, name: &str, id: Uuid) -> (ConnHandle, mpsc::Receiver<DeviceCommand>) {
        let (tx, rx) = mpsc::channel(4);
        (ConnHandle {
            connection_id: id,
            user_id: Uuid::now_v7(),
            token: token.to_string(),
            device_name: name.to_string(),
            connected_at: time::OffsetDateTime::now_utc(),
            last_seen: time::OffsetDateTime::now_utc(),
            tx,
        }, rx)
    }

    #[tokio::test]
    async fn insert_replaces_old_connection_and_closes_old() {
        let registry = DeviceConnectionRegistry::default();
        let (first, mut first_rx) = handle("token", "devbox", Uuid::now_v7());
        let (second, _second_rx) = handle("token", "devbox", Uuid::now_v7());
        registry.insert(first).await;
        registry.insert(second).await;
        match first_rx.recv().await.unwrap() {
            DeviceCommand::Close(close) => assert_eq!(close.code, DeviceCloseCode::Normal),
            other => panic!("expected close, got {other:?}"),
        }
        assert!(registry.is_online("token").await);
    }

    #[tokio::test]
    async fn stale_remove_does_not_remove_new_connection() {
        let registry = DeviceConnectionRegistry::default();
        let first_id = Uuid::now_v7();
        let second_id = Uuid::now_v7();
        let (first, _first_rx) = handle("token", "devbox", first_id);
        let (second, _second_rx) = handle("token", "devbox", second_id);
        registry.insert(first).await;
        registry.insert(second).await;
        registry.remove_if_current("token", first_id).await;
        assert!(registry.is_online("token").await);
        registry.remove_if_current("token", second_id).await;
        assert!(!registry.is_online("token").await);
    }
}
```

- [ ] **Step 2: Run tests and verify they fail**

```bash
cargo test -p plexus-server devices::registry::tests -- --nocapture
```

Expected: module not found.

- [ ] **Step 3: Add runtime module**

Create `plexus-server/src/devices/mod.rs`:

```rust
pub mod registry;
pub mod ws;

pub use registry::{DeviceCloseCode, DeviceCloseReason, DeviceCommand, DeviceConnectionRegistry};

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    pub ping_interval: Duration,
    pub timeout: Duration,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self { ping_interval: Duration::from_secs(30), timeout: Duration::from_secs(70) }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DeviceRuntime {
    registry: DeviceConnectionRegistry,
    heartbeat: HeartbeatConfig,
}

impl DeviceRuntime {
    pub fn registry(&self) -> &DeviceConnectionRegistry { &self.registry }
    pub fn heartbeat(&self) -> &HeartbeatConfig { &self.heartbeat }

    #[cfg(test)]
    pub fn with_heartbeat(heartbeat: HeartbeatConfig) -> Self {
        Self { registry: DeviceConnectionRegistry::default(), heartbeat }
    }
}
```

Modify `plexus-server/src/lib.rs`:

```rust
pub mod devices;
```

- [ ] **Step 4: Implement registry**

Implement `registry.rs` above the tests:

```rust
use plexus_common::WsFrame;
use std::{collections::HashMap, sync::Arc};
use time::OffsetDateTime;
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceCloseCode { Normal, Unauthorized, HeartbeatTimeout, VersionUnsupported }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCloseReason { pub code: DeviceCloseCode, pub reason: Option<String> }

#[derive(Debug, Clone)]
pub enum DeviceCommand { Frame(WsFrame), Close(DeviceCloseReason) }

#[derive(Clone)]
pub struct ConnHandle {
    pub connection_id: Uuid,
    pub user_id: Uuid,
    pub token: String,
    pub device_name: String,
    pub connected_at: OffsetDateTime,
    pub last_seen: OffsetDateTime,
    pub tx: mpsc::Sender<DeviceCommand>,
}

#[derive(Clone, Default)]
pub struct DeviceConnectionRegistry {
    inner: Arc<RwLock<HashMap<String, ConnHandle>>>,
}

impl std::fmt::Debug for DeviceConnectionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceConnectionRegistry").finish_non_exhaustive()
    }
}

impl DeviceConnectionRegistry {
    pub async fn insert(&self, handle: ConnHandle) {
        let old = self.inner.write().await.insert(handle.token.clone(), handle);
        if let Some(old) = old {
            let _ = old.tx.send(DeviceCommand::Close(DeviceCloseReason { code: DeviceCloseCode::Normal, reason: None })).await;
        }
    }

    pub async fn is_online(&self, token: &str) -> bool {
        self.inner.read().await.contains_key(token)
    }

    pub async fn close_token(&self, token: &str, reason: DeviceCloseReason) {
        if let Some(handle) = self.inner.write().await.remove(token) {
            let _ = handle.tx.send(DeviceCommand::Close(reason)).await;
        }
    }

    pub async fn remove_if_current(&self, token: &str, connection_id: Uuid) {
        let mut guard = self.inner.write().await;
        if guard.get(token).is_some_and(|handle| handle.connection_id == connection_id) {
            guard.remove(token);
        }
    }

    pub async fn send_frame(&self, token: &str, frame: WsFrame) -> bool {
        let handle = self.inner.read().await.get(token).cloned();
        if let Some(handle) = handle {
            handle.tx.send(DeviceCommand::Frame(frame)).await.is_ok()
        } else {
            false
        }
    }
}
```

- [ ] **Step 5: Add runtime to AppState**

Modify `app.rs` imports and inner state:

```rust
use crate::{chat::ChatRuntime, config::ServerConfig, devices::DeviceRuntime, openai::OpenAiRuntime, routes, workspace::WorkspaceFs};

pub struct AppStateInner {
    pub pool: PgPool,
    pub config: ServerConfig,
    pub openai: OpenAiRuntime,
    pub chat: ChatRuntime,
    pub workspace_fs: WorkspaceFs,
    pub device_runtime: DeviceRuntime,
    pub admin_config_lock: Mutex<()>,
}
```

Set `device_runtime: DeviceRuntime::default()` in constructors and add:

```rust
pub fn devices(&self) -> &DeviceRuntime {
    &self.inner.device_runtime
}
```

- [ ] **Step 6: Wire online status in REST responses**

In `routes/devices.rs`, replace hardcoded `false` with:

```rust
let online = state.devices().registry().is_online(&row.token).await;
```

Use that value for create, list, patch, regenerate responses.

- [ ] **Step 7: Run tests**

```bash
cargo test -p plexus-server devices::registry::tests -- --nocapture
cargo test -p plexus-server --test m1e_devices_rest -- --nocapture
```

Expected: registry and REST tests pass.

- [ ] **Step 8: Commit**

```bash
git add plexus-server/src/lib.rs plexus-server/src/app.rs plexus-server/src/devices plexus-server/src/routes/devices.rs
git commit -m "feat: add device connection registry"
```

---

### Task 5: WebSocket Handshake and Fake Device Client

**Files:**
- Modify: `Cargo.toml`
- Modify: `plexus-server/Cargo.toml`
- Create: `plexus-server/src/devices/ws.rs`
- Modify: `plexus-server/src/routes/mod.rs`
- Modify: `plexus-server/tests/support/mod.rs`
- Create: `plexus-server/tests/support/device_client.rs`
- Create: `plexus-server/tests/m1e_device_ws.rs`

- [ ] **Step 1: Add dependencies**

Modify workspace dependencies in `Cargo.toml`:

```toml
axum = { version = "0.8", features = ["ws"] }
futures-util = "0.3"
tokio-tungstenite = "0.27"
```

Modify `plexus-server/Cargo.toml`:

```toml
[dependencies]
futures-util.workspace = true

[dev-dependencies]
tokio-tungstenite.workspace = true
```

- [ ] **Step 2: Add running server support**

Append to `plexus-server/tests/support/mod.rs`:

```rust
pub struct RunningTestServer {
    pub app: TestApp,
    pub base_url: String,
    handle: tokio::task::JoinHandle<()>,
}

impl RunningTestServer {
    pub async fn spawn() -> Self {
        let app = TestApp::spawn().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = app.router.clone();
        let handle = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
        Self { app, base_url: format!("http://{}", addr), handle }
    }

    pub fn ws_url(&self, token: &str) -> String {
        self.base_url.replace("http://", "ws://") + "/ws/device?token=" + token
    }
}

impl Drop for RunningTestServer {
    fn drop(&mut self) { self.handle.abort(); }
}

pub mod device_client;
```

- [ ] **Step 3: Add fake device client**

Create `plexus-server/tests/support/device_client.rs`:

```rust
use futures_util::{SinkExt, StreamExt};
use plexus_common::{HelloCaps, HelloFrame, PongFrame, WsFrame, PROTOCOL_VERSION};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use uuid::Uuid;

pub struct DeviceClient { ws: WebSocketStream<MaybeTlsStream<TcpStream>> }

impl DeviceClient {
    pub async fn connect(url: &str) -> Self {
        let (ws, _) = connect_async(url).await.expect("connect device ws");
        Self { ws }
    }

    pub async fn send_hello(&mut self) -> Uuid {
        self.send_hello_with_version(PROTOCOL_VERSION).await
    }

    pub async fn send_hello_with_version(&mut self, version: &str) -> Uuid {
        let id = Uuid::now_v7();
        self.send_frame(WsFrame::Hello(HelloFrame {
            id,
            version: version.to_string(),
            client_version: "test-client".to_string(),
            os: "linux".to_string(),
            caps: HelloCaps { sandbox: "none".to_string(), exec: false, fs: "rw".to_string() },
        })).await;
        id
    }

    pub async fn send_frame(&mut self, frame: WsFrame) {
        self.ws.send(Message::Text(serde_json::to_string(&frame).unwrap().into())).await.unwrap();
    }

    pub async fn recv_frame(&mut self) -> WsFrame {
        loop {
            match self.ws.next().await.expect("ws message").expect("ws ok") {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(bytes) => self.ws.send(Message::Pong(bytes)).await.unwrap(),
                Message::Close(frame) => panic!("unexpected close: {frame:?}"),
                _ => {}
            }
        }
    }

    pub async fn recv_close_code(&mut self) -> u16 {
        loop {
            match self.ws.next().await.expect("ws message").expect("ws ok") {
                Message::Close(Some(frame)) => return frame.code.into(),
                Message::Close(None) => return 1005,
                _ => {}
            }
        }
    }

    pub async fn send_pong(&mut self, id: Uuid) {
        self.send_frame(WsFrame::Pong(PongFrame { id })).await;
    }
}
```

- [ ] **Step 4: Write failing handshake tests**

Create `plexus-server/tests/m1e_device_ws.rs`:

```rust
mod support;

use axum::http::{Method, StatusCode};
use plexus_common::WsFrame;
use serde_json::json;
use support::{RunningTestServer, device_client::DeviceClient, json_request, register_user};

#[tokio::test]
async fn valid_device_hello_receives_hello_ack_and_appears_online() {
    let server = RunningTestServer::spawn().await;
    let (jwt, user_id) = register_user(&server.app, "ws-owner@example.com").await;
    let (_, created) = json_request(server.app.router.clone(), Method::POST, "/api/devices", json!({ "name": "MacBook Pro" }), Some(&jwt)).await;
    let token = created["token"].as_str().unwrap();

    let mut client = DeviceClient::connect(&server.ws_url(token)).await;
    let hello_id = client.send_hello().await;
    match client.recv_frame().await {
        WsFrame::HelloAck(ack) => {
            assert_eq!(ack.id, hello_id);
            assert_eq!(ack.device_name, "macbook-pro");
            assert_eq!(ack.user_id, user_id);
            assert_eq!(ack.config.workspace_path, "~/plexus/workspace");
        }
        other => panic!("expected hello_ack, got {other:?}"),
    }

    let (status, listed) = json_request(server.app.router.clone(), Method::GET, "/api/devices", json!(null), Some(&jwt)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed[0]["online"], true);
}

#[tokio::test]
async fn invalid_token_closes_4401() {
    let server = RunningTestServer::spawn().await;
    let mut client = DeviceClient::connect(&server.ws_url("plexus_dev_invalid")).await;
    assert_eq!(client.recv_close_code().await, 4401);
}

#[tokio::test]
async fn protocol_mismatch_closes_4409() {
    let server = RunningTestServer::spawn().await;
    let (jwt, _) = register_user(&server.app, "version-owner@example.com").await;
    let (_, created) = json_request(server.app.router.clone(), Method::POST, "/api/devices", json!({ "name": "devbox" }), Some(&jwt)).await;
    let token = created["token"].as_str().unwrap();
    let mut client = DeviceClient::connect(&server.ws_url(token)).await;
    client.send_hello_with_version("999").await;
    assert_eq!(client.recv_close_code().await, 4409);
}
```

- [ ] **Step 5: Run tests and verify they fail**

```bash
cargo test -p plexus-server --test m1e_device_ws -- --nocapture
```

Expected: websocket route missing.

- [ ] **Step 6: Implement `/ws/device` handshake**

Create `plexus-server/src/devices/ws.rs` with token extraction, DB lookup, `hello` validation, `hello_ack`, registry insert, command writer loop, and close helper. Use `plexus_common::{HelloAckFrame, PROTOCOL_VERSION, WsFrame}` and `axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade}`. Map close codes:

```rust
fn close_code(code: DeviceCloseCode) -> u16 {
    match code {
        DeviceCloseCode::Normal => 1000,
        DeviceCloseCode::Unauthorized => 4401,
        DeviceCloseCode::HeartbeatTimeout => 4408,
        DeviceCloseCode::VersionUnsupported => 4409,
    }
}
```

During handshake, send:

```rust
WsFrame::HelloAck(HelloAckFrame {
    id: hello.id,
    device_name: row.name.clone(),
    user_id: row.user_id,
    config: crate::routes::devices::row_to_config(&row)?,
})
```

Move config conversion into `db::devices::to_device_config` before writing `ws.rs`, then call that function from both REST and WebSocket code so config serialization has one implementation.

- [ ] **Step 7: Mount WebSocket route**

Add to `routes/mod.rs`:

```rust
.route("/ws/device", get(crate::devices::ws::device_ws))
```

- [ ] **Step 8: Run handshake tests**

```bash
cargo test -p plexus-server --test m1e_device_ws -- --nocapture
```

Expected: all handshake tests pass.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml plexus-server/Cargo.toml plexus-server/src/devices/ws.rs plexus-server/src/routes/mod.rs plexus-server/tests/support/mod.rs plexus-server/tests/support/device_client.rs plexus-server/tests/m1e_device_ws.rs
git commit -m "feat: add device websocket handshake"
```

---

### Task 6: Heartbeat, Duplicate Replacement, and Revocation Close

**Files:**
- Modify: `plexus-server/src/app.rs`
- Modify: `plexus-server/src/devices/ws.rs`
- Modify: `plexus-server/src/routes/devices.rs`
- Modify: `plexus-server/tests/support/mod.rs`
- Modify: `plexus-server/tests/m1e_device_ws.rs`

- [ ] **Step 1: Add failing behavior tests**

Append to `m1e_device_ws.rs` tests for duplicate replacement, regenerate close, delete close, and missed pong close:

```rust
#[tokio::test]
async fn duplicate_connection_replaces_old_connection() {
    let server = RunningTestServer::spawn().await;
    let (jwt, _) = register_user(&server.app, "dupe-owner@example.com").await;
    let (_, created) = json_request(server.app.router.clone(), Method::POST, "/api/devices", json!({ "name": "devbox" }), Some(&jwt)).await;
    let token = created["token"].as_str().unwrap();
    let mut first = DeviceClient::connect(&server.ws_url(token)).await;
    first.send_hello().await;
    let _ = first.recv_frame().await;
    let mut second = DeviceClient::connect(&server.ws_url(token)).await;
    second.send_hello().await;
    let _ = second.recv_frame().await;
    assert_eq!(first.recv_close_code().await, 1000);
}

#[tokio::test]
async fn regenerate_closes_active_connection_4401() {
    let server = RunningTestServer::spawn().await;
    let (jwt, _) = register_user(&server.app, "revoke-owner@example.com").await;
    let (_, created) = json_request(server.app.router.clone(), Method::POST, "/api/devices", json!({ "name": "devbox" }), Some(&jwt)).await;
    let token = created["token"].as_str().unwrap();
    let mut client = DeviceClient::connect(&server.ws_url(token)).await;
    client.send_hello().await;
    let _ = client.recv_frame().await;
    let (status, _) = json_request(server.app.router.clone(), Method::POST, "/api/devices/devbox/regenerate-token", json!(null), Some(&jwt)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(client.recv_close_code().await, 4401);
}
```

Add the delete test in the same shape, calling `DELETE /api/devices/devbox` and expecting `4401`.

For heartbeat, add support for `RunningTestServer::spawn_with_heartbeat(Duration::from_millis(20), Duration::from_millis(60))`, connect a client, ignore pings, and expect close `4408`.

- [ ] **Step 2: Run tests and verify they fail**

```bash
cargo test -p plexus-server --test m1e_device_ws -- --nocapture
```

Expected: revocation and heartbeat tests fail.

- [ ] **Step 3: Add test AppState constructor for custom heartbeat**

In `app.rs`, add a `#[cfg(test)]` constructor that accepts `DeviceRuntime` and uses the same field initialization as `new_with_openai_runtime`.

In `tests/support/mod.rs`, implement `RunningTestServer::spawn_with_heartbeat(interval, timeout)` using `DeviceRuntime::with_heartbeat(HeartbeatConfig { ping_interval: interval, timeout })`.

- [ ] **Step 4: Implement heartbeat**

In `ws.rs`, after registry insert:

```rust
let heartbeat = state.devices().heartbeat().clone();
let mut interval = tokio::time::interval(heartbeat.ping_interval);
let mut last_pong = tokio::time::Instant::now();
```

Add a select branch that sends `WsFrame::Ping(PingFrame { id: Uuid::now_v7() })` and closes with `4408` when `last_pong.elapsed() > heartbeat.timeout`. Parse incoming `WsFrame::Pong(_)` to refresh `last_pong`. Parse incoming `WsFrame::Ping(ping)` and reply with `WsFrame::Pong(PongFrame { id: ping.id })`.

- [ ] **Step 5: Close active connections on regenerate/delete**

In `routes/devices.rs`, after regenerate returns `old_token`, call:

```rust
state.devices().registry().close_token(&old_token, crate::devices::DeviceCloseReason {
    code: crate::devices::DeviceCloseCode::Unauthorized,
    reason: Some("unauthorized".to_string()),
}).await;
```

After delete returns a token, call the same close logic for that token.

- [ ] **Step 6: Run behavior tests**

```bash
cargo test -p plexus-server --test m1e_device_ws -- --nocapture
cargo test -p plexus-server --test m1e_devices_rest -- --nocapture
```

Expected: all M1e tests pass.

- [ ] **Step 7: Commit**

```bash
git add plexus-server/src/app.rs plexus-server/src/devices plexus-server/src/routes/devices.rs plexus-server/tests/support/mod.rs plexus-server/tests/m1e_device_ws.rs
git commit -m "feat: handle device heartbeat and revocation"
```

---

### Task 7: Config Update Frames on REST Patch

**Files:**
- Modify: `plexus-server/src/routes/devices.rs`
- Modify: `plexus-server/tests/m1e_device_ws.rs`

- [ ] **Step 1: Add failing config update test**

Append to `m1e_device_ws.rs`:

```rust
#[tokio::test]
async fn patch_online_device_sends_config_update() {
    let server = RunningTestServer::spawn().await;
    let (jwt, _) = register_user(&server.app, "config-owner@example.com").await;
    let (_, created) = json_request(server.app.router.clone(), Method::POST, "/api/devices", json!({ "name": "devbox" }), Some(&jwt)).await;
    let token = created["token"].as_str().unwrap();
    let mut client = DeviceClient::connect(&server.ws_url(token)).await;
    client.send_hello().await;
    let _ = client.recv_frame().await;

    let (status, _) = json_request(server.app.router.clone(), Method::PATCH, "/api/devices/devbox/config", json!({ "workspace_path": "/tmp/updated-workspace" }), Some(&jwt)).await;
    assert_eq!(status, StatusCode::OK);

    match client.recv_frame().await {
        WsFrame::ConfigUpdate(update) => assert_eq!(update.config.workspace_path, "/tmp/updated-workspace"),
        other => panic!("expected config_update, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run test and verify it fails**

```bash
cargo test -p plexus-server --test m1e_device_ws patch_online_device_sends_config_update -- --nocapture
```

Expected: no `config_update` is received.

- [ ] **Step 3: Send config update after successful patch**

In `patch_device_config`, after DB update:

```rust
let config = row_to_config(&row)?;
let _sent = state.devices().registry().send_frame(
    &row.token,
    WsFrame::ConfigUpdate(plexus_common::ConfigUpdateFrame {
        id: uuid::Uuid::now_v7(),
        config,
    }),
).await;
let online = state.devices().registry().is_online(&row.token).await;
Ok(Json(row_to_response(&row, online)?))
```

A failed send does not fail the REST request; it only means the connection was stale or offline.

- [ ] **Step 4: Run tests**

```bash
cargo test -p plexus-server --test m1e_device_ws patch_online_device_sends_config_update -- --nocapture
cargo test -p plexus-server --test m1e_device_ws -- --nocapture
cargo test -p plexus-server --test m1e_devices_rest -- --nocapture
```

Expected: all M1e tests pass.

- [ ] **Step 5: Commit**

```bash
git add plexus-server/src/routes/devices.rs plexus-server/tests/m1e_device_ws.rs
git commit -m "feat: push device config updates over websocket"
```

---

### Task 8: Docs and Final Verification

**Files:**
- Modify: `docs/SCHEMA.md`
- Modify: `docs/superpowers/specs/2026-05-12-plexus-m1-living-design.md`

- [ ] **Step 1: Update docs**

Ensure `docs/SCHEMA.md` says:

```markdown
- `GET /api/devices` returns all devices for the authenticated user with full config details, derived `online`, and `token_hint`; it never returns the full token.
- M1e does not expose `GET /api/devices/{name}`. Mutating routes use `{name}` for single-resource targeting.
- `POST /api/devices/{name}/regenerate-token` preserves config and returns the new token once.
- Device online state is in memory only.
```

After all tests pass, update the M1e row in `docs/superpowers/specs/2026-05-12-plexus-m1-living-design.md` from `Planned` to `Verified`.

- [ ] **Step 2: Run focused verification**

```bash
cargo fmt --all -- --check
cargo test -p plexus-server --test m1e_devices_rest -- --nocapture
cargo test -p plexus-server --test m1e_device_ws -- --nocapture
```

Expected: all pass.

- [ ] **Step 3: Run package verification**

```bash
cargo test -p plexus-server
```

Expected: all `plexus-server` tests pass.

- [ ] **Step 4: Run workspace check**

```bash
cargo check --workspace
```

Expected: workspace compiles.

- [ ] **Step 5: Check diff hygiene**

```bash
git diff --check
git status --short
```

Expected: no whitespace errors; only intended M1e files modified.

- [ ] **Step 6: Commit docs**

```bash
git add docs/SCHEMA.md docs/superpowers/specs/2026-05-12-plexus-m1-living-design.md
git commit -m "docs: record M1e device connectivity behavior"
```

---

## Plan Self-Review

Spec coverage:

- Device REST lifecycle: Tasks 1-3.
- Slug naming: Task 1.
- Plaintext token with create/regenerate-only full return and list `token_hint`: Tasks 1-3.
- Single read API `GET /api/devices` with full details and no pagination: Task 2.
- Workspace default and explicit override: Task 2.
- In-memory online registry and stale cleanup: Task 4.
- `/ws/device`, token auth, `hello_ack`, `4401`, `4409`: Task 5.
- Duplicate replacement, heartbeat timeout, regenerate/delete `4401`: Task 6.
- REST patch `config_update`: Task 7.
- Docs and verification: Task 8.

Placeholder scan: this plan contains no unresolved placeholder markers or unspecified implementation steps.

Type consistency: `DeviceRuntime`, `DeviceConnectionRegistry`, `DeviceCommand`, `DeviceCloseReason`, `DeviceRow`, `DeviceResponse`, and `DeviceClient` are used consistently across tasks.
