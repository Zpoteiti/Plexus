//! OpenAI-compatible chat completions provider.
//! Single POST {api_base}/chat/completions endpoint.
//! Retry with exponential backoff for 429/5xx. Strips <think> tags.

use crate::config::LlmConfig;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;
use tracing::{debug, warn};

// -- Request/Response types --

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageUrl {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: Some(Content::Text(content.into())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(Content::Text(content.into())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user_with_blocks(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role: "user".into(),
            content: Some(Content::Blocks(blocks)),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant_text(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: Some(Content::Text(content.into())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(Content::Text(content.into())),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: None,
        }
    }
}

impl Content {
    /// Returns the concatenated text from this content. For Blocks, image
    /// blocks are dropped and text blocks joined in order.
    #[allow(dead_code)]
    pub fn as_text(&self) -> String {
        match self {
            Content::Text(s) => s.clone(),
            Content::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    ContentBlock::ImageUrl { .. } => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        }
    }

    /// Consuming variant that avoids cloning when the caller owns the Content.
    pub fn into_text(self) -> String {
        match self {
            Content::Text(s) => s,
            Content::Blocks(blocks) => blocks
                .into_iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text),
                    ContentBlock::ImageUrl { .. } => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        }
    }

    /// Length in bytes of the text this content would produce, without
    /// materializing the string.
    pub fn len(&self) -> usize {
        match self {
            Content::Text(s) => s.len(),
            Content::Blocks(blocks) => blocks
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => text.len(),
                    ContentBlock::ImageUrl { .. } => 0,
                })
                .sum(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

pub enum LlmResponse {
    Text { content: String, vision_stripped: bool },
    ToolCalls { calls: Vec<ToolCall>, vision_stripped: bool },
}

// -- Internal request/response structs --

#[derive(Serialize)]
struct CompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Deserialize)]
struct CompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<Content>,
    tool_calls: Option<Vec<ToolCall>>,
}

// -- Think tag stripping --

static THINK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<think>.*?</think>").unwrap());

fn strip_think_tags(content: &str) -> String {
    THINK_RE.replace_all(content, "").trim().to_string()
}

// -- Main entry point --

pub async fn call_llm(
    client: &reqwest::Client,
    config: &LlmConfig,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<Value>>,
) -> Result<LlmResponse, String> {
    let url = format!("{}/chat/completions", config.api_base.trim_end_matches('/'));

    // First attempt: original messages (may contain images).
    let first = attempt_chat(client, &url, config, &messages, tools.as_ref()).await;

    match first {
        Ok(resp) => Ok(resp),
        Err(CallError::Transient(msg)) => Err(msg),
        Err(CallError::NonTransient(msg)) => {
            // Strip images and retry once if any image was in the payload.
            let mut stripped = messages.clone();
            if !strip_images_in_place(&mut stripped) {
                return Err(msg);
            }
            warn!("LLM non-transient error with images; retrying stripped");
            match attempt_chat(client, &url, config, &stripped, tools.as_ref()).await {
                Ok(mut r) => {
                    // Flip vision_stripped on successful retry.
                    match &mut r {
                        LlmResponse::Text { vision_stripped, .. } => *vision_stripped = true,
                        LlmResponse::ToolCalls { vision_stripped, .. } => *vision_stripped = true,
                    }
                    Ok(r)
                }
                Err(CallError::Transient(m)) | Err(CallError::NonTransient(m)) => Err(m),
            }
        }
    }
}

enum CallError {
    Transient(String),
    NonTransient(String),
}

async fn attempt_chat(
    client: &reqwest::Client,
    url: &str,
    config: &LlmConfig,
    messages: &[ChatMessage],
    tools: Option<&Vec<Value>>,
) -> Result<LlmResponse, CallError> {
    let tool_choice = if tools.is_some_and(|t| !t.is_empty()) {
        Some("auto".to_string())
    } else {
        None
    };

    let body = CompletionRequest {
        model: config.model.clone(),
        messages: messages.to_vec(),
        tools: tools.cloned(),
        tool_choice,
    };

    let mut last_error = String::new();

    for attempt in 0..3 {
        if attempt > 0 {
            let delay = 1u64 << attempt; // 1s, 2s, 4s
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            debug!("LLM retry attempt {attempt}");
        }

        let resp = match client
            .post(url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_error = format!("HTTP error: {e}");
                warn!("LLM request failed: {last_error}");
                continue; // transient network error
            }
        };

        let status = resp.status().as_u16();

        if status == 429 || status >= 500 {
            last_error = format!("HTTP {status}");
            warn!("LLM transient error: {last_error}");
            continue;
        }

        if status != 200 {
            let body_text = resp.text().await.unwrap_or_default();
            let msg = format!("HTTP {status}: {body_text}");
            warn!("LLM non-transient error: {msg}");
            return Err(CallError::NonTransient(msg));
        }

        let body_text = resp
            .text()
            .await
            .map_err(|e| CallError::NonTransient(format!("Read response body: {e}")))?;

        let parsed: CompletionResponse = serde_json::from_str(&body_text).map_err(|e| {
            CallError::NonTransient(format!(
                "Parse LLM response: {e}\nBody: {}",
                &body_text[..body_text.len().min(500)]
            ))
        })?;

        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| CallError::NonTransient("No choices in LLM response".into()))?;

        if let Some(tool_calls) = choice.message.tool_calls {
            if !tool_calls.is_empty() {
                return Ok(LlmResponse::ToolCalls { calls: tool_calls, vision_stripped: false });
            }
        }

        if let Some(content) = choice.message.content {
            let cleaned = strip_think_tags(&content.into_text());
            return Ok(LlmResponse::Text { content: cleaned, vision_stripped: false });
        }

        return Err(CallError::NonTransient(
            "LLM returned empty response (no content, no tool_calls)".into(),
        ));
    }

    Err(CallError::Transient(format!(
        "LLM failed after 3 attempts: {last_error}"
    )))
}

