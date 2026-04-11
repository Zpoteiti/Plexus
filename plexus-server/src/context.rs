//! Build full LLM prompt: system + soul + memory + skills + devices + history.

use crate::db::messages::Message;
use crate::db::users::User;
use crate::providers::openai::{ChatMessage, FunctionCall, ToolCall};
use crate::state::AppState;
use serde_json::Value;

/// Channel-agnostic sender identity for security boundaries.
/// Constructed by each channel (Discord, Telegram, Gateway) and passed through InboundEvent.
#[derive(Debug, Clone)]
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

/// Build the full context for an LLM call.
/// Returns (messages, tool_schemas).
pub fn build_context(
    state: &AppState,
    user: &User,
    history: &[Message],
    skills: &[SkillInfo],
    tool_schemas: &[Value],
    identity: &ChannelIdentity,
    default_soul: &Option<String>,
    chat_id: Option<&str>,
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

    // 2c — Current Session (channel-specific; 2b channel configs live in the channel itself)
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
    system += &build_device_status(state, &user.user_id);

    // ── Runtime ───────────────────────────────────────────────────────────────
    system += &format!(
        "Current time: {}\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    messages.push(ChatMessage::system(system));

    // 8. Message history (reconstruct from DB rows — includes current user message)
    messages.extend(reconstruct_history(history));

    // Note: current user message is already in DB history (saved before agent loop starts).
    // Non-partner untrusted wrapper is applied when saving to DB in agent_loop.rs.

    messages
}

/// Build device status section for system prompt.
fn build_device_status(state: &AppState, user_id: &str) -> String {
    let mut section = "## Connected Devices\n".to_string();
    let mut has_devices = false;

    // Get all device tokens for this user from the devices_by_user map
    if let Some(keys) = state.devices_by_user.get(user_id) {
        for key in keys.value() {
            if let Some(conn) = state.devices.get(key) {
                let tool_names: Vec<String> = conn
                    .tools
                    .iter()
                    .filter_map(|t| {
                        t.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect();
                section += &format!(
                    "- {}: online ({})\n",
                    conn.device_name,
                    tool_names.join(", ")
                );
                has_devices = true;
            }
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
            let content_len = m.content.as_deref().map(|c| c.len()).unwrap_or(0);
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
        assert_eq!(result[1].content.as_deref(), Some("hello"));
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
