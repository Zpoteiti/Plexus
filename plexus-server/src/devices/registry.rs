use plexus_common::protocol::WsFrame;
use std::{collections::HashMap, sync::Arc};
use time::OffsetDateTime;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Replaced,
    Unauthorized,
    HeartbeatTimeout,
}

#[derive(Clone)]
pub struct ConnHandle {
    pub token: String,
    pub user_id: Uuid,
    pub device_name: String,
    pub connected_at: OffsetDateTime,
    pub last_seen: OffsetDateTime,
    pub tx: mpsc::Sender<WsFrame>,
}

#[derive(Clone, Default)]
pub struct DeviceRuntime {
    #[allow(dead_code)]
    inner: Arc<Mutex<HashMap<String, ConnHandle>>>,
}

impl DeviceRuntime {
    pub fn new() -> Self {
        Self::default()
    }
}
