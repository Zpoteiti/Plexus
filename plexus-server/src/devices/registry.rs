use crate::db::devices::DeviceRow;
use plexus_common::protocol::{ConfigUpdateFrame, WsFrame};
use std::{collections::HashMap, sync::Arc, time::Duration};
use time::OffsetDateTime;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Replaced,
    Unauthorized,
    HeartbeatTimeout,
}

#[derive(Debug, Clone)]
pub enum DeviceCommand {
    Frame(WsFrame),
    Close(CloseReason),
}

#[derive(Clone)]
pub struct ConnHandle {
    pub token: String,
    pub user_id: Uuid,
    pub device_name: String,
    pub connected_at: OffsetDateTime,
    pub last_seen: OffsetDateTime,
    pub tx: mpsc::Sender<DeviceCommand>,
}

#[derive(Clone)]
struct RegistryEntry {
    generation: u64,
    handle: ConnHandle,
}

#[derive(Clone, Default)]
pub struct DeviceRuntime {
    inner: Arc<Mutex<RegistryState>>,
}

struct RegistryState {
    next_generation: u64,
    by_token: HashMap<String, RegistryEntry>,
    heartbeat_interval: Duration,
    heartbeat_missed_limit: u8,
}

impl Default for RegistryState {
    fn default() -> Self {
        Self {
            next_generation: 0,
            by_token: HashMap::new(),
            heartbeat_interval: Duration::from_secs(crate::devices::HEARTBEAT_INTERVAL_SECS),
            heartbeat_missed_limit: crate::devices::HEARTBEAT_MISSED_LIMIT,
        }
    }
}

