use plexus_common::ReasoningEffort;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

pub mod content;
pub mod prompt;
pub mod sse;
pub mod worker;

#[derive(Clone, Default)]
pub struct ChatRuntime {
    broker: sse::SseBroker,
    workers: Arc<Mutex<HashMap<Uuid, SessionWorkState>>>,
}

#[derive(Clone, Copy, Default)]
struct SessionWorkState {
    active: bool,
    wake_requested: bool,
    reasoning_effort: Option<Option<ReasoningEffort>>,
}

impl ChatRuntime {
    pub fn broker(&self) -> &sse::SseBroker {
        &self.broker
    }

    pub async fn enqueue_turn(&self, session_id: Uuid, effort: Option<ReasoningEffort>) -> bool {
        let mut workers = self.workers.lock().await;
        let worker = workers.entry(session_id).or_default();
        worker.reasoning_effort = Some(effort);
        if worker.active {
            worker.wake_requested = true;
            false
        } else {
            worker.active = true;
            worker.wake_requested = false;
            true
        }
    }

    pub async fn finish_or_continue_worker(&self, session_id: Uuid) -> bool {
        let mut workers = self.workers.lock().await;
        let Some(worker) = workers.get_mut(&session_id) else {
            return false;
        };
        if worker.wake_requested {
            worker.wake_requested = false;
            true
        } else {
            workers.remove(&session_id);
            false
        }
    }

    pub async fn abort_worker_start(&self, session_id: Uuid) {
        let mut workers = self.workers.lock().await;
        if workers
            .get(&session_id)
            .is_some_and(|worker| worker.active && !worker.wake_requested)
        {
            workers.remove(&session_id);
        }
    }

    pub async fn clear_observed_wake(&self, session_id: Uuid) {
        if let Some(worker) = self.workers.lock().await.get_mut(&session_id) {
            worker.wake_requested = false;
        }
    }

    pub async fn update_reasoning_effort(&self, session_id: Uuid, effort: Option<ReasoningEffort>) {
        if let Some(worker) = self.workers.lock().await.get_mut(&session_id) {
            worker.reasoning_effort = Some(effort);
        }
    }

    pub async fn reasoning_effort(&self, session_id: Uuid) -> Option<Option<ReasoningEffort>> {
        self.workers
            .lock()
            .await
            .get(&session_id)
            .and_then(|worker| worker.reasoning_effort)
    }
}

#[cfg(test)]
mod tests {
    use super::ChatRuntime;
    use plexus_common::ReasoningEffort;
    use uuid::Uuid;

    #[tokio::test]
    async fn active_worker_keeps_wake_and_reasoning_until_followup_pass() {
        let runtime = ChatRuntime::default();
        let session_id = Uuid::now_v7();

        assert!(
            runtime
                .enqueue_turn(session_id, Some(ReasoningEffort::Medium))
                .await
        );
        assert_eq!(
            runtime.reasoning_effort(session_id).await,
            Some(Some(ReasoningEffort::Medium))
        );

        assert!(
            !runtime
                .enqueue_turn(session_id, Some(ReasoningEffort::High))
                .await
        );
        assert_eq!(
            runtime.reasoning_effort(session_id).await,
            Some(Some(ReasoningEffort::High))
        );

        assert!(runtime.finish_or_continue_worker(session_id).await);
        assert_eq!(
            runtime.reasoning_effort(session_id).await,
            Some(Some(ReasoningEffort::High))
        );

        assert!(!runtime.finish_or_continue_worker(session_id).await);
        assert_eq!(runtime.reasoning_effort(session_id).await, None);
        assert!(
            runtime
                .enqueue_turn(session_id, Some(ReasoningEffort::Low))
                .await
        );
    }
}
