use std::{collections::HashSet, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

pub mod content;
pub mod sse;

#[derive(Clone, Default)]
pub struct ChatRuntime {
    broker: sse::SseBroker,
    active_workers: Arc<Mutex<HashSet<Uuid>>>,
}

impl ChatRuntime {
    pub fn broker(&self) -> &sse::SseBroker {
        &self.broker
    }

    pub async fn try_start_worker(&self, session_id: Uuid) -> bool {
        self.active_workers.lock().await.insert(session_id)
    }

    pub async fn finish_worker(&self, session_id: Uuid) {
        self.active_workers.lock().await.remove(&session_id);
    }
}
