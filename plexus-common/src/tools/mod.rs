//! Shared tool infrastructure. See ADR-038, ADR-077, ADR-095.
//!
//! - [`result`] — wrap_result() for the [untrusted tool result]: prefix.
//! - [`path`] — resolve_in_workspace() for the file-tool jail.
//! - [`format`] — line-numbered output and head-only truncation helpers.
//! - [`schemas`] — hardcoded JSON schemas for the 14 first-class tools.
//! - [`validate`] — JSON Schema validation for tool_call args.

pub mod format;
pub mod path;
pub mod result;
pub mod schemas;
pub mod validate;

use crate::errors::ToolError;
use serde_json::Value;

/// The Tool trait — every tool the agent can dispatch implements this.
///
/// Per ADR-077:
/// - `name()`: the wrapped tool name (e.g. "read_file" or "mcp_google_search").
/// - `schema()`: JSON Schema describing accepted args (matches one of the
///   constants in [`schemas`] for built-in tools).
/// - `max_output_chars()`: result-content cap before truncation. Defaults
///   to 16,000 (ADR-076); per-tool override via custom impl.
/// - `execute()`: dispatch the tool with parsed args, returning the raw
///   result string. The dispatcher wraps the result with [`result::wrap_result`]
///   before emitting the `tool_result` block.
///
/// Implementors hold their own context as struct fields (e.g. a server
/// `ReadFileTool` would hold `Arc<WorkspaceFs>`).
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    /// The wrapped tool name as the agent sees it.
    fn name(&self) -> &str;

    /// JSON Schema for accepted args.
    fn schema(&self) -> &Value;

    /// Maximum characters in the raw result before head-only truncation
    /// (ADR-076). Default 16,000. Override for tools with larger outputs
    /// (e.g. `read_file` overrides to 128,000).
    fn max_output_chars(&self) -> usize {
        16_000
    }

    /// Dispatch with the agent-supplied args. Returns the raw result string.
    /// The dispatcher wraps it via [`result::wrap_result`] before sending.
    async fn execute(&self, args: Value) -> Result<String, ToolError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ToolError;
    use serde_json::{Value, json};

    /// Minimal Tool impl for trait-shape testing.
    struct EchoTool {
        schema: Value,
    }

    #[async_trait::async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn schema(&self) -> &Value {
            &self.schema
        }

        async fn execute(&self, args: Value) -> Result<String, ToolError> {
            Ok(args.to_string())
        }
    }

    #[tokio::test]
    async fn tool_trait_dispatches_execute() {
        let tool = EchoTool {
            schema: json!({"name": "echo", "description": "test", "input_schema": {}}),
        };
        assert_eq!(tool.name(), "echo");
        assert_eq!(tool.schema()["name"], "echo");
        let result = tool.execute(json!({"x": 1})).await.unwrap();
        assert!(result.contains("\"x\""));
    }

    #[tokio::test]
    async fn tool_default_max_output_chars_is_16k() {
        let tool = EchoTool {
            schema: json!({"name": "echo", "description": "test", "input_schema": {}}),
        };
        assert_eq!(tool.max_output_chars(), 16_000);
    }
}
