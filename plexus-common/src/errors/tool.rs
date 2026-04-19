//! Tool-execution errors.
//!
//! Raised by the tool dispatch layer when a tool call cannot be executed or
//! fails during execution. Additional variants will be introduced by later
//! cleanup tasks as the unified tool surface lands.

use super::ErrorCode;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// Tool execution returned a non-zero exit / error result. The string
    /// carries the tool-provided message.
    #[error("execution failed: {0}")]
    ExecutionFailed(String),

    /// Tool execution exceeded its timeout.
    #[error("execution timed out after {0}s")]
    Timeout(u64),

    /// Dispatched to a client device that is not currently connected.
    #[error("device unreachable: {0}")]
    DeviceUnreachable(String),

    /// Transient failure — caller may retry. The string describes the reason.
    #[error("retriable failure: {0}")]
    Retriable(String),
}

impl ToolError {
    pub fn code(&self) -> ErrorCode {
        match self {
            ToolError::ExecutionFailed(_) => ErrorCode::ExecutionFailed,
            ToolError::Timeout(_) => ErrorCode::ToolTimeout,
            ToolError::DeviceUnreachable(_) => ErrorCode::DeviceOffline,
            ToolError::Retriable(_) => ErrorCode::ExecutionFailed,
        }
    }
}
