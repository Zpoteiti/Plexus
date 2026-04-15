//! Per-session handle: inbox channel + mutex for DB write serialization.

use crate::bus::InboundEvent;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::{Mutex, mpsc};

pub struct SessionHandle {
    #[allow(dead_code)]
    pub user_id: String,
    pub inbox_tx: mpsc::Sender<InboundEvent>,
    pub lock: Arc<Mutex<()>>,
    /// Set to true after the provider's strip-and-retry succeeds in this
    /// session. When true, context::build_user_content replaces image
    /// blocks with text placeholders. Reset to false when the admin
    /// updates the LLM config.
    #[allow(dead_code)]
    pub vision_stripped: Arc<AtomicBool>,
}
