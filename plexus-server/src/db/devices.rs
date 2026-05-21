use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use plexus_common::consts::DEVICE_TOKEN_PREFIX;
use rand_core::{OsRng, RngCore};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

pub const DEFAULT_WORKSPACE_PATH: &str = "~/plexus/workspace";
pub const DEFAULT_FS_POLICY: &str = "sandbox";
pub const DEFAULT_SHELL_TIMEOUT_MAX: i32 = 300;
pub const MAX_DEVICE_NAME_CHARS: usize = 64;

#[derive(Clone, sqlx::FromRow)]
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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DevicePatch {
    pub name: Option<String>,
    pub workspace_path: Option<String>,
    pub fs_policy: Option<String>,
    pub shell_timeout_max: Option<i32>,
    pub ssrf_whitelist: Option<Value>,
    pub mcp_servers: Option<Value>,
}

#[derive(sqlx::FromRow)]
struct RegeneratedDeviceRow {
    pub old_token: String,
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

impl RegeneratedDeviceRow {
    fn into_parts(self) -> (String, DeviceRow) {
        (
            self.old_token,
            DeviceRow {
                token: self.token,
                user_id: self.user_id,
                name: self.name,
                workspace_path: self.workspace_path,
                fs_policy: self.fs_policy,
                shell_timeout_max: self.shell_timeout_max,
                ssrf_whitelist: self.ssrf_whitelist,
                mcp_servers: self.mcp_servers,
                created_at: self.created_at,
            },
        )
    }
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
        return Err(format!(
            "device name must be at most {MAX_DEVICE_NAME_CHARS} characters"
        ));
    }
    Ok(out)
}

pub fn validate_fs_policy(value: &str) -> Result<String, String> {
    match value {
        "sandbox" | "unrestricted" => Ok(value.to_string()),
        _ => Err("fs_policy must be 'sandbox' or 'unrestricted'".to_string()),
    }
}

pub fn validate_shell_timeout(value: i32) -> Result<i32, String> {
    if (1..=3600).contains(&value) {
        Ok(value)
    } else {
        Err("shell_timeout_max must be between 1 and 3600".to_string())
    }
}

pub fn default_new_device(raw_name: &str) -> Result<NewDevice, String> {
    Ok(NewDevice {
        name: normalize_device_name(raw_name)?,
        workspace_path: DEFAULT_WORKSPACE_PATH.to_string(),
        fs_policy: DEFAULT_FS_POLICY.to_string(),
        shell_timeout_max: DEFAULT_SHELL_TIMEOUT_MAX,
        ssrf_whitelist: json!([]),
        mcp_servers: json!({}),
    })
}

pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    new: NewDevice,
) -> Result<DeviceRow, sqlx::Error> {
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

pub async fn list_by_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<DeviceRow>, sqlx::Error> {
    sqlx::query_as::<_, DeviceRow>(
        r#"
        SELECT token, user_id, name, workspace_path, fs_policy,
               shell_timeout_max, ssrf_whitelist, mcp_servers, created_at
        FROM devices
        WHERE user_id = $1
        ORDER BY name ASC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn find_by_user_and_name(
    pool: &PgPool,
    user_id: Uuid,
    name: &str,
) -> Result<Option<DeviceRow>, sqlx::Error> {
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

pub async fn patch(
    pool: &PgPool,
    user_id: Uuid,
    current_name: &str,
    patch: DevicePatch,
) -> Result<Option<DeviceRow>, sqlx::Error> {
    sqlx::query_as::<_, DeviceRow>(
        r#"
        UPDATE devices
        SET name = COALESCE($3::text, name),
            workspace_path = COALESCE($4::text, workspace_path),
            fs_policy = COALESCE($5::text, fs_policy),
            shell_timeout_max = COALESCE($6::integer, shell_timeout_max),
            ssrf_whitelist = COALESCE($7::jsonb, ssrf_whitelist),
            mcp_servers = COALESCE($8::jsonb, mcp_servers)
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

pub async fn regenerate_token(
    pool: &PgPool,
    user_id: Uuid,
    name: &str,
) -> Result<Option<(String, DeviceRow)>, sqlx::Error> {
    let new_token = generate_device_token();
    sqlx::query_as::<_, RegeneratedDeviceRow>(
        r#"
        WITH current AS (
            SELECT token
            FROM devices
            WHERE user_id = $1 AND name = $2
            FOR UPDATE
        )
        UPDATE devices
        SET token = $3
        FROM current
        WHERE devices.token = current.token
        RETURNING current.token AS old_token,
                  devices.token, devices.user_id, devices.name, devices.workspace_path,
                  devices.fs_policy, devices.shell_timeout_max, devices.ssrf_whitelist,
                  devices.mcp_servers, devices.created_at
        "#,
    )
    .bind(user_id)
    .bind(name)
    .bind(new_token)
    .fetch_optional(pool)
    .await
    .map(|row| row.map(RegeneratedDeviceRow::into_parts))
}

pub async fn delete_by_user_and_name(
    pool: &PgPool,
    user_id: Uuid,
    name: &str,
) -> Result<Option<DeviceRow>, sqlx::Error> {
    sqlx::query_as::<_, DeviceRow>(
        r#"
        DELETE FROM devices
        WHERE user_id = $1 AND name = $2
        RETURNING token, user_id, name, workspace_path, fs_policy,
                  shell_timeout_max, ssrf_whitelist, mcp_servers, created_at
        "#,
    )
    .bind(user_id)
    .bind(name)
    .fetch_optional(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_name_normalizes_to_slug() {
        assert_eq!(normalize_device_name("MacBook Pro").unwrap(), "macbook-pro");
        assert_eq!(
            normalize_device_name("John's iPhone 6S").unwrap(),
            "johns-iphone-6s"
        );
        assert_eq!(normalize_device_name("lab_pc_01").unwrap(), "lab-pc-01");
        assert_eq!(
            normalize_device_name("lab--machine").unwrap(),
            "lab-machine"
        );
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
        assert_eq!(
            token_hint("plexus_dev_abcdefghijklmnopqrstuvwxyz"),
            "plexus_dev_...wxyz"
        );
    }

    #[test]
    fn generated_token_has_device_prefix_and_entropy() {
        let token = generate_device_token();
        assert!(token.starts_with(plexus_common::consts::DEVICE_TOKEN_PREFIX));
        assert!(token.len() > plexus_common::consts::DEVICE_TOKEN_PREFIX.len() + 32);
    }
}
