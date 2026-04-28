//! Integration test: spawn fake-mcp via lifecycle::spawn_mcp, exercise
//! all six McpSession methods (list_tools/resources/prompts +
//! call_tool/read_resource/get_prompt), then teardown.
//!
//! The fake-mcp fixture is a separate binary in this crate (declared as
//! [[bin]] in Cargo.toml). Cargo sets `CARGO_BIN_EXE_fake-mcp` to its
//! built path when running integration tests.

use plexus_common::mcp::lifecycle::{spawn_mcp, teardown_mcp};
use plexus_common::protocol::McpServerConfig;
use serde_json::json;
use std::collections::HashMap;

fn fake_mcp_config() -> McpServerConfig {
    McpServerConfig {
        command: vec![env!("CARGO_BIN_EXE_fake-mcp").to_string()],
        env: HashMap::new(),
        description: None,
        enabled: None,
    }
}

#[tokio::test]
async fn spawn_then_list_then_teardown() {
    let config = fake_mcp_config();
    let (session, schemas) = spawn_mcp(&config).await.expect("spawn");
    assert_eq!(schemas.tools.len(), 1, "fake-mcp advertises 1 tool");
    assert_eq!(schemas.tools[0].name, "echo");
    assert_eq!(schemas.resources.len(), 1, "fake-mcp advertises 1 resource");
    assert_eq!(schemas.resources[0].uri, "fake://fixed");
    assert_eq!(schemas.prompts.len(), 1, "fake-mcp advertises 1 prompt");
    assert_eq!(schemas.prompts[0].name, "greet");
    teardown_mcp(session).await;
}

#[tokio::test]
async fn call_tool_returns_echoed_args() {
    let config = fake_mcp_config();
    let (session, _) = spawn_mcp(&config).await.expect("spawn");
    let result = session
        .call_tool("echo", json!({"x": 42}))
        .await
        .expect("call_tool");
    assert!(
        result.contains("42"),
        "echo result should contain the arg, got: {result}"
    );
    assert!(
        result.starts_with("echoed:"),
        "echo result should be tagged, got: {result}"
    );
    teardown_mcp(session).await;
}

#[tokio::test]
async fn read_resource_returns_text() {
    let config = fake_mcp_config();
    let (session, _) = spawn_mcp(&config).await.expect("spawn");
    let result = session
        .read_resource("fake://fixed")
        .await
        .expect("read_resource");
    assert_eq!(result, "fixed-resource-content");
    teardown_mcp(session).await;
}

#[tokio::test]
async fn get_prompt_returns_joined_messages() {
    let config = fake_mcp_config();
    let (session, _) = spawn_mcp(&config).await.expect("spawn");
    let result = session
        .get_prompt("greet", json!({}))
        .await
        .expect("get_prompt");
    assert_eq!(result, "hello from greet");
    teardown_mcp(session).await;
}

#[tokio::test]
async fn spawn_with_invalid_command_fails() {
    let config = McpServerConfig {
        command: vec!["/this/binary/does/not/exist".to_string()],
        env: HashMap::new(),
        description: None,
        enabled: None,
    };
    let result = spawn_mcp(&config).await;
    assert!(result.is_err(), "expected spawn to fail for nonexistent binary");
}

#[tokio::test]
async fn spawn_with_empty_command_fails() {
    let config = McpServerConfig {
        command: vec![],
        env: HashMap::new(),
        description: None,
        enabled: None,
    };
    let result = spawn_mcp(&config).await;
    assert!(result.is_err(), "expected spawn to fail for empty command");
}
