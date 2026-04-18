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

    // Pull media files from device or server workspace, save to file store, collect URLs
    let mut media_urls = Vec::new();
    if !media_paths.is_empty() {
        let Some(device_name) = from_device else {
            return (
                1,
                format!(
                    "Media not sent: `from_device` is required when sending files. \
                     Call this tool again with `from_device` set to \"server\" (for workspace files) \
                     or the name of a client device that holds the files. Files: {}",
                    media_paths.join(", ")
                ),
            );
        };

        for path in &media_paths {
            let (bytes, filename) = if device_name == "server" {
                // Read from the user's server workspace.
                let ws_root = std::path::Path::new(&state.config.workspace_root);
                let resolved =
                    match crate::workspace::resolve_user_path(ws_root, &ctx.user_id, path).await {
                        Ok(p) => p,
                        Err(crate::workspace::WorkspaceError::Traversal(_)) => {
                            return (1, "Path escapes user workspace".into());
                        }
                        Err(crate::workspace::WorkspaceError::Io(e))
                            if e.kind() == std::io::ErrorKind::NotFound =>
                        {
                            return (1, format!("File not found: {path}"));
                        }
                        Err(e) => return (1, format!("Resolve error on {path}: {e}")),
                    };
                let bytes = match tokio::fs::read(&resolved).await {
                    Ok(b) => b,
                    Err(e) => return (1, format!("Read error on {path}: {e}")),
                };
                let filename = resolved
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                (bytes, filename)
            } else {
                // Request from a client device.
                match super::file_transfer::request_file_from_device(
                    state,
                    &ctx.user_id,
                    device_name,
                    path,
                )
                .await
                {
                    Ok(pair) => pair,
                    Err(e) => return (1, e),
                }
            };

            match crate::file_store::save_upload(state, &ctx.user_id, &filename, &bytes).await {
                Ok(file_id) => media_urls.push(format!("/api/files/{file_id}")),
                Err(e) => return (1, format!("Save media failed: {}", e.message)),
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

    #[tokio::test]
    async fn test_message_tool_server_media_reads_workspace_file() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(user_dir.join("uploads"))
            .await
            .unwrap();
        tokio::fs::write(user_dir.join("uploads/report.pdf"), b"fake-pdf-bytes")
            .await
            .unwrap();

        let (state, mut outbound_rx) = AppState::test_minimal_with_outbound(tmp.path());

        let ctx = ToolContext {
            user_id: "alice".into(),
            session_id: "sess-1".into(),
            channel: "gateway".into(),
            chat_id: Some("chat-1".into()),
            is_cron: false,
            is_partner: true,
        };
        let args = serde_json::json!({
            "content": "Here's the report",
            "channel": "gateway",
            "chat_id": "chat-1",
            "media": ["uploads/report.pdf"],
            "from_device": "server",
        });
        let (code, out) = message_tool(&state, &ctx, &args).await;
        assert_eq!(code, 0, "expected success, got: {out}");

        let event = outbound_rx.recv().await.expect("outbound event published");
        assert_eq!(event.content, "Here's the report");
        assert_eq!(event.media.len(), 1, "expected 1 media URL");
        assert!(
            event.media[0].starts_with("/api/files/"),
            "expected /api/files URL, got: {}",
            event.media[0]
        );
    }

    #[tokio::test]
    async fn test_message_tool_server_media_traversal_rejected() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        let other = tmp.path().join("bob");
        tokio::fs::create_dir_all(&other).await.unwrap();
        tokio::fs::write(other.join("secret.pdf"), b"x")
            .await
            .unwrap();

        let state = AppState::test_minimal(tmp.path());

        let ctx = ToolContext {
            user_id: "alice".into(),
            session_id: "sess-1".into(),
            channel: "gateway".into(),
            chat_id: Some("chat-1".into()),
            is_cron: false,
            is_partner: true,
        };
        let args = serde_json::json!({
            "content": "Try to leak",
            "channel": "gateway",
            "chat_id": "chat-1",
            "media": ["../bob/secret.pdf"],
            "from_device": "server",
        });
        let (code, out) = message_tool(&state, &ctx, &args).await;
        assert_eq!(code, 1);
        assert!(out.contains("escapes"), "got: {out}");
    }

    #[tokio::test]
    async fn test_message_tool_server_media_missing_errors() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let state = AppState::test_minimal(tmp.path());

        let ctx = ToolContext {
            user_id: "alice".into(),
            session_id: "sess-1".into(),
            channel: "gateway".into(),
            chat_id: Some("chat-1".into()),
            is_cron: false,
            is_partner: true,
        };
        let args = serde_json::json!({
            "content": "Ghost file",
            "channel": "gateway",
            "chat_id": "chat-1",
            "media": ["uploads/ghost.pdf"],
            "from_device": "server",
        });
        let (code, out) = message_tool(&state, &ctx, &args).await;
        assert_eq!(code, 1);
        assert!(out.contains("not found"), "got: {out}");
        assert!(
            out.contains("uploads/ghost.pdf"),
            "expected path echo, got: {out}"
        );
    }

    #[tokio::test]
    async fn test_message_tool_server_media_multiple_files() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("alice");
        tokio::fs::create_dir_all(user_dir.join("uploads"))
            .await
            .unwrap();
        tokio::fs::write(user_dir.join("uploads/a.txt"), b"one")
            .await
            .unwrap();
        tokio::fs::write(user_dir.join("uploads/b.txt"), b"two")
            .await
            .unwrap();

        let (state, mut outbound_rx) = AppState::test_minimal_with_outbound(tmp.path());

        let ctx = ToolContext {
            user_id: "alice".into(),
            session_id: "sess-1".into(),
            channel: "gateway".into(),
            chat_id: Some("chat-1".into()),
            is_cron: false,
            is_partner: true,
        };
        let args = serde_json::json!({
            "content": "two files",
            "channel": "gateway",
            "chat_id": "chat-1",
            "media": ["uploads/a.txt", "uploads/b.txt"],
            "from_device": "server",
        });
        let (code, out) = message_tool(&state, &ctx, &args).await;
        assert_eq!(code, 0, "expected success, got: {out}");

        let event = outbound_rx.recv().await.expect("outbound event published");
        assert_eq!(event.content, "two files");
        assert_eq!(event.media.len(), 2, "expected 2 media URLs");
        for url in &event.media {
            assert!(
                url.starts_with("/api/files/"),
                "expected /api/files URL, got: {url}"
            );
        }
    }

    // Note: "mixed media" (one server file + one client device file in a single call) is not
    // representable because the tool schema has a single `from_device` field. There is no
    // per-item device selector, so per-device mixing cannot occur at the protocol level.

    #[test]
    fn test_guard_allows_partner_cross_channel() {
        let err = check_cross_channel(
            /*is_partner=*/ true,
            "discord",
            Some("c1"),
            "telegram",
            Some("c2"),
        );
        assert!(err.is_none(), "partner should be allowed cross-channel");
    }

    #[test]
    fn test_guard_allows_non_partner_same_channel() {
        let err = check_cross_channel(
            /*is_partner=*/ false,
            "discord",
            Some("c1"),
            "discord",
            Some("c1"),
        );
        assert!(
            err.is_none(),
            "non-partner same-channel same-chat_id should be allowed"
        );
    }

    #[test]
    fn test_guard_blocks_non_partner_different_channel() {
        let err = check_cross_channel(
            /*is_partner=*/ false,
            "discord",
            Some("c1"),
            "telegram",
            Some("c2"),
        );
        assert!(err.is_some(), "non-partner cross-channel must be rejected");
    }

    #[test]
    fn test_guard_blocks_non_partner_different_chat_id_same_channel() {
        let err = check_cross_channel(
            /*is_partner=*/ false,
            "discord",
            Some("c1"),
            "discord",
            Some("c2"),
        );
        assert!(
            err.is_some(),
            "non-partner different chat_id must be rejected even on same channel"
        );
    }
}