/// Replace every ContentBlock::ImageUrl in every user message with a text
/// placeholder. Returns true if any image was stripped.
pub(crate) fn strip_images_in_place(messages: &mut [ChatMessage]) -> bool {
    let mut found = false;
    for m in messages.iter_mut() {
        if m.role != "user" {
            continue;
        }
        if let Some(Content::Blocks(blocks)) = m.content.as_mut() {
            for b in blocks.iter_mut() {
                if let ContentBlock::ImageUrl { .. } = b {
                    *b = ContentBlock::Text {
                        text: "[Image omitted: model does not support vision]".into(),
                    };
                    found = true;
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_think_tags() {
        assert_eq!(strip_think_tags("hello"), "hello");
        assert_eq!(
            strip_think_tags("<think>reasoning here</think>answer"),
            "answer"
        );
        assert_eq!(
            strip_think_tags("<think>line1\nline2</think>\nresult"),
            "result"
        );
        assert_eq!(strip_think_tags("<think>all thinking</think>"), "");
    }

    #[test]
    fn test_chat_message_system() {
        let m = ChatMessage::system("hello");
        assert_eq!(m.role, "system");
        assert!(matches!(m.content.as_ref().unwrap(), Content::Text(t) if t == "hello"));
    }

    #[test]
    fn test_chat_message_user_serializes_as_text() {
        // Plain user constructor keeps string wire form for backwards compat;
        // user messages with blocks use user_with_blocks.
        let m = ChatMessage::user("hi");
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains(r#""role":"user""#));
        assert!(json.contains(r#""content":"hi""#));
        assert!(!json.contains("tool_calls"));
    }

    #[test]
    fn test_chat_message_user_with_blocks_serializes_as_array() {
        let m = ChatMessage::user_with_blocks(vec![
            ContentBlock::Text { text: "describe".into() },
            ContentBlock::ImageUrl {
                image_url: ImageUrl { url: "data:image/png;base64,AA".into() },
            },
        ]);
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains(r#""role":"user""#));
        assert!(json.contains(r#""content":[{"type":"text","text":"describe"}"#));
        assert!(json.contains(r#"{"type":"image_url","image_url":{"url":"data:image/png;base64,AA"}}]"#));
    }

    #[test]
    fn test_content_text_serializes_as_string() {
        let c = Content::Text("hello".into());
        assert_eq!(serde_json::to_string(&c).unwrap(), "\"hello\"");
    }

    #[test]
    fn test_content_blocks_serializes_as_array() {
        let c = Content::Blocks(vec![
            ContentBlock::Text { text: "hi".into() },
            ContentBlock::ImageUrl {
                image_url: ImageUrl {
                    url: "data:image/png;base64,AAAA".into(),
                },
            },
        ]);
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(
            json,
            r#"[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]"#
        );
    }

    #[test]
    fn test_content_deserializes_from_string_or_array() {
        let s: Content = serde_json::from_str("\"hello\"").unwrap();
        assert!(matches!(s, Content::Text(ref t) if t == "hello"));
        let a: Content = serde_json::from_str(r#"[{"type":"text","text":"hi"}]"#).unwrap();
        assert!(matches!(a, Content::Blocks(ref v) if v.len() == 1));
    }

    #[test]
    fn test_content_len_matches_as_text_len() {
        let text = Content::Text("hello".into());
        assert_eq!(text.len(), 5);

        let blocks = Content::Blocks(vec![
            ContentBlock::Text { text: "ab".into() },
            ContentBlock::ImageUrl {
                image_url: ImageUrl { url: "ignored".into() },
            },
            ContentBlock::Text { text: "cde".into() },
        ]);
        assert_eq!(blocks.len(), 5); // "ab" + "cde", image dropped
        assert_eq!(blocks.as_text().len(), 5);
    }

    #[test]
    fn test_content_into_text_consumes() {
        let c = Content::Text("hi".into());
        assert_eq!(c.into_text(), "hi");

        let c = Content::Blocks(vec![
            ContentBlock::Text { text: "a".into() },
            ContentBlock::ImageUrl {
                image_url: ImageUrl { url: "x".into() },
            },
            ContentBlock::Text { text: "b".into() },
        ]);
        assert_eq!(c.into_text(), "ab");
    }

    fn make_user_with_image() -> ChatMessage {
        ChatMessage::user_with_blocks(vec![
            ContentBlock::Text { text: "what is this".into() },
            ContentBlock::ImageUrl {
                image_url: ImageUrl { url: "data:image/png;base64,AA".into() },
            },
        ])
    }

    #[test]
    fn test_strip_images_in_place_replaces_image_blocks() {
        let mut msgs = vec![ChatMessage::system("hi"), make_user_with_image()];
        let had_images = strip_images_in_place(&mut msgs);
        assert!(had_images);
        // system untouched
        assert!(matches!(
            msgs[0].content.as_ref().unwrap(),
            Content::Text(t) if t == "hi"
        ));
        // user message: image block replaced with placeholder text block
        match msgs[1].content.as_ref().unwrap() {
            Content::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
                assert!(matches!(&blocks[0], ContentBlock::Text { text } if text == "what is this"));
                assert!(matches!(
                    &blocks[1],
                    ContentBlock::Text { text } if text.starts_with("[Image omitted")
                ));
            }
            _ => panic!("expected Blocks"),
        }
    }

    #[test]
    fn test_strip_images_in_place_returns_false_when_no_images() {
        let mut msgs = vec![ChatMessage::system("x"), ChatMessage::user("y")];
        assert!(!strip_images_in_place(&mut msgs));
    }

    #[test]
    fn test_llm_response_has_vision_stripped_flag() {
        let r = LlmResponse::Text {
            content: "hi".into(),
            vision_stripped: false,
        };
        match r {
            LlmResponse::Text { content, vision_stripped } => {
                assert_eq!(content, "hi");
                assert!(!vision_stripped);
            }
            _ => panic!("expected Text"),
        }
    }
}
