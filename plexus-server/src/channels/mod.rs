//! Channel abstraction and outbound dispatch loop.
//! Each channel converts incoming messages to InboundEvents and delivers OutboundEvents.

pub mod discord;
pub mod gateway;
pub mod telegram;

/// Sanitize an inbound attachment filename before it's used as a workspace path segment.
/// - Strips path separators.
/// - Collapses `..` / `.` / empty into `_`.
/// - Returns `"attachment"` if the result is empty after cleaning (defensive default).
pub(crate) fn safe_attachment_filename(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == '\0' { '_' } else { c })
        .collect();
    match cleaned.as_str() {
        "" | "." | ".." => "attachment".into(),
        _ => cleaned,
    }
}

use crate::bus::OutboundEvent;
use crate::state::AppState;
use plexus_common::consts::{CHANNEL_DISCORD, CHANNEL_GATEWAY};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Channel name for Telegram.
pub const CHANNEL_TELEGRAM: &str = "telegram";

/// Spawn the outbound dispatch loop. Routes OutboundEvents to the correct channel handler.
pub fn spawn_outbound_dispatch(state: Arc<AppState>, mut rx: mpsc::Receiver<OutboundEvent>) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => {
                    info!("outbound dispatch shutting down");
                    break;
                }
                maybe_event = rx.recv() => {
                    let Some(event) = maybe_event else { break; };
                    match event.channel.as_str() {
                        CHANNEL_GATEWAY => {
                            gateway::deliver(&state, &event).await;
                        }
                        CHANNEL_DISCORD => {
                            discord::deliver(&state, &event).await;
                        }
                        CHANNEL_TELEGRAM => {
                            telegram::deliver(&state, &event).await;
                        }
                        "internal" => {
                            // Heartbeat (Plan E) sets channel="internal" on InboundEvent;
                            // Phase 2's per-tool progress hints + send_error inherit that
                            // channel. Intentionally drop — heartbeat has no interactive
                            // channel to receive them; final delivery goes through
                            // publish_final_heartbeat's evaluator + external-channel path.
                            debug!(
                                "Dropped outbound on 'internal' channel (heartbeat progress/error hint): {} chars",
                                event.content.len()
                            );
                        }
                        other => {
                            warn!("Unknown outbound channel: {other}");
                        }
                    }
                    debug!(
                        "Outbound [{}]: {} chars to {:?}",
                        event.channel,
                        event.content.len(),
                        event.chat_id
                    );
                }
            }
        }
    });
}
