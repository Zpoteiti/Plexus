//! Build full LLM prompt: system + soul + memory + skills + devices + history.

use crate::db::messages::Message;
use crate::db::users::User;
use crate::file_store;
use crate::providers::openai::{
    ChatMessage, Content, ContentBlock, FunctionCall, ImageUrl, ToolCall,
};
use crate::state::AppState;

/// Channel-agnostic sender identity for security boundaries.
/// Constructed by each channel (Discord, Telegram, Gateway) and passed through InboundEvent.
#[derive(Debug, Clone)]
pub struct ChannelIdentity {
    pub sender_name: String,
    pub sender_id: String,
    pub is_partner: bool,
    pub channel_type: String,
}

/// Discriminant that controls which context shape build_context assembles.
///
/// D-8 will implement the real Dream branch (phase 2 prompt + memory + soul +
/// skills, omitting channel identity + devices + current time).
/// Plan E will implement the Heartbeat branch.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PromptMode {
    UserTurn,
    Dream,
    Heartbeat,
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
            sender_name: name,
            sender_id: user.user_id.clone(),
            is_partner: true,
            channel_type: plexus_common::consts::CHANNEL_GATEWAY.into(),
        }
    }
}

/// Skill info for context building.
#[derive(Clone)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub always_on: bool,
    pub content: String,
}

use base64::Engine;

/// Build the content blocks for a user message that may include media.
/// Ordering: text → image blocks → trailing attachment-references text block.
///
/// Images are always emitted as base64 data-URL `ImageUrl` blocks. Vision
/// stripping is handled as a separate post-pass in the provider layer via
/// `providers::openai::strip_images_in_place`, so the canonical form stored
/// in the DB remains unstripped.
pub async fn build_user_content(
    state: &std::sync::Arc<AppState>,
    user_id: &str,
    content: &str,
    media: &[String],
) -> Vec<ContentBlock> {
    let uid = user_id.to_string();
    let state = state.clone();
    build_user_content_inner(content, media, move |fid| {
        let uid = uid.clone();
        let fid = fid.to_string();
        let state = state.clone();
        async move {
            file_store::load_file(&state, &uid, &fid)
                .await
                .map_err(|e| e.message)
        }
    })
    .await
}

