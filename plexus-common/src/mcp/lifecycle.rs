//! MCP lifecycle helpers per ADR-105.
//!
//! `spawn_mcp` boots the rmcp subprocess, performs the client handshake,
//! lists tools/resources/prompts, and returns an `McpSession` plus the
//! collected `McpSchemas`. Bounded by a 30-second startup timeout.
//!
//! `teardown_mcp` cancels the running service cleanly.
//!
//! Tests live in `tests/mcp_lifecycle.rs` using the `fake-mcp` fixture
//! binary to exercise the full client/server flow.

use crate::errors::McpError;
use crate::mcp::session::McpSession;
use crate::protocol::{McpSchemas, McpServerConfig};
use rmcp::{ServiceExt, transport::TokioChildProcess};
use std::time::Duration;
use tokio::process::Command;

/// Maximum startup time per ADR-105.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(30);

/// Spawn an MCP server subprocess, perform the rmcp handshake, list its
/// tools / resources / prompts, and return the session + schemas.
///
/// Bounded by 30 seconds (`SPAWN_TIMEOUT`). On timeout or any other
/// failure during startup, returns `McpError::SpawnFailed` with detail.
pub async fn spawn_mcp(config: &McpServerConfig) -> Result<(McpSession, McpSchemas), McpError> {
    let Some(server_label) = config.command.first().cloned() else {
        return Err(McpError::SpawnFailed {
            server: "spawn".to_string(),
            detail: "empty command argv".to_string(),
        });
    };

    tokio::time::timeout(SPAWN_TIMEOUT, spawn_inner(config, &server_label))
        .await
        .map_err(|_| McpError::SpawnFailed {
            server: server_label.clone(),
            detail: format!("startup timeout after {}s", SPAWN_TIMEOUT.as_secs()),
        })?
}

async fn spawn_inner(
    config: &McpServerConfig,
    server_label: &str,
) -> Result<(McpSession, McpSchemas), McpError> {
    let mut cmd = Command::new(&config.command[0]);
    if config.command.len() > 1 {
        cmd.args(&config.command[1..]);
    }
    for (k, v) in &config.env {
        cmd.env(k, v);
    }

    let transport = TokioChildProcess::new(cmd).map_err(|e| McpError::SpawnFailed {
        server: server_label.to_string(),
        detail: format!("subprocess transport: {e}"),
    })?;

    // No-op handler: Plexus doesn't act on server-initiated requests.
    let running =
        ().serve(transport)
            .await
            .map_err(|e| McpError::SpawnFailed {
                server: server_label.to_string(),
                detail: format!("rmcp handshake: {e}"),
            })?;

    let session = McpSession::from_running(running);

    let (tools, resources, prompts) = tokio::try_join!(
        session.list_tools(),
        session.list_resources(),
        session.list_prompts(),
    )?;

    let schemas = McpSchemas {
        server_name: server_label.to_string(),
        tools,
        resources,
        prompts,
    };

    Ok((session, schemas))
}

/// Cancel the running session and reap the subprocess.
pub async fn teardown_mcp(session: McpSession) {
    session.cancel().await;
}
