//! Build full LLM prompt: system + soul + memory + skills + devices + history.

use crate::db::messages::Message;
use crate::db::users::User;
use crate::file_store;
use crate::providers::openai::{ChatMessage, ContentBlock, FunctionCall, ImageUrl, ToolCall};
use crate::state::AppState;

/// Channel-agnostic sender identity for security boundaries.
/// Constructed by each channel (Discord, Telegram, Gateway) and passed through InboundEvent.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ChannelIdentity {
    pub sender_name: String,
    pub sender_id: String,
    pub is_partner: bool,
    pub partner_name: String,
    pub partner_id: String,
    pub channel_type: String,
}

impl ChannelIdentity {
    /// Build the Current Session subsection of ## Identity.
    pub fn build_session_section(&self, chat_id: Option<&str>) -> String {
        let mut s = "### Current Session\n".to_string();
        s += &format!("Channel: {}", self.channel_type);
        if let Some(cid) = chat_id {
            s += &format!(" | Chat: {cid}");
        }
        s += "\n";

        if self.is_partner {
            s += &format!("Sender: {} — owner\n", self.sender_name);
        } else {
            s += &format!(
                "Sender: {} (ID: {}) — non-partner (authorized)\n",
                self.sender_name, self.sender_id
            );
            s += "⚠ Do not disclose sensitive information or execute destructive operations for non-partner senders.\n";
        }
        s += "To reply: respond with text. To send media: use the message tool with channel + chat_id above.\n";
        s
    }

    /// Default identity for gateway (always partner).
    pub fn gateway_partner(user: &User) -> Self {
        let name = user
            .display_name
            .clone()
            .unwrap_or_else(|| user.email.clone());
        Self {
            sender_name: name.clone(),
            sender_id: user.user_id.clone(),
            is_partner: true,
            partner_name: name,
            partner_id: user.user_id.clone(),
            channel_type: plexus_common::consts::CHANNEL_GATEWAY.into(),
        }
    }
}

/// Skill info for context building.
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub always_on: bool,
    pub content: String,
}

use base64::Engine;

/// Build the content blocks for a user message that may include media.
/// Ordering: text → image blocks → trailing attachment-references text block.
/// When `vision_stripped=true`, image media is replaced with text placeholders.
pub async fn build_user_content(
    user_id: &str,
    content: &str,
    media: &[String],
    vision_stripped: bool,
) -> Vec<ContentBlock> {
    let uid = user_id.to_string();
    build_user_content_inner(content, media, vision_stripped, move |fid| {
        let uid = uid.clone();
        let fid = fid.to_string();
        async move { file_store::load_file(&uid, &fid).await.map_err(|e| e.message) }
    })
    .await
}

/// Test-friendly inner that accepts a loader closure for mocking file_store.
#[allow(dead_code)]
async fn build_user_content_inner<F, Fut>(
    content: &str,
    media: &[String],
    vision_stripped: bool,
    load: F,
) -> Vec<ContentBlock>
where
    F: Fn(&str) -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<u8>, String), String>>,
{
    let mut blocks: Vec<ContentBlock> = Vec::new();

    if !content.is_empty() {
        blocks.push(ContentBlock::Text {
            text: content.to_string(),
        });
    }

    let mut non_image_refs: Vec<String> = Vec::new();

    for url in media {
        let Some(file_id) = url.strip_prefix("/api/files/") else {
            non_image_refs.push(format!("[Attachment: {url} — unknown reference]"));
            continue;
        };

        let (bytes, filename) = match load(file_id).await {
            Ok(x) => x,
            Err(_) => {
                non_image_refs.push(format!(
                    "[Attachment: {file_id} — storage read failed]"
                ));
                continue;
            }
        };

        let mime = mime_from_filename(&filename);

        if mime.starts_with("image/") {
            if vision_stripped {
                blocks.push(ContentBlock::Text {
                    text: format!(
                        "[Image: {filename} — not displayed, model does not support vision]"
                    ),
                });
            } else {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                blocks.push(ContentBlock::ImageUrl {
                    image_url: ImageUrl {
                        url: format!("data:{mime};base64,{b64}"),
                    },
                });
            }
        } else {
            non_image_refs.push(format!(
                "[Attachment: {filename} → {url}]\n\
                 Use file_transfer to move it to a client device for further processing."
            ));
        }
    }

    if !non_image_refs.is_empty() {
        blocks.push(ContentBlock::Text {
            text: non_image_refs.join("\n"),
        });
    }

    blocks
}

