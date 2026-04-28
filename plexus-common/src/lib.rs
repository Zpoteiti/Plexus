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
//! # Plan 2 surface (Tools)
//!
//! - [`tools`] — Tool trait + path validation + result wrap + format helpers
//!   + 14 hardcoded tool schemas + JSON Schema arg validation.
//!
//! Plan 3 (`mcp`) extends the public surface.

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
    ConfigUpdateFrame, DeviceConfig, ErrorFrame, FsPolicy, HEADER_SIZE, HelloAckFrame, HelloCaps,
    HelloFrame, McpSchemas, McpServerConfig, PingFrame, PongFrame, PromptArgument, PromptDef,
    RegisterMcpFrame, ResourceDef, SpawnFailure, ToolCallFrame, ToolDef, ToolResultFrame,
    TransferBeginFrame, TransferDirection, TransferEndFrame, TransferProgressFrame, WsFrame,
    pack_chunk, parse_chunk,
};
pub use secrets::{DeviceToken, JwtSecret, LlmApiKey};
pub use tools::Tool;
pub use tools::result::wrap_result;
pub use tools::validate::{validate_args, validate_with};
pub use version::{PROTOCOL_VERSION, crate_version};
