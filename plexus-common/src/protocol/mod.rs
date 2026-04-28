//! WebSocket protocol structs. See `docs/PROTOCOL.md`.

pub mod frames;
pub mod transfer;
pub mod types;

pub use frames::WsFrame;
pub use types::{DeviceConfig, FsPolicy, McpSchemas, McpServerConfig, PromptArgument, PromptDef, ResourceDef, ToolDef};