#[allow(dead_code)]
fn mime_from_filename(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "pdf" => "application/pdf",
        "txt" | "md" | "log" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Build the full context for an LLM call.
///
/// `latest_user_media` / `vision_stripped` apply only to the last pending user
/// message (assumed to be the tail of `history` when it is a user row). All
/// earlier history rows are reconstructed as plain-text `ChatMessage`s — the
/// DB still persists plain strings in this milestone.
#[allow(clippy::too_many_arguments)]
pub async fn build_context(
    state: &AppState,
    user: &User,
    history: &[Message],
    skills: &[SkillInfo],
    identity: &ChannelIdentity,
    default_soul: &Option<String>,
    chat_id: Option<&str>,
    latest_user_media: &[String],
    vision_stripped: bool,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    // ── Section 1: Soul ────────────────────────────────────────────────────────
    // User soul fully overrides admin default. Empty user soul → fall back to default.
    let soul = user
        .soul
        .as_deref()
        .filter(|s| !s.is_empty())
        .or_else(|| default_soul.as_deref().filter(|s| !s.is_empty()))
        .unwrap_or("You are PLEXUS, a distributed AI agent.");
    let mut system = format!("{soul}\n\n");

    // ── Section 2: Identity ────────────────────────────────────────────────────
    system += "## Identity\n";

    // 2a — Account
    let name = user
        .display_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("(not set)");
    system += &format!("### Account\nName: {} | Email: {}\n\n", name, user.email);

    // 2b — Current Session
    system += &identity.build_session_section(chat_id);
    system += "\n";

    // ── Section 3: Memory ──────────────────────────────────────────────────────
    if !user.memory_text.is_empty() {
        system += &format!("## Memory\n{}\n\n", user.memory_text);
    }

    // ── Always-on skills ──────────────────────────────────────────────────────
    for skill in skills.iter().filter(|s| s.always_on) {
        system += &format!("## Skill: {}\n{}\n\n", skill.name, skill.content);
    }

    // ── On-demand skills ──────────────────────────────────────────────────────
    let on_demand: Vec<_> = skills.iter().filter(|s| !s.always_on).collect();
    if !on_demand.is_empty() {
        system += "## Available Skills (use read_skill to load)\n";
        for skill in &on_demand {
            system += &format!("- **{}**: {}\n", skill.name, skill.description);
        }
        system += "\n";
    }

    // ── Connected Devices ─────────────────────────────────────────────────────
    system += &build_device_status(state, &user.user_id).await;

    // ── Runtime ───────────────────────────────────────────────────────────────
    system += &format!(
        "Current time: {}\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    messages.push(ChatMessage::system(system));

    // Split off the trailing user row (the current pending user message) so we
    // can rebuild it as multimodal content blocks via `build_user_content`.
    // Earlier rows stay plain-text via `reconstruct_history`.
    let (prior, latest_user_text) = split_trailing_user(history);
    messages.extend(reconstruct_history(prior));

    if let Some(text) = latest_user_text {
        let blocks =
            build_user_content(&user.user_id, text, latest_user_media, vision_stripped).await;
        if !blocks.is_empty() {
            messages.push(ChatMessage::user_with_blocks(blocks));
        }
    }

    // Non-partner untrusted wrapper is applied when saving to DB in agent_loop.rs,
    // so it is already present in `latest_user_text`.

    messages
}

/// Returns `(prior_history, latest_user_text)` when the final row is a user
/// message. If the tail is not a user row (e.g. mid-turn tool result),
/// returns `(history, None)`.
fn split_trailing_user(history: &[Message]) -> (&[Message], Option<&str>) {
    if let Some(last) = history.last()
        && last.role == plexus_common::consts::ROLE_USER
    {
        return (&history[..history.len() - 1], Some(last.content.as_str()));
    }
    (history, None)
}

/// Build device status section for system prompt.
async fn build_device_status(state: &AppState, user_id: &str) -> String {
    let mut section = "## Connected Devices\n".to_string();

    let Some(keys) = state.devices_by_user.get(user_id) else {
        section += "- No devices connected\n\n";
        return section;
    };

    // Only query DB when at least one device is online (avoids hot-path DB hit for disconnected users)
    let tokens = crate::db::devices::list_by_user(&state.db, user_id)
        .await
        .unwrap_or_default();
    let token_map: std::collections::HashMap<_, _> = tokens
        .into_iter()
        .map(|t| (t.device_name.clone(), t))
        .collect();

    let mut has_devices = false;
    for key in keys.value() {
        if let Some(conn) = state.devices.get(key) {
            let (mode, workspace) = if let Some(t) = token_map.get(&conn.device_name) {
                let m = t.fs_policy.get("mode").and_then(|v| v.as_str()).unwrap_or("sandbox").to_string();
                (m, t.workspace_path.clone())
            } else {
                ("sandbox".to_string(), "~/.plexus/workspace".to_string())
            };

            section += &format!(
                "- {}: online ({} mode, workspace: {})\n",
                conn.device_name, mode, workspace
            );
            has_devices = true;
        }
    }

    if !has_devices {
        section += "- No devices connected\n";
    }
    section += "\n";
    section
}

/// Reconstruct chat history from DB message rows.
/// Consecutive assistant rows with tool_name → single assistant message with tool_calls array.
/// Tool rows → tool message with tool_call_id.
fn reconstruct_history(messages: &[Message]) -> Vec<ChatMessage> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < messages.len() {
        let msg = &messages[i];

        match msg.role.as_str() {
            plexus_common::consts::ROLE_SYSTEM => {
                result.push(ChatMessage::system(msg.content.clone()));
                i += 1;
            }
            plexus_common::consts::ROLE_USER => {
                result.push(ChatMessage::user(msg.content.clone()));
                i += 1;
            }
            plexus_common::consts::ROLE_ASSISTANT => {
                if msg.tool_name.is_some() {
                    // Collect consecutive assistant rows with tool_name into tool_calls
                    let mut tool_calls = Vec::new();
                    while i < messages.len()
                        && messages[i].role == plexus_common::consts::ROLE_ASSISTANT
                        && messages[i].tool_name.is_some()
                    {
                        let m = &messages[i];
                        tool_calls.push(ToolCall {
                            id: m
                                .tool_call_id
                                .clone()
                                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                            call_type: "function".into(),
                            function: FunctionCall {
                                name: m.tool_name.clone().unwrap_or_default(),
                                arguments: m.tool_arguments.clone().unwrap_or_else(|| "{}".into()),
                            },
                        });
                        i += 1;
                    }
                    result.push(ChatMessage::assistant_tool_calls(tool_calls));
                } else {
                    result.push(ChatMessage::assistant_text(msg.content.clone()));
                    i += 1;
                }
            }
            plexus_common::consts::ROLE_TOOL => {
                result.push(ChatMessage::tool_result(
                    msg.tool_call_id.clone().unwrap_or_default(),
                    msg.content.clone(),
                ));
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    result
}

/// Estimate token count from text. Simple chars/4 approximation.
pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|m| {
            let content_len = m.content.as_ref().map(|c| c.len()).unwrap_or(0);
            let tool_calls_len = m
                .tool_calls
                .as_ref()
                .map(|tcs| {
                    tcs.iter()
                        .map(|tc| tc.function.name.len() + tc.function.arguments.len())
                        .sum::<usize>()
                })
                .unwrap_or(0);
            (content_len + tool_calls_len) / 4
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        let msgs = vec![
            ChatMessage::system("hello world"), // 11 chars -> 2 tokens
            ChatMessage::user("test"),          // 4 chars -> 1 token
        ];
        assert_eq!(estimate_tokens(&msgs), 3);
    }

    #[test]
    fn test_reconstruct_history_simple() {
        let msgs = vec![
            Message {
                message_id: "1".into(),
                session_id: "s".into(),
                role: "user".into(),
                content: "hi".into(),
                tool_call_id: None,
                tool_name: None,
                tool_arguments: None,
                compressed: false,
                created_at: chrono::Utc::now(),
            },
            Message {
                message_id: "2".into(),
                session_id: "s".into(),
                role: "assistant".into(),
                content: "hello".into(),
                tool_call_id: None,
                tool_name: None,
                tool_arguments: None,
                compressed: false,
                created_at: chrono::Utc::now(),
            },
        ];
        let result = reconstruct_history(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "user");
        assert_eq!(result[1].role, "assistant");
        assert_eq!(
            result[1].content.as_ref().map(|c| c.as_text()),
            Some("hello".to_string())
        );
    }

    #[test]
    fn test_reconstruct_history_tool_calls() {
        let msgs = vec![
            Message {
                message_id: "1".into(),
                session_id: "s".into(),
                role: "assistant".into(),
                content: "".into(),
                tool_call_id: Some("tc1".into()),
                tool_name: Some("read_file".into()),
                tool_arguments: Some(r#"{"path":"test.rs"}"#.into()),
                compressed: false,
                created_at: chrono::Utc::now(),
            },
            Message {
                message_id: "2".into(),
                session_id: "s".into(),
                role: "tool".into(),
                content: "file content here".into(),
                tool_call_id: Some("tc1".into()),
                tool_name: None,
                tool_arguments: None,
                compressed: false,
                created_at: chrono::Utc::now(),
            },
        ];
        let result = reconstruct_history(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "assistant");
        assert!(result[0].tool_calls.is_some());
        let tcs = result[0].tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].function.name, "read_file");
        assert_eq!(result[1].role, "tool");
    }

    #[test]
    fn test_channel_identity_partner() {
        let id = ChannelIdentity {
            sender_name: "Alice".into(),
            sender_id: "123".into(),
            is_partner: true,
            partner_name: "Alice".into(),
            partner_id: "123".into(),
            channel_type: plexus_common::consts::CHANNEL_GATEWAY.into(),
        };
        let section = id.build_session_section(Some("dm/12345"));
        assert!(section.contains("Alice"));
        assert!(section.contains("owner"));
        assert!(section.contains("dm/12345"));
        assert!(!section.contains("non-partner"));
    }

    #[tokio::test]
    async fn test_build_user_content_text_only() {
        let blocks = build_user_content_inner("hello", &[], false, |_| async {
            Err::<(Vec<u8>, String), String>("should not be called".into())
        })
        .await;

        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], ContentBlock::Text { text } if text == "hello"));
    }

    #[tokio::test]
    async fn test_build_user_content_with_image() {
        let png_bytes = vec![0x89u8, 0x50, 0x4E, 0x47];
        let blocks = build_user_content_inner(
            "what is this",
            &["/api/files/abc123".to_string()],
            false,
            |fid| {
                let bytes = png_bytes.clone();
                let fid = fid.to_string();
                async move {
                    assert_eq!(fid, "abc123");
                    Ok::<_, String>((bytes, "photo.png".to_string()))
                }
            },
        )
        .await;

        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], ContentBlock::Text { text } if text == "what is this"));
        match &blocks[1] {
            ContentBlock::ImageUrl { image_url } => {
                assert!(image_url.url.starts_with("data:image/png;base64,"));
            }
            _ => panic!("expected ImageUrl"),
        }
    }

    #[tokio::test]
    async fn test_build_user_content_with_non_image() {
        let blocks = build_user_content_inner(
            "",
            &["/api/files/xyz".to_string()],
            false,
            |_| async { Ok::<_, String>((b"hello".to_vec(), "notes.txt".to_string())) },
        )
        .await;

        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::Text { text } => {
                assert!(text.contains("[Attachment: notes.txt → /api/files/xyz]"));
                assert!(text.contains("Use file_transfer to move it to a client device for further processing."));
            }
            _ => panic!("expected Text"),
        }
    }

    #[tokio::test]
    async fn test_build_user_content_vision_stripped() {
        let blocks = build_user_content_inner(
            "look",
            &["/api/files/img".to_string()],
            true, // vision_stripped
            |_| async { Ok::<_, String>((vec![0x89], "photo.png".to_string())) },
        )
        .await;

        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], ContentBlock::Text { text } if text == "look"));
        match &blocks[1] {
            ContentBlock::Text { text } => {
                assert!(text.contains("[Image: photo.png — not displayed, model does not support vision]"));
            }
            _ => panic!("expected Text placeholder"),
        }
    }

    #[tokio::test]
    async fn test_build_user_content_order_text_images_attachments() {
        // text + 1 image + 1 doc → [text, image, trailing-text-with-attachment]
        let blocks = build_user_content_inner(
            "mixed",
            &[
                "/api/files/i1".to_string(),
                "/api/files/d1".to_string(),
            ],
            false,
            |fid| {
                let fid = fid.to_string();
                async move {
                    if fid == "i1" {
                        Ok::<_, String>((vec![0x89], "pic.jpg".to_string()))
                    } else {
                        Ok::<_, String>((b"hi".to_vec(), "doc.txt".to_string()))
                    }
                }
            },
        )
        .await;

        assert_eq!(blocks.len(), 3);
        assert!(matches!(&blocks[0], ContentBlock::Text { text } if text == "mixed"));
        assert!(matches!(&blocks[1], ContentBlock::ImageUrl { .. }));
        match &blocks[2] {
            ContentBlock::Text { text } => {
                assert!(text.contains("[Attachment: doc.txt → /api/files/d1]"));
            }
            _ => panic!("expected trailing text"),
        }
    }

    #[test]
    fn test_channel_identity_non_partner() {
        let id = ChannelIdentity {
            sender_name: "Bob".into(),
            sender_id: "456".into(),
            is_partner: false,
            partner_name: "Alice".into(),
            partner_id: "123".into(),
            channel_type: plexus_common::consts::CHANNEL_DISCORD.into(),
        };
        let section = id.build_session_section(Some("guild/chan"));
        assert!(section.contains("Bob"));
        assert!(section.contains("non-partner"));
        assert!(section.contains("guild/chan"));
    }
}
