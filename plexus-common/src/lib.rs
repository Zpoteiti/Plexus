pub mod consts;
pub mod errors;
pub mod fuzzy_match;
pub mod mcp_utils;
pub mod mime;
pub mod network;
pub mod protocol;
pub mod tool_schemas;

// Brevity re-export: `plexus_common::file_ops_schemas::read_file_schema` etc.
pub use tool_schemas::file_ops as file_ops_schemas;

// Top-level re-exports of the typed error hierarchy. Keeps
// `plexus_common::ErrorCode` / `plexus_common::ApiError` paths valid for
// downstream crates and gives the new typed errors a short path too.
pub use errors::{
    ApiError, AuthError, ErrorCode, McpError, NetworkError, PlexusError, ProtocolError, ToolError,
    WorkspaceError,
};
pub use fuzzy_match::{MatchFailure, MatchResult, find_match};
