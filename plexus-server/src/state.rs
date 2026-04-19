//! Global application state shared across all handlers via Arc.

use crate::bus::OutboundEvent;
use crate::config::{LlmConfig, ServerConfig};
use crate::session::SessionHandle;
use axum::extract::ws::Message;
use dashmap::DashMap;
use futures_util::stream::SplitSink;
use plexus_common::protocol::ToolExecutionResult;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::time::Instant;
use tokio::sync::{Mutex, RwLock, Semaphore, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub type WsSink = SplitSink<axum::extract::ws::WebSocket, Message>;

pub struct DeviceConnection {
    pub user_id: String,
    pub device_name: String,
    pub sink: Arc<Mutex<WsSink>>,
    pub last_seen: Arc<AtomicI64>,
    pub tools: Vec<String>,
}

pub struct AppState {
    pub db: PgPool,
    pub config: ServerConfig,

    // Hot-reloadable LLM config
    pub llm_config: Arc<RwLock<Option<LlmConfig>>>,

    // Online device routing: "user_id:device_name" -> connection
    pub devices: DashMap<String, DeviceConnection>,
    // user_id -> [device_keys]
    pub devices_by_user: DashMap<String, Vec<String>>,

    // Tool request/response matching: device_key -> { request_id -> sender }
    pub pending: DashMap<String, DashMap<String, oneshot::Sender<ToolExecutionResult>>>,

    // Per-user tool schema cache (Arc to avoid deep-cloning JSON on cache hits)
    pub tool_schema_cache: DashMap<String, Arc<Vec<Value>>>,

    // Rate limiting: user_id -> (remaining, last_refill)
    pub rate_limiter: DashMap<String, (u32, Instant)>,
    pub rate_limit_config: Arc<RwLock<u32>>,

    // Default soul cache
    pub default_soul: Arc<RwLock<Option<String>>>,

    // Dream prompt templates (loaded once at boot from system_config / embedded fallback)
    pub dream_phase1_prompt: Arc<str>,
    pub dream_phase2_prompt: Arc<str>,

    // Heartbeat Phase 1 prompt (loaded once at boot from system_config / embedded fallback)
    pub heartbeat_phase1_prompt: Arc<str>,

    // Session handles
    pub sessions: DashMap<String, Arc<SessionHandle>>,

    // Web fetch concurrency limit
    pub web_fetch_semaphore: Arc<Semaphore>,

    // Outbound event channel (agent loop -> channel handlers)
    pub outbound_tx: mpsc::Sender<OutboundEvent>,

    // Shared HTTP client (connection pooling for LLM calls + skill install)
    pub http_client: reqwest::Client,

    // Pre-configured HTTP client for web_fetch (custom timeout/redirect)
    pub web_fetch_client: reqwest::Client,

    // Server-side MCP manager (admin-configured)
    pub server_mcp: Arc<RwLock<crate::server_mcp::ServerMcpManager>>,

    // Gateway WebSocket sink (for outbound delivery)
    pub gateway_sink: RwLock<
        Option<
            Arc<
                Mutex<
                    futures_util::stream::SplitSink<
                        tokio_tungstenite::WebSocketStream<
                            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                        >,
                        tokio_tungstenite::tungstenite::Message,
                    >,
                >,
            >,
        >,
    >,

    // Shutdown signal — cancelled on SIGINT/SIGTERM; background tasks watch it.
    pub shutdown: CancellationToken,

    // Per-user disk quota tracking.
    pub quota: std::sync::Arc<crate::workspace::QuotaCache>,

    // Per-user in-memory skills cache.
    pub skills_cache: std::sync::Arc<crate::skills_cache::SkillsCache>,

    // Workspace filesystem service (path checks, quota, skills-cache invalidation).
    pub workspace_fs: std::sync::Arc<crate::workspace::WorkspaceFs>,
}

impl AppState {
    pub fn device_key(user_id: &str, device_name: &str) -> String {
        format!("{user_id}:{device_name}")
    }
}

#[cfg(test)]
impl AppState {
    /// Build a minimal AppState with a specified workspace root.
    /// Does NOT connect to a real DB or start any background tasks.
    /// Used by server-tool unit tests that only need file I/O paths.
    ///
    /// WARNING: `db` is `PgPool::connect_lazy` with an invalid URL — any code path
    /// that actually queries the DB will fail at runtime (not compile-time). Tests
    /// that need DB access should use `#[sqlx::test]` and build AppState explicitly.
    pub fn test_minimal(workspace_root: &std::path::Path) -> std::sync::Arc<Self> {
        use tokio::sync::mpsc;

        let config = crate::config::ServerConfig {
            database_url: "postgres://invalid".into(),
            admin_token: "test".into(),
            jwt_secret: "test".into(),
            server_port: 0,
            gateway_ws_url: "ws://invalid".into(),
            gateway_token: "test".into(),
            workspace_root: workspace_root.to_string_lossy().into_owned(),
        };

        let (outbound_tx, _outbound_rx) = mpsc::channel::<crate::bus::OutboundEvent>(1);

        Self::build_test_state(config, outbound_tx, 1024 * 1024)
    }

    /// Same as `test_minimal` but returns the outbound receiver so tests can
    /// observe `OutboundEvent`s published by the tool under test.
    pub fn test_minimal_with_outbound(
        workspace_root: &std::path::Path,
    ) -> (
        std::sync::Arc<Self>,
        tokio::sync::mpsc::Receiver<crate::bus::OutboundEvent>,
    ) {
        use tokio::sync::mpsc;

        let config = crate::config::ServerConfig {
            database_url: "postgres://invalid".into(),
            admin_token: "test".into(),
            jwt_secret: "test".into(),
            server_port: 0,
            gateway_ws_url: "ws://invalid".into(),
            gateway_token: "test".into(),
            workspace_root: workspace_root.to_string_lossy().into_owned(),
        };

        let (outbound_tx, outbound_rx) = mpsc::channel::<crate::bus::OutboundEvent>(16);
        (
            Self::build_test_state(config, outbound_tx, 1024 * 1024),
            outbound_rx,
        )
    }

    /// Same as `test_minimal` but with an explicit quota size.
    /// Use when tests need to exercise quota boundaries without GBs of memory.
    pub fn test_minimal_with_quota(
        workspace_root: &std::path::Path,
        quota_bytes: u64,
    ) -> std::sync::Arc<Self> {
        use tokio::sync::mpsc;

        let config = crate::config::ServerConfig {
            database_url: "postgres://invalid".into(),
            admin_token: "test".into(),
            jwt_secret: "test".into(),
            server_port: 0,
            gateway_ws_url: "ws://invalid".into(),
            gateway_token: "test".into(),
            workspace_root: workspace_root.to_string_lossy().into_owned(),
        };

        let (outbound_tx, _outbound_rx) = mpsc::channel::<crate::bus::OutboundEvent>(1);

        Self::build_test_state(config, outbound_tx, quota_bytes)
    }

    /// Build a minimal AppState backed by a real PgPool.
    /// Used by DB-integrated tests that need to call tool functions against an actual
    /// database. Requires `DATABASE_URL` — gate callers with `#[ignore]`.
    pub fn test_with_pool(
        pool: sqlx::PgPool,
        workspace_root: &std::path::Path,
    ) -> std::sync::Arc<Self> {
        use tokio::sync::{RwLock, Semaphore, mpsc};
        use tokio_util::sync::CancellationToken;

        let config = crate::config::ServerConfig {
            database_url: "postgres://test".into(),
            admin_token: "test".into(),
            jwt_secret: "test".into(),
            server_port: 0,
            gateway_ws_url: "ws://invalid".into(),
            gateway_token: "test".into(),
            workspace_root: workspace_root.to_string_lossy().into_owned(),
        };

        let (outbound_tx, _outbound_rx) = mpsc::channel::<crate::bus::OutboundEvent>(16);

        let quota = std::sync::Arc::new(crate::workspace::QuotaCache::new(1024 * 1024));
        let skills_cache = std::sync::Arc::new(crate::skills_cache::SkillsCache::new());
        let workspace_fs = std::sync::Arc::new(crate::workspace::WorkspaceFs::new(
            std::path::PathBuf::from(workspace_root.to_string_lossy().as_ref()),
            quota.clone(),
            skills_cache.clone(),
        ));

        std::sync::Arc::new(AppState {
            db: pool,
            config,
            llm_config: std::sync::Arc::new(RwLock::new(None)),
            devices: Default::default(),
            devices_by_user: Default::default(),
            pending: Default::default(),
            tool_schema_cache: Default::default(),
            rate_limiter: Default::default(),
            rate_limit_config: std::sync::Arc::new(RwLock::new(0)),
            default_soul: std::sync::Arc::new(RwLock::new(None)),
            dream_phase1_prompt: Arc::from(
                include_str!("../templates/prompts/dream_phase1.md"),
            ),
            dream_phase2_prompt: Arc::from(
                include_str!("../templates/prompts/dream_phase2.md"),
            ),
            heartbeat_phase1_prompt: Arc::from(
                include_str!("../templates/prompts/heartbeat_phase1.md"),
            ),
            sessions: Default::default(),
            web_fetch_semaphore: std::sync::Arc::new(Semaphore::new(1)),
            http_client: reqwest::Client::new(),
            web_fetch_client: reqwest::Client::new(),
            server_mcp: std::sync::Arc::new(
                RwLock::new(crate::server_mcp::ServerMcpManager::new()),
            ),
            gateway_sink: RwLock::new(None),
            outbound_tx,
            shutdown: CancellationToken::new(),
            quota,
            skills_cache,
            workspace_fs,
        })
    }

    fn build_test_state(
        config: crate::config::ServerConfig,
        outbound_tx: tokio::sync::mpsc::Sender<crate::bus::OutboundEvent>,
        quota_bytes: u64,
    ) -> std::sync::Arc<Self> {
        use tokio::sync::{RwLock, Semaphore};
        use tokio_util::sync::CancellationToken;

        let quota = std::sync::Arc::new(crate::workspace::QuotaCache::new(quota_bytes));
        let skills_cache = std::sync::Arc::new(crate::skills_cache::SkillsCache::new());
        let workspace_fs = std::sync::Arc::new(crate::workspace::WorkspaceFs::new(
            std::path::PathBuf::from(&config.workspace_root),
            quota.clone(),
            skills_cache.clone(),
        ));

        std::sync::Arc::new(AppState {
            db: sqlx::PgPool::connect_lazy("postgres://invalid").unwrap(),
            config,
            llm_config: std::sync::Arc::new(RwLock::new(None)),
            devices: Default::default(),
            devices_by_user: Default::default(),
            pending: Default::default(),
            tool_schema_cache: Default::default(),
            rate_limiter: Default::default(),
            rate_limit_config: std::sync::Arc::new(RwLock::new(0)),
            default_soul: std::sync::Arc::new(RwLock::new(None)),
            dream_phase1_prompt: std::sync::Arc::from(
                include_str!("../templates/prompts/dream_phase1.md"),
            ),
            dream_phase2_prompt: std::sync::Arc::from(
                include_str!("../templates/prompts/dream_phase2.md"),
            ),
            heartbeat_phase1_prompt: std::sync::Arc::from(
                include_str!("../templates/prompts/heartbeat_phase1.md"),
            ),
            sessions: Default::default(),
            web_fetch_semaphore: std::sync::Arc::new(Semaphore::new(1)),
            http_client: reqwest::Client::new(),
            web_fetch_client: reqwest::Client::new(),
            server_mcp: std::sync::Arc::new(
                RwLock::new(crate::server_mcp::ServerMcpManager::new()),
            ),
            gateway_sink: RwLock::new(None),
            outbound_tx,
            shutdown: CancellationToken::new(),
            quota,
            skills_cache,
            workspace_fs,
        })
    }
}
