//! message server tool: send content to a channel with optional media from a device.

use crate::server_tools::ToolContext;
use crate::state::AppState;
use serde_json::Value;
use std::sync::Arc;

/// Enforce the security rule that non-partner senders cannot relay
/// to a different channel or chat_id than their inbound one. Returns
/// an error message on violation, None otherwise. Partners (including
/// cron / server-originated contexts where is_partner=true) are
/// unrestricted.
fn check_cross_channel(
    is_partner: bool,
    ctx_channel: &str,
    ctx_chat_id: Option<&str>,
    target_channel: &str,
    target_chat_id: Option<&str>,
) -> Option<String> {
    if is_partner {
        return None;
    }
    if ctx_channel != target_channel || ctx_chat_id != target_chat_id {
        return Some(
            "Non-partner senders cannot relay messages to a different channel or chat_id".into(),
        );
    }
    None
}

pub async fn message_tool(state: &Arc<AppState>, ctx: &ToolContext, args: &Value) -> (i32, String) {
    let content = match args.get("content").and_then(Value::as_str) {
        Some(c) => c.to_string(),
        None => return (1, "Missing required parameter: content".into()),
    };
    let channel = match args.get("channel").and_then(Value::as_str) {
        Some(c) => c.to_string(),
        None => return (1, "Missing required parameter: channel".into()),
    };
    let chat_id = args
        .get("chat_id")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .or_else(|| ctx.chat_id.clone());

    let media_paths: Vec<String> = args
        .get("media")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let from_device = args.get("from_device").and_then(Value::as_str);

    // Check cross-channel guard
    if let Some(err) = check_cross_channel(
        ctx.is_partner,
        &ctx.channel,
        ctx.chat_id.as_deref(),
        &channel,
        chat_id.as_deref(),
    ) {
        return (1, err);
    }

    // Pull media files from device, save to server, collect URLs
    let mut media_urls = Vec::new();
    if !media_paths.is_empty() {
        let Some(device_name) = from_device else {
            return (
                1,
                format!(
                    "Media not sent: `from_device` is required when sending files. \
                     Call this tool again with `from_device` set to the device that holds the files (e.g. \"local\"). \
                     Files: {}",
                    media_paths.join(", ")
                ),
            );
        };
        for path in &media_paths {
            match super::file_transfer::request_file_from_device(
                state,
                &ctx.user_id,
                device_name,
                path,
            )
            .await
            {
                Ok((bytes, filename)) => {
                    match crate::file_store::save_upload(&ctx.user_id, &filename, &bytes).await {
                        Ok(file_id) => media_urls.push(format!("/api/files/{file_id}")),
                        Err(e) => return (1, format!("Save media failed: {}", e.message)),
                    }
                }
                Err(e) => return (1, e),
            }
        }
    }

    // Publish OutboundEvent
    let _ = state
        .outbound_tx
        .send(crate::bus::OutboundEvent {
            channel,
            chat_id,
            session_id: ctx.session_id.clone(),
            user_id: ctx.user_id.clone(),
            content,
            media: media_urls,
        })
        .await;

    (0, "Message sent.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guard_allows_partner_cross_channel() {
        let err = check_cross_channel(
            /*is_partner=*/ true,
            "discord", Some("c1"),
            "telegram", Some("c2"),
        );
        assert!(err.is_none(), "partner should be allowed cross-channel");
    }

    #[test]
    fn test_guard_allows_non_partner_same_channel() {
        let err = check_cross_channel(
            /*is_partner=*/ false,
            "discord", Some("c1"),
            "discord", Some("c1"),
        );
        assert!(err.is_none(), "non-partner same-channel same-chat_id should be allowed");
    }

    #[test]
    fn test_guard_blocks_non_partner_different_channel() {
        let err = check_cross_channel(
            /*is_partner=*/ false,
            "discord", Some("c1"),
            "telegram", Some("c2"),
        );
        assert!(err.is_some(), "non-partner cross-channel must be rejected");
    }

    #[test]
    fn test_guard_blocks_non_partner_different_chat_id_same_channel() {
        let err = check_cross_channel(
            /*is_partner=*/ false,
            "discord", Some("c1"),
            "discord", Some("c2"),
        );
        assert!(err.is_some(), "non-partner different chat_id must be rejected even on same channel");
    }
}
