pub mod registry;
pub mod ws;

pub use registry::{CloseReason, ConnHandle, DeviceRuntime};

pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;
pub const HEARTBEAT_MISSED_LIMIT: u8 = 2;
