pub mod consts;
pub mod errors;
pub mod fuzzy_match;
pub mod mcp_utils;
pub mod mime;
pub mod protocol;

// Top-level re-exports of the typed error hierarchy. Keeps
// `plexus_common::ErrorCode` / `plexus_common::ApiError` paths valid for
// downstream crates and gives the new typed errors a short path too.
pub use errors::{
    ApiError, AuthError, ErrorCode, McpError, PlexusError, ProtocolError, ToolError, WorkspaceError,
};
pub use fuzzy_match::{MatchFailure, MatchResult, find_match};
