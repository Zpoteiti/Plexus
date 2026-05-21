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

    pub async fn is_online(&self, token: &str) -> bool {
        self.inner.lock().await.contains_key(token)
    }

    pub async fn send_config_update(&self, _row: &crate::db::devices::DeviceRow) {}

    pub async fn close(&self, _token: &str, _reason: CloseReason) {}
}
