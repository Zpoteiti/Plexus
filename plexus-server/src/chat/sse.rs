use crate::db::messages::Message;
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
