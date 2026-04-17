//! Server configuration from env vars + DB-stored LLM config.

use serde::{Deserialize, Serialize};

/// Configuration loaded from environment variables at startup.
/// All required fields panic if missing — fail fast.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub database_url: String,
    pub admin_token: String,
    pub jwt_secret: String,
    pub server_port: u16,
    pub gateway_ws_url: String,
    pub gateway_token: String,
    pub workspace_root: String,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        Self {
            database_url: env_required("DATABASE_URL"),
            admin_token: env_required("ADMIN_TOKEN"),
            jwt_secret: env_required("JWT_SECRET"),
            server_port: env_required("SERVER_PORT")
                .parse()
                .expect("SERVER_PORT must be a number"),
            gateway_ws_url: env_required("PLEXUS_GATEWAY_WS_URL"),
            gateway_token: env_required("PLEXUS_GATEWAY_TOKEN"),
            workspace_root: std::env::var("PLEXUS_WORKSPACE_ROOT").unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                format!("{home}/.plexus/workspace")
            }),
        }
    }

    /// TEMPORARY: returns `{workspace_root}/{user_id}/skills`.
    /// Removed in Task A-17 once all callers have migrated.
    pub fn legacy_skills_dir_for_user(&self, user_id: &str) -> String {
        format!("{}/{user_id}/skills", self.workspace_root)
    }
}

fn env_required(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} must be set"))
}

/// LLM provider config stored in system_config table. Hot-reloadable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub api_base: String,
    pub model: String,
    pub api_key: String,
    pub context_window: u32,
}
