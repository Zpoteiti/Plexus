use crate::db::messages::Message;
use axum::response::sse::Event;
use std::convert::Infallible;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, broadcast};
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct SseBroker {
    inner: Arc<Mutex<HashMap<Uuid, broadcast::Sender<Message>>>>,
}

impl SseBroker {
    pub async fn subscribe(&self, session_id: Uuid) -> broadcast::Receiver<Message> {
        self.sender(session_id).await.subscribe()
    }

    pub async fn broadcast(&self, message: Message) {
        let session_id = message.session_id;
        let sender = {
            let mut inner = self.inner.lock().await;
            let Some(sender) = inner.get(&session_id).cloned() else {
                return;
            };
            if sender.receiver_count() == 0 {
                inner.remove(&session_id);
                return;
            }
            sender
        };

        if sender.send(message).is_err() {
            self.remove_if_idle(session_id).await;
        }
    }

    async fn sender(&self, session_id: Uuid) -> broadcast::Sender<Message> {
        let mut inner = self.inner.lock().await;
        inner
            .entry(session_id)
            .or_insert_with(|| broadcast::channel(256).0)
            .clone()
    }

    async fn remove_if_idle(&self, session_id: Uuid) {
        let mut inner = self.inner.lock().await;
        if inner
            .get(&session_id)
            .is_some_and(|sender| sender.receiver_count() == 0)
        {
            inner.remove(&session_id);
        }
    }
}

pub fn message_event(message: &Message) -> Result<Event, Infallible> {
    Ok(Event::default()
        .event("message")
        .id(message.id.to_string())
        .json_data(message)
        .expect("message serializes for SSE"))
}

pub fn history_end_event() -> Result<Event, Infallible> {
    Ok(Event::default().event("history_end").data("{}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use time::OffsetDateTime;

    fn message(session_id: Uuid) -> Message {
        Message {
            id: Uuid::now_v7(),
            session_id,
            role: "user".to_string(),
            content: json!([{"type": "text", "text": "hello"}]),
            reasoning_content: None,
            is_compaction_summary: false,
            created_at: OffsetDateTime::now_utc(),
        }
    }

    async fn channel_count(broker: &SseBroker) -> usize {
        broker.inner.lock().await.len()
    }

    #[tokio::test]
    async fn broadcast_without_subscribers_does_not_allocate_channel() {
        let broker = SseBroker::default();
        let session_id = Uuid::now_v7();

        broker.broadcast(message(session_id)).await;

        assert_eq!(channel_count(&broker).await, 0);
    }

    #[tokio::test]
    async fn broadcast_prunes_idle_channel_after_last_receiver_drops() {
        let broker = SseBroker::default();
        let session_id = Uuid::now_v7();
        let receiver = broker.subscribe(session_id).await;
        assert_eq!(channel_count(&broker).await, 1);

        drop(receiver);
        broker.broadcast(message(session_id)).await;

        assert_eq!(channel_count(&broker).await, 0);
    }
}