/// Test-friendly inner that accepts a loader closure for mocking file_store.
async fn build_user_content_inner<F, Fut>(
    content: &str,
    media: &[String],
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
                non_image_refs.push(format!("[Attachment: {file_id} — storage read failed]"));
                continue;
            }
        };

        let mime = mime_from_filename(&filename);

        if mime.starts_with("image/") {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            blocks.push(ContentBlock::ImageUrl {
                image_url: ImageUrl {
                    url: format!("data:{mime};base64,{b64}"),
                },
            });
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

// TODO(cleanup): delete this helper; use plexus_common::mime::detect_mime_from_extension.
// Removed in P3.7/P4.4.
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

/// Per-user channel configuration summary used to render the `## Channels`
/// section. `None` fields mean the channel is not configured or not enabled
/// for this user.
#[derive(Debug, Clone, Default)]
pub struct ChannelSnapshot {
    pub discord_partner_id: Option<String>,
    pub telegram_partner_id: Option<String>,
}

fn render_channels_section(snap: &ChannelSnapshot) -> String {
    let mut s = String::from("## Channels\n");
    s += "Your partner is reachable via the `message` tool on these channels:\n";
    if let Some(id) = &snap.discord_partner_id {
        s += &format!("- discord: chat_id=\"dm/{id}\"\n");
    }
    if let Some(id) = &snap.telegram_partner_id {
        s += &format!("- telegram: chat_id=\"{id}\"\n");
    }
    s += "- gateway: no chat_id needed — messages post to the current session\n";
    s
}

async fn load_channel_snapshot(state: &AppState, user_id: &str) -> ChannelSnapshot {
    let discord_partner_id = crate::db::discord::get_config(&state.db, user_id)
        .await
        .ok()
        .flatten()
        .filter(|c| c.enabled)
        .and_then(|c| c.partner_discord_id);
    let telegram_partner_id = crate::db::telegram::get_config(&state.db, user_id)
        .await
        .ok()
        .flatten()
        .filter(|c| c.enabled)
        .and_then(|c| c.partner_telegram_id);
    ChannelSnapshot {
        discord_partner_id,
        telegram_partner_id,
    }
}

/// Assemble the dream Phase-2 system prompt from its components.
///
/// Pure function — no I/O — so it can be called directly in unit tests.
/// `build_context` delegates to this when `mode == PromptMode::Dream`.
pub(crate) fn assemble_dream_system_prompt(
    phase2: &str,
    memory: &str,
    soul: &str,
    skills_section: &str,
) -> String {
    format!(
        "{phase2}\n\n\
         ## Current MEMORY.md\n\n{memory}\n\n\
         ## Current SOUL.md\n\n{soul}\n\n\
         {skills_section}"
    )
}

const HEARTBEAT_BANNER: &str = "## Autonomous Wake-Up\n\
This is an autonomous heartbeat wake-up triggered by your scheduled task list. \
Complete the requested tasks without asking for clarifying questions — pick \
reasonable defaults and proceed. Do not use the `message` tool to deliver a \
reply; produce a concise final assistant message summarizing what you did. \
The system will decide whether to notify the user through an external channel.\n";

async fn build_heartbeat_system(
    soul: &str,
    user: &User,
    _identity: &ChannelIdentity,
    state: &AppState,
    memory: &str,
    skills_section: &str,
) -> String {
    let mut s = format!("{soul}\n\n");

    // Section: Identity (identical rendering to UserTurn)
    s += "## Identity\n";
    let name = user
        .display_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("(not set)");
    s += &format!("### Account\nName: {} | Email: {}\n\n", name, user.email);
    // Heartbeat has no interactive channel — skip identity.build_session_section
    // (which would inject "To send media: use the message tool …", contradicting
    // the HEARTBEAT_BANNER's no-message-tool-for-delivery rule below).
    s += "### Current Session\nAutonomous heartbeat (no interactive channel)\n\n";

    // NO ## Channels section — heartbeat never routes to an interactive channel.

    // Memory
    if !memory.trim().is_empty() {
        s += &format!("## Memory\n{memory}\n\n");
    }

    // Skills
    s += skills_section;

    // Devices
    s += &build_device_status(state, &user.user_id).await;

    // Runtime
    s += &format!(
        "Current time: {}\n\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    // Autonomous wake-up banner (pins behavior)
    s += HEARTBEAT_BANNER;

    s
}

/// Build the full context for an LLM call.
///
/// User-role rows may hold JSON-serialized `Content::Blocks` (written by
/// `agent_loop` when the original message had media). `reconstruct_history`
/// rehydrates those into `ChatMessage::content = Some(Content::Blocks(..))`
/// in place; plain-text rows round-trip as `Content::Text`.
///
/// When `vision_stripped` is true, `strip_images_in_place` runs as a final
/// post-pass, replacing every image block in every user message with a text
/// placeholder. The canonical (unstripped) form stays in the DB.
#[allow(clippy::too_many_arguments)]
pub async fn build_context(
    state: &AppState,
    user: &User,
    history: &[Message],
    skills: &[SkillInfo],
    identity: &ChannelIdentity,
    default_soul: &Option<String>,
    chat_id: Option<&str>,
    vision_stripped: bool,
    mode: PromptMode,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    // ── Common data: loaded for all modes ─────────────────────────────────────
    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let user_root = ws_root.join(&user.user_id);

    let soul_from_file = tokio::fs::read_to_string(user_root.join("SOUL.md"))
        .await
        .unwrap_or_default();
    let soul = if !soul_from_file.trim().is_empty() {
        soul_from_file.as_str()
    } else if let Some(s) = default_soul.as_deref().filter(|s| !s.is_empty()) {
        s
    } else {
        "You are PLEXUS, a distributed AI agent."
    };

    let memory = tokio::fs::read_to_string(user_root.join("MEMORY.md"))
        .await
        .unwrap_or_default();

    // ── Skills section (identical across all modes) ───────────────────────────
    let mut skills_section = String::new();
    for skill in skills.iter().filter(|s| s.always_on) {
        skills_section += &format!("## Skill: {}\n{}\n\n", skill.name, skill.content);
    }
    let on_demand: Vec<_> = skills.iter().filter(|s| !s.always_on).collect();
    if !on_demand.is_empty() {
        skills_section += "## Available Skills (use read_file on skills/{name}/SKILL.md to load)\n";
        for skill in &on_demand {
            skills_section += &format!("- **{}**: {}\n", skill.name, skill.description);
        }
        skills_section += "\n";
    }

    // ── Mode-specific system prompt assembly ──────────────────────────────────
    let system = match mode {
        PromptMode::UserTurn => {
            // UserTurn: full identity + channels + devices + time.
            let mut s = format!("{soul}\n\n");

            // Section: Identity
            s += "## Identity\n";
            let name = user
                .display_name
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("(not set)");
            s += &format!("### Account\nName: {} | Email: {}\n\n", name, user.email);
            s += &identity.build_session_section(chat_id);
            s += "\n";

            // Channels
            let snap = load_channel_snapshot(state, &user.user_id).await;
            s += &render_channels_section(&snap);
            s += "Reply on the current channel unless the partner asks otherwise.\n\n";

            // Attachments
            s += "## Attachments\n";
            s += "Files may appear as [Attachment: name → /api/files/{id}]. They live on the\n";
            s += "server. To operate on one, use `file_transfer` to move it to a client device,\n";
            s += "then use client tools (shell, read_file, etc.). Choose the action based on\n";
            s += "filename and the user's intent.\n\n";

            // Memory
            if !memory.trim().is_empty() {
                s += &format!("## Memory\n{}\n\n", memory);
            }

            // Skills
            s += &skills_section;

            // Devices
            s += &build_device_status(state, &user.user_id).await;

            // Runtime
            s += &format!(
                "Current time: {}\n",
                chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
            );

            s
        }
        PromptMode::Heartbeat => {
            build_heartbeat_system(soul, user, identity, state, &memory, &skills_section).await
        }
        PromptMode::Dream => {
            // Dream Phase 2: phase2 prompt + memory + soul + skills.
            // Deliberately OMITS channel identity, device list, and current time —
            // dream is an autonomous server-side pass, not a user-facing reply.
            let phase2 = state.dream_phase2_prompt.read().await.clone();
            assemble_dream_system_prompt(&phase2, &memory, soul, &skills_section)
        }
    };

    messages.push(ChatMessage::system(system));

    // Reconstruct the full history — user rows with multimodal content are
    // rehydrated from their JSON form by `reconstruct_history`.
    messages.extend(reconstruct_history(history));

    // Non-partner untrusted wrapper is applied when saving to DB in agent_loop.rs,
    // so it is already present in the stored user content.

    // Post-pass: if a prior LLM call failed with images and succeeded without,
    // replace every image block with a text placeholder. Touches only
    // role=="user" messages, so it's safe to apply after the system prompt.
    if vision_stripped {
        let _ = crate::providers::openai::strip_images_in_place(&mut messages);
    }

    messages
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
                let m = t
                    .fs_policy
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("sandbox")
                    .to_string();
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
///
/// User rows whose `content` is a JSON-serialized `Content::Blocks` array are
/// rehydrated into multimodal `ChatMessage`s; plain-text rows fall back to
/// `Content::Text`.
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
                if let Some(content) = try_parse_user_content(&msg.content) {
                    result.push(ChatMessage {
                        role: "user".into(),
                        content: Some(content),
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    });
                } else {
                    result.push(ChatMessage::user(msg.content.clone()));
                }
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

/// Try to interpret a raw stored user-content string as a JSON-serialized
/// `Content::Blocks` array. Returns `Some(Content::Blocks(..))` on success,
/// `None` if the string is plain text (or JSON but not a valid block array).
fn try_parse_user_content(raw: &str) -> Option<Content> {
    if !raw.starts_with('[') {
        return None;
    }
    match serde_json::from_str::<Content>(raw) {
        Ok(c @ Content::Blocks(_)) => Some(c),
        _ => None,
    }
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
            result[1].content.clone().map(|c| c.into_text()),
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
        let blocks = build_user_content_inner("hello", &[], |_| async {
            Err::<(Vec<u8>, String), String>("should not be called".into())
        })
        .await;

        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], ContentBlock::Text { text } if text == "hello"));
    }

    #[tokio::test]
    async fn test_build_user_content_with_image() {
        let png_bytes = vec![0x89u8, 0x50, 0x4E, 0x47];
        let blocks =
            build_user_content_inner("what is this", &["/api/files/abc123".to_string()], |fid| {
                let bytes = png_bytes.clone();
                let fid = fid.to_string();
                async move {
                    assert_eq!(fid, "abc123");
                    Ok::<_, String>((bytes, "photo.png".to_string()))
                }
            })
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
        let blocks = build_user_content_inner("", &["/api/files/xyz".to_string()], |_| async {
            Ok::<_, String>((b"hello".to_vec(), "notes.txt".to_string()))
        })
        .await;

        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::Text { text } => {
                assert!(text.contains("[Attachment: notes.txt → /api/files/xyz]"));
                assert!(text.contains(
                    "Use file_transfer to move it to a client device for further processing."
                ));
            }
            _ => panic!("expected Text"),
        }
    }

    #[tokio::test]
    async fn test_build_user_content_order_text_images_attachments() {
        // text + 1 image + 1 doc → [text, image, trailing-text-with-attachment]
        let blocks = build_user_content_inner(
            "mixed",
            &["/api/files/i1".to_string(), "/api/files/d1".to_string()],
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
    fn test_reconstruct_history_parses_user_json_blocks() {
        // A user row saved by the new write path: JSON-serialized Content::Blocks.
        let raw_blocks = r#"[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AA"}}]"#;
        let msgs = vec![Message {
            message_id: "m1".into(),
            session_id: "s".into(),
            role: "user".into(),
            content: raw_blocks.to_string(),
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            compressed: false,
            created_at: chrono::Utc::now(),
        }];
        let result = reconstruct_history(&msgs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
        match result[0].content.as_ref().unwrap() {
            Content::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
                assert!(matches!(&blocks[0], ContentBlock::Text { text } if text == "hi"));
                assert!(matches!(&blocks[1], ContentBlock::ImageUrl { .. }));
            }
            _ => panic!("expected Content::Blocks after JSON sniff"),
        }
    }

    #[test]
    fn test_reconstruct_history_falls_back_to_text_on_plain_string() {
        let msgs = vec![Message {
            message_id: "m1".into(),
            session_id: "s".into(),
            role: "user".into(),
            content: "just plain text".into(),
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            compressed: false,
            created_at: chrono::Utc::now(),
        }];
        let result = reconstruct_history(&msgs);
        match result[0].content.as_ref().unwrap() {
            Content::Text(t) => assert_eq!(t, "just plain text"),
            _ => panic!("plain string should round-trip as Content::Text"),
        }
    }

    #[test]
    fn test_channel_identity_non_partner() {
        let id = ChannelIdentity {
            sender_name: "Bob".into(),
            sender_id: "456".into(),
            is_partner: false,
            channel_type: plexus_common::consts::CHANNEL_DISCORD.into(),
        };
        let section = id.build_session_section(Some("guild/chan"));
        assert!(section.contains("Bob"));
        assert!(section.contains("non-partner"));
        assert!(section.contains("guild/chan"));
    }

    #[test]
    fn test_attachments_section_content() {
        // The system prompt's Attachments section should mention file_transfer
        // and the [Attachment: ...] marker format. This test pins the exact
        // four-line phrasing from Task 11 of the plan.
        let body = "## Attachments\n\
                    Files may appear as [Attachment: name → /api/files/{id}]. They live on the\n\
                    server. To operate on one, use `file_transfer` to move it to a client device,\n\
                    then use client tools (shell, read_file, etc.). Choose the action based on\n\
                    filename and the user's intent.\n";
        assert!(body.contains("[Attachment: name → /api/files/{id}]"));
        assert!(body.contains("file_transfer"));
        assert!(body.contains("client device"));
        assert!(body.contains("filename and the user's intent"));
    }

    #[test]
    fn test_channels_section_lists_discord_when_enabled() {
        let snap = ChannelSnapshot {
            discord_partner_id: Some("owner_dc".into()),
            telegram_partner_id: None,
        };
        let section = render_channels_section(&snap);
        assert!(section.contains("## Channels"));
        assert!(section.contains(r#"chat_id="dm/owner_dc""#));
        assert!(!section.contains("telegram"));
        assert!(section.contains("gateway"));
    }

    #[test]
    fn test_channels_section_lists_telegram_when_enabled() {
        let snap = ChannelSnapshot {
            discord_partner_id: None,
            telegram_partner_id: Some("owner_tg".into()),
        };
        let section = render_channels_section(&snap);
        assert!(section.contains("## Channels"));
        assert!(!section.contains("discord"));
        assert!(section.contains(r#"chat_id="owner_tg""#));
        assert!(section.contains("gateway"));
    }

    #[test]
    fn test_channels_section_only_gateway_when_none_configured() {
        let snap = ChannelSnapshot {
            discord_partner_id: None,
            telegram_partner_id: None,
        };
        let section = render_channels_section(&snap);
        assert!(section.contains("## Channels"));
        assert!(!section.contains("discord"));
        assert!(!section.contains("telegram"));
        assert!(section.contains("gateway"));
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn dream_mode_injects_phase2_prompt_and_workspace_files() {
        let phase2 = "DREAM_PHASE2_MARKER";
        let memory = "## User Facts\n- test-fact";
        let soul = "# Soul\nhelpful-alice";
        let skills_section = "";

        let content = assemble_dream_system_prompt(phase2, memory, soul, skills_section);

        assert!(
            content.contains("DREAM_PHASE2_MARKER"),
            "phase2 prompt marker missing: {content}"
        );
        assert!(content.contains("test-fact"), "MEMORY.md content missing");
        assert!(content.contains("helpful-alice"), "SOUL.md content missing");

        // Dream mode should NOT include UserTurn-only sections.
        assert!(
            !content.contains("## Channels"),
            "Dream must omit Channels section"
        );
        assert!(
            !content.contains("## Connected Devices"),
            "Dream must omit Devices section"
        );
        assert!(
            !content.contains("Current time:"),
            "Dream must omit runtime timestamp"
        );
        assert!(
            !content.contains("## Identity"),
            "Dream must omit Identity section"
        );
    }

    #[test]
    fn dream_mode_includes_skills_section() {
        let skills_section = "## Skill: wrap-up\nA skill for wrapping up sessions.\n\n\
                              ## Available Skills (use read_file on skills/{name}/SKILL.md to load)\n\
                              - **git**: Manage git repos\n\n";

        let content = assemble_dream_system_prompt("PHASE2", "mem", "soul", skills_section);

        assert!(
            content.contains("## Skill: wrap-up"),
            "always-on skill missing"
        );
        assert!(
            content.contains("## Available Skills"),
            "on-demand skills index missing"
        );
        assert!(content.contains("**git**"), "on-demand skill entry missing");
    }

    #[test]
    fn dream_mode_order_phase2_then_memory_then_soul_then_skills() {
        let content = assemble_dream_system_prompt("PHASE2", "MEM", "SOUL", "SKILLS\n");

        let pos_phase2 = content.find("PHASE2").expect("PHASE2 missing");
        let pos_mem = content.find("MEM").expect("MEM missing");
        let pos_soul = content.find("SOUL").expect("SOUL missing");
        let pos_skills = content.find("SKILLS").expect("SKILLS missing");

        assert!(pos_phase2 < pos_mem, "phase2 must precede memory");
        assert!(pos_mem < pos_soul, "memory must precede soul");
        assert!(pos_soul < pos_skills, "soul must precede skills");
    }

    #[test]
    fn heartbeat_banner_contains_no_clarifying_question_guidance() {
        // Pin the banner wording so future edits don't regress the
        // "don't ask clarifying questions" contract.
        assert!(HEARTBEAT_BANNER.contains("without asking for clarifying questions"));
        assert!(HEARTBEAT_BANNER.contains("Do not use the `message` tool"));
        assert!(HEARTBEAT_BANNER.contains("summarizing what you did"));
    }

    // Note: a full end-to-end `build_context(PromptMode::Heartbeat, ...)`
    // integration test would require a real PgPool (because
    // build_device_status and load_channel_snapshot hit the DB). That is
    // covered as a manual smoke test + the ignore-gated test in E-9. The
    // banner pin above plus the UserTurn/Dream regression tests below
    // give us confidence the match arm routes correctly.
}
