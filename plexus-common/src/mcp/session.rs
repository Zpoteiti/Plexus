//! `McpSession` — thin wrapper around `rmcp::RunningService` that exposes
//! exactly the six methods Plexus consumers need, returning crate-typed
//! errors (`McpError`) instead of leaking rmcp's error type.
//!
//! See ADR-047 for the wrapping rationale. Tests live in
//! `tests/mcp_lifecycle.rs` — they spawn the `fake-mcp` fixture to exercise
//! the full client/server protocol.

use crate::errors::McpError;
use crate::protocol::{PromptArgument, PromptDef, ResourceDef, ToolDef};
use rmcp::model::{
    CallToolRequestParams, GetPromptRequestParams, PromptMessageContent, RawContent,
    ReadResourceRequestParams, ResourceContents,
};
use serde_json::{Map, Value};
use std::fmt::Display;

/// MCP client session. Hides the underlying rmcp running service.
///
/// Every method returns `McpError` on failure — the inner rmcp error
/// types do not leak through.
pub struct McpSession {
    inner: rmcp::service::RunningService<rmcp::RoleClient, ()>,
}

fn session_err(detail: impl Display) -> McpError {
    McpError::CallFailed {
        server: "session".to_string(),
        detail: detail.to_string(),
    }
}

/// Convert a JSON value into the owned `Map` rmcp expects, moving the
/// inner map when the value is already an object (no clone) and returning
/// `None` for null/non-object values.
fn args_to_map(args: Value) -> Option<Map<String, Value>> {
    match args {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

impl McpSession {
    /// Construct from a started rmcp service. Used by `lifecycle::spawn_mcp`.
    pub(crate) fn from_running(inner: rmcp::service::RunningService<rmcp::RoleClient, ()>) -> Self {
        Self { inner }
    }

    /// List the tools advertised by the MCP server.
    pub async fn list_tools(&self) -> Result<Vec<ToolDef>, McpError> {
        let response = self
            .inner
            .list_tools(None)
            .await
            .map_err(|e| session_err(format!("list_tools: {e}")))?;
        Ok(response
            .tools
            .into_iter()
            .map(|t| ToolDef {
                name: t.name.to_string(),
                input_schema: Value::Object(t.input_schema.as_ref().clone()),
                description: t.description.map(|s| s.to_string()),
            })
            .collect())
    }

    /// List the resources advertised by the MCP server.
    pub async fn list_resources(&self) -> Result<Vec<ResourceDef>, McpError> {
        let response = self
            .inner
            .list_resources(None)
            .await
            .map_err(|e| session_err(format!("list_resources: {e}")))?;
        Ok(response
            .resources
            .into_iter()
            .map(|r| ResourceDef {
                name: r.name.clone(),
                uri: r.uri.clone(),
                description: r.description.clone(),
                mime_type: r.mime_type.clone(),
            })
            .collect())
    }

    /// List the prompts advertised by the MCP server.
    pub async fn list_prompts(&self) -> Result<Vec<PromptDef>, McpError> {
        let response = self
            .inner
            .list_prompts(None)
            .await
            .map_err(|e| session_err(format!("list_prompts: {e}")))?;
        Ok(response
            .prompts
            .into_iter()
            .map(|p| PromptDef {
                name: p.name.clone(),
                arguments: p
                    .arguments
                    .unwrap_or_default()
                    .into_iter()
                    .map(|a| PromptArgument {
                        name: a.name,
                        description: a.description,
                        required: a.required.unwrap_or(false),
                    })
                    .collect(),
                description: p.description.clone(),
            })
            .collect())
    }

    /// Call a tool. Returns the tool result content concatenated as text.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<String, McpError> {
        let mut params = CallToolRequestParams::new(name.to_string());
        params.arguments = args_to_map(args);
        let response = self
            .inner
            .call_tool(params)
            .await
            .map_err(|e| session_err(format!("call_tool {name}: {e}")))?;
        Ok(content_blocks_to_string(response.content))
    }

    /// Read a resource. Returns its text content.
    pub async fn read_resource(&self, uri: &str) -> Result<String, McpError> {
        let response = self
            .inner
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .map_err(|e| session_err(format!("read_resource {uri}: {e}")))?;
        Ok(resource_contents_to_string(response.contents))
    }

    /// Get a prompt with arguments. Returns the rendered prompt messages
    /// joined with `"\n"` per ADR-048's prompt-output stringify convention.
    /// Empty result → `"(no output)"`.
    pub async fn get_prompt(&self, name: &str, args: Value) -> Result<String, McpError> {
        let mut params = GetPromptRequestParams::new(name.to_string());
        params.arguments = args_to_map(args);
        let response = self
            .inner
            .get_prompt(params)
            .await
            .map_err(|e| session_err(format!("get_prompt {name}: {e}")))?;
        Ok(prompt_messages_to_string(response.messages))
    }

    /// Cancel and tear down the session. Used by `lifecycle::teardown_mcp`.
    pub(crate) async fn cancel(self) {
        if let Err(e) = self.inner.cancel().await {
            eprintln!("[plexus-common] mcp cancel error: {e}");
        }
    }
}

/// Concatenate rmcp content blocks (text/image/resource) to a single string.
/// Non-text blocks are stringified via Debug.
fn content_blocks_to_string(blocks: Vec<rmcp::model::Content>) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(blocks.len());
    for block in blocks {
        match block.raw {
            RawContent::Text(t) => parts.push(t.text),
            other => parts.push(format!("{other:?}")),
        }
    }
    parts.join("\n")
}

/// Concatenate read-resource contents to a single string.
fn resource_contents_to_string(contents: Vec<ResourceContents>) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(contents.len());
    for content in contents {
        match content {
            ResourceContents::TextResourceContents { text, .. } => parts.push(text),
            ResourceContents::BlobResourceContents { blob, .. } => parts.push(blob),
        }
    }
    parts.join("\n")
}

/// Stringify a list of PromptMessage per ADR-048 (`get_prompt` output).
///
/// Joins text content with `"\n"`. Non-text content stringified via Debug.
/// Empty list → `"(no output)"`.
fn prompt_messages_to_string(messages: Vec<rmcp::model::PromptMessage>) -> String {
    if messages.is_empty() {
        return "(no output)".to_string();
    }
    let mut parts: Vec<String> = Vec::with_capacity(messages.len());
    for msg in messages {
        match msg.content {
            PromptMessageContent::Text { text } => parts.push(text),
            other => parts.push(format!("{other:?}")),
        }
    }
    parts.join("\n")
}
