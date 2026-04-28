//! Shared types, errors, protocol, and tool infrastructure for Plexus.
//!
//! See `docs/superpowers/specs/2026-04-28-plexus-m0-design.md` for the full
//! design and `docs/DECISIONS.md` for cross-cutting architecture decisions.
//!
//! # Plan 1 surface (Foundation + Protocol)
//!
//! - [`consts`] — wire-level reserved string constants.
//! - [`version`] — `PROTOCOL_VERSION` + `crate_version()`.
//! - [`secrets`] — redacting newtypes for tokens / API keys.
//! - [`errors`] — typed error enums + `ErrorCode` + `Code` trait.
//! - [`protocol`] — WS frame types + binary transfer header.
//!
//! Plans 2 (`tools`) and 3 (`mcp`) extend the public surface.

pub mod consts;
pub mod errors;
pub mod protocol;
pub mod secrets;
pub mod tools;
pub mod version;

// Top-level re-exports for ergonomic access.
pub use errors::{
    AuthError, Code, ErrorCode, McpError, NetworkError, ProtocolError, ToolError, WorkspaceError,
};
pub use protocol::{
    DeviceConfig, FsPolicy, McpSchemas, McpServerConfig, PromptArgument, PromptDef, ResourceDef,
    ToolDef, WsFrame,
};
pub use secrets::{DeviceToken, JwtSecret, LlmApiKey};
pub use version::{PROTOCOL_VERSION, crate_version};
