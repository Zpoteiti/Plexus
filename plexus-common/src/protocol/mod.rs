//! WebSocket protocol structs. See `docs/PROTOCOL.md`.

pub mod frames;
pub mod transfer;
pub mod types;

pub use frames::{
    ConfigUpdateFrame, ErrorFrame, HelloAckFrame, HelloCaps, HelloFrame, PingFrame, PongFrame,
    RegisterMcpFrame, SpawnFailure, ToolCallFrame, ToolResultFrame, TransferBeginFrame,
    TransferDirection, TransferEndFrame, TransferProgressFrame, WsFrame,
};
pub use transfer::{HEADER_SIZE, pack_chunk, parse_chunk};
pub use types::{
    DeviceConfig, FsPolicy, McpSchemas, McpServerConfig, PromptArgument, PromptDef, ResourceDef,
    ToolDef,
};
