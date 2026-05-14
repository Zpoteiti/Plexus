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
    ChatRole, ContentBlock, DeviceConfig, FsPolicy, ImageUrlBlock, McpSchemas, McpServerConfig,
    PromptArgument, PromptDef, ReasoningEffort, ResourceDef, ToolDef, contains_image, strip_images,
};