impl DeviceRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(&self, handle: ConnHandle) -> (u64, Option<ConnHandle>) {
        let mut state = self.inner.lock().await;
        state.next_generation += 1;
        let generation = state.next_generation;
        let old = state
            .by_token
            .insert(handle.token.clone(), RegistryEntry { generation, handle });
        (generation, old.map(|entry| entry.handle))
    }

    pub async fn generation(&self, token: &str) -> Option<u64> {
        self.inner
            .lock()
            .await
            .by_token
            .get(token)
            .map(|entry| entry.generation)
    }

    pub async fn get(&self, token: &str) -> Option<ConnHandle> {
        self.inner
            .lock()
            .await
            .by_token
            .get(token)
            .map(|entry| entry.handle.clone())
    }

    pub async fn is_online(&self, token: &str) -> bool {
        self.inner.lock().await.by_token.contains_key(token)
    }

    pub async fn unregister_if_current(&self, token: &str, generation: u64) {
        let mut state = self.inner.lock().await;
        if state
            .by_token
            .get(token)
            .is_some_and(|entry| entry.generation == generation)
        {
            state.by_token.remove(token);
        }
    }

    pub async fn send(&self, token: &str, frame: WsFrame) -> bool {
        let handle = self.get(token).await;
        let Some(handle) = handle else {
            return false;
        };
        if handle.tx.send(DeviceCommand::Frame(frame)).await.is_ok() {
            return true;
        }
        self.remove_stale_sender(token, &handle.tx).await;
        false
    }

    pub async fn send_config_update(&self, row: &DeviceRow) {
        let config = crate::devices::ws::device_config_from_row(row);
        let frame = WsFrame::ConfigUpdate(ConfigUpdateFrame {
            id: Uuid::now_v7(),
            config,
        });
        let _ = self.send(&row.token, frame).await;
    }

    pub async fn close(&self, token: &str, reason: CloseReason) {
        let Some(handle) = self.get(token).await else {
            return;
        };
        match handle.tx.send(DeviceCommand::Close(reason)).await {
            Ok(()) | Err(_) => {
                self.remove_stale_sender(token, &handle.tx).await;
            }
        }
    }

    pub async fn heartbeat_config(&self) -> (Duration, u8) {
        let state = self.inner.lock().await;
        (state.heartbeat_interval, state.heartbeat_missed_limit)
    }

    pub async fn set_heartbeat_for_tests(&self, interval: Duration, missed_limit: u8) {
        let mut state = self.inner.lock().await;
        state.heartbeat_interval = interval;
        state.heartbeat_missed_limit = missed_limit;
    }

    async fn remove_stale_sender(&self, token: &str, tx: &mpsc::Sender<DeviceCommand>) {
        let mut state = self.inner.lock().await;
        if state
            .by_token
            .get(token)
            .is_some_and(|entry| entry.handle.tx.same_channel(tx))
        {
            state.by_token.remove(token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plexus_common::protocol::{PingFrame, WsFrame};

    fn handle(token: &str, name: &str) -> (ConnHandle, mpsc::Receiver<DeviceCommand>) {
        let (tx, rx) = mpsc::channel(8);
        let now = OffsetDateTime::now_utc();
        (
            ConnHandle {
                token: token.to_string(),
                user_id: Uuid::now_v7(),
                device_name: name.to_string(),
                connected_at: now,
                last_seen: now,
                tx,
            },
            rx,
        )
    }

    #[tokio::test]
    async fn replace_returns_old_handle_and_keeps_new_online() {
        let runtime = DeviceRuntime::new();
        let (old, _old_rx) = handle("t", "old");
        let (old_generation, old_replaced) = runtime.register(old).await;
        assert!(old_replaced.is_none());
        let (new, _new_rx) = handle("t", "new");
        let (new_generation, new_replaced) = runtime.register(new.clone()).await;
        assert!(new_replaced.is_some());
        assert!(new_generation > old_generation);
        assert!(runtime.is_online("t").await);
        assert_eq!(runtime.get("t").await.unwrap().device_name, "new");
    }

    #[tokio::test]
    async fn stale_cleanup_does_not_remove_replacement() {
        let runtime = DeviceRuntime::new();
        let (old, _old_rx) = handle("t", "old");
        let (old_generation, old_replaced) = runtime.register(old).await;
        assert!(old_replaced.is_none());
        let (new, _new_rx) = handle("t", "new");
        let (new_generation, new_replaced) = runtime.register(new).await;
        assert!(new_replaced.is_some());
        runtime.unregister_if_current("t", old_generation).await;
        assert_eq!(runtime.generation("t").await, Some(new_generation));
    }

    #[tokio::test]
    async fn send_frame_removes_stale_closed_channel() {
        let runtime = DeviceRuntime::new();
        let (h, rx) = handle("t", "devbox");
        drop(rx);
        runtime.register(h).await;
        let ok = runtime
            .send("t", WsFrame::Ping(PingFrame { id: Uuid::now_v7() }))
            .await;
        assert!(!ok);
        assert!(!runtime.is_online("t").await);
    }

    #[tokio::test]
    async fn close_removes_stale_closed_channel() {
        let runtime = DeviceRuntime::new();
        let (h, rx) = handle("t", "devbox");
        drop(rx);
        runtime.register(h).await;
        runtime.close("t", CloseReason::Unauthorized).await;
        assert!(!runtime.is_online("t").await);
    }

    #[tokio::test]
    async fn send_wraps_frame_command() {
        let runtime = DeviceRuntime::new();
        let (h, mut rx) = handle("t", "devbox");
        runtime.register(h).await;
        let id = Uuid::now_v7();
        let ok = runtime.send("t", WsFrame::Ping(PingFrame { id })).await;
        assert!(ok);
        assert!(matches!(
            rx.recv().await,
            Some(DeviceCommand::Frame(WsFrame::Ping(frame))) if frame.id == id
        ));
    }

    #[tokio::test]
    async fn close_sends_close_command_and_marks_offline() {
        let runtime = DeviceRuntime::new();
        let (h, mut rx) = handle("t", "devbox");
        runtime.register(h).await;
        runtime.close("t", CloseReason::Unauthorized).await;
        assert!(matches!(
            rx.recv().await,
            Some(DeviceCommand::Close(CloseReason::Unauthorized))
        ));
        assert!(!runtime.is_online("t").await);
    }
}
