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
        let sender = self.sender(message.session_id).await;
        let _ = sender.send(message);
    }

    async fn sender(&self, session_id: Uuid) -> broadcast::Sender<Message> {
        let mut inner = self.inner.lock().await;
        inner
            .entry(session_id)
            .or_insert_with(|| broadcast::channel(256).0)
            .clone()
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
