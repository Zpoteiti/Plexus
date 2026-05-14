use plexus_common::ReasoningEffort;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::Mutex;
use uuid::Uuid;

pub mod content;
pub mod prompt;
pub mod sse;
pub mod worker;

#[derive(Clone, Default)]
pub struct ChatRuntime {
    broker: sse::SseBroker,
    active_workers: Arc<Mutex<HashSet<Uuid>>>,
    reasoning_efforts: Arc<Mutex<HashMap<Uuid, ReasoningEffort>>>,
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
        self.reasoning_efforts.lock().await.remove(&session_id);
    }

    pub async fn set_reasoning_effort(&self, session_id: Uuid, effort: ReasoningEffort) {
        self.reasoning_efforts
            .lock()
            .await
            .insert(session_id, effort);
    }

    pub async fn reasoning_effort(&self, session_id: Uuid) -> Option<ReasoningEffort> {
        self.reasoning_efforts
            .lock()
            .await
            .get(&session_id)
            .copied()
    }
}
