/// Gateway application state — stateless w.r.t. sessions.
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

use crate::config::Config;

pub struct AppState {
    pub config: Config,
    pub browsers: Arc<DashMap<String, BrowserConnection>>,
    pub plexus: Arc<RwLock<Option<mpsc::Sender<serde_json::Value>>>>,
    pub http_client: reqwest::Client,
    pub shutdown: CancellationToken,
}

#[derive(Clone)]
pub struct BrowserConnection {
    pub tx: mpsc::Sender<OutboundFrame>,
    pub user_id: String,
    pub cancel: CancellationToken,
}

#[derive(Debug)]
pub enum OutboundFrame {
    Message(serde_json::Value),
    Progress(serde_json::Value),
    Error(serde_json::Value),
    Ping,
    SessionUpdate(serde_json::Value),
}
