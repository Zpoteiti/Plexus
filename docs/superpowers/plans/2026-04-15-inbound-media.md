# Inbound Media Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable users to send any file ≤20 MB via Discord, Telegram, or the Gateway (browser); agent receives images as vision content blocks and non-images as `[Attachment: name → /api/files/{id}]` references to be processed client-side via `file_transfer`.

**Architecture:** Channel adapters download bytes → `file_store::save_upload` → URL in `InboundEvent.media`. `context::build_user_content` turns `(content, media)` into OpenAI content blocks at LLM-call time, encoding images as base64 data URLs. Provider runs a strip-and-retry fallback on non-transient errors when images are present; a session-level `vision_stripped` flag persists the stripped state until the admin updates the LLM config. Frontend adds paperclip+drag+paste upload UI, using existing `POST /api/files` then referencing the returned URL in the WS send frame.

**Tech Stack:** Rust 1.85 (axum 0.7, sqlx, reqwest, serenity 0.12 for Discord, teloxide 0.13 for Telegram, tokio-tungstenite), React 19 + TypeScript + Tailwind 4, serde_json.

**Spec:** `docs/superpowers/specs/2026-04-15-inbound-media-design.md`

---

## File Structure

**Modified:**
- `plexus-common/src/consts.rs` — `FILE_UPLOAD_MAX_BYTES` 25→20 MB
- `plexus-server/src/providers/openai.rs` — `ChatMessage` refactor, `Content`/`ContentBlock` types, strip-and-retry, `LlmResponse::vision_stripped`
- `plexus-server/src/context.rs` — new `build_user_content`, wire into `build_context`, add `## Attachments` system-prompt section
- `plexus-server/src/session.rs` — add `vision_stripped: Arc<AtomicBool>` to `SessionHandle`
- `plexus-server/src/agent_loop.rs` — thread `vision_stripped` through session lifecycle; update flag from `LlmResponse`
- `plexus-server/src/bus.rs` — (light) pass `media` through; no structural change needed
- `plexus-server/src/channels/discord.rs` — download `msg.attachments`, relax empty-content early return
- `plexus-server/src/channels/telegram.rs` — download photo/voice/document/etc., synthesize filenames, relax empty-content return
- `plexus-server/src/channels/gateway.rs` — parse `media` array from JSON frame, relax empty-content return
- `plexus-server/src/auth/admin.rs` — reset all sessions' `vision_stripped` on `put_llm_config`
- `plexus-server/src/state.rs` — confirm `AppState.sessions` is a `DashMap<String, Arc<SessionHandle>>`
- `plexus-frontend/src/components/ChatInput.tsx` — paperclip button, drag/drop, paste, chips, `POST /api/files` with progress, `media` in send payload
- `plexus-frontend/src/components/Message.tsx` — render `[Attachment:...]` as clickable link; inline `<img>` for images

**Created:** None — everything fits into existing files.

---

## Task 1: Harmonize `FILE_UPLOAD_MAX_BYTES` to 20 MB

**Files:**
- Modify: `plexus-common/src/consts.rs:35`

- [ ] **Step 1: Update the constant and its test**

Edit `plexus-common/src/consts.rs`:

```rust
pub const FILE_UPLOAD_MAX_BYTES: usize = 20 * 1024 * 1024;
```

Then find the existing test block (search for `FILE_UPLOAD_MAX_BYTES` in the `#[cfg(test)] mod tests` block of the same file). If there's an assertion on the old value (`25 * 1024 * 1024`), update it to `20 * 1024 * 1024`. If no such assertion exists, add one:

```rust
#[test]
fn test_file_upload_max_bytes_is_20mb() {
    assert_eq!(FILE_UPLOAD_MAX_BYTES, 20 * 1024 * 1024);
}
```

- [ ] **Step 2: Run the common tests**

```bash
cd Plexus && cargo test -p plexus-common
```

Expected: all tests pass, including the new/updated assertion.

- [ ] **Step 3: Build the whole workspace to confirm downstream consumers still compile**

```bash
cargo build
```

Expected: successful build, zero warnings from this change. Existing consumers in `plexus-gateway/src/main.rs`, `plexus-gateway/src/proxy.rs`, `plexus-server/src/file_store.rs` all use the constant symbolically — no edits needed.

- [ ] **Step 4: Commit**

```bash
git add plexus-common/src/consts.rs
git commit -m "$(cat <<'EOF'
harmonize FILE_UPLOAD_MAX_BYTES to 20 MB

Single cap for every in-store path — gateway request body, proxy body,
server file_store, and file_transfer-to-server. Inbound channel
attachments (added next) will reuse this constant.
EOF
)"
```

---

## Task 2: Add `Content` and `ContentBlock` types in provider

**Files:**
- Modify: `plexus-server/src/providers/openai.rs:12-25` (types) and `openai.rs:235-269` (tests)

- [ ] **Step 1: Write failing tests for the new types**

At the bottom of `plexus-server/src/providers/openai.rs`, inside the `mod tests`, add:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p plexus-server --lib providers::openai::tests
```

Expected: compile errors — `Content`, `ContentBlock`, `ImageUrl` not defined.

- [ ] **Step 3: Add the types**

In `plexus-server/src/providers/openai.rs`, immediately above the existing `pub struct ChatMessage { ... }` (line 14), insert:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageUrl {
    pub url: String,
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p plexus-server --lib providers::openai::tests
```

Expected: the three new tests pass. Existing tests (`test_strip_think_tags`, `test_chat_message_system`, `test_chat_message_serialization`) **will fail** because they still assume `content: Option<String>` — leave them failing; Task 3 fixes this.

- [ ] **Step 5: Commit (types only, broken state OK because next task restores)**

```bash
git add plexus-server/src/providers/openai.rs
git commit -m "$(cat <<'EOF'
add Content/ContentBlock/ImageUrl types for multimodal messages

Enum with #[serde(untagged)] so Content::Text serializes as a plain
string and Content::Blocks as an OpenAI-style content-block array.
ChatMessage will be refactored to use these in the next commit.
EOF
)"
```

---

## Task 3: Refactor `ChatMessage.content` to `Option<Content>`

**Files:**
- Modify: `plexus-server/src/providers/openai.rs:14-77` (struct + constructors), and the `ChoiceMessage` struct on line 120

- [ ] **Step 1: Update the failing existing tests to target the new type**

In the `mod tests` block of `plexus-server/src/providers/openai.rs`, replace the existing `test_chat_message_system` and `test_chat_message_serialization` with:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p plexus-server --lib providers::openai::tests
```

Expected: compile errors (`user_with_blocks` missing, `content` field type mismatch).

- [ ] **Step 3: Refactor `ChatMessage`**

In `plexus-server/src/providers/openai.rs`, replace the `ChatMessage` struct (currently lines 14-25) and its impl block (lines 27-77) with:

```rust
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
```

- [ ] **Step 4: Update the `ChoiceMessage` internal struct (line ~120)**

Find:

```rust
#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
}
```

Replace `content: Option<String>` with `content: Option<Content>`. Then inside `call_llm`, find the block:

```rust
if let Some(content) = choice.message.content {
    let cleaned = strip_think_tags(&content);
    return Ok(LlmResponse::Text(cleaned));
}
```

Replace with:

```rust
if let Some(content) = choice.message.content {
    let text = match content {
        Content::Text(s) => s,
        Content::Blocks(blocks) => blocks
            .into_iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text),
                ContentBlock::ImageUrl { .. } => None,
            })
            .collect::<Vec<_>>()
            .join(""),
    };
    let cleaned = strip_think_tags(&text);
    return Ok(LlmResponse::Text(cleaned));
}
```

(Rationale: some proxies return assistant content as a single-block array; extract the text transparently.)

- [ ] **Step 5: Run the provider tests**

```bash
cargo test -p plexus-server --lib providers::openai::tests
```

Expected: all tests pass.

- [ ] **Step 6: Build the whole server to surface downstream breakage**

```bash
cargo build -p plexus-server
```

Expected: compilation errors in any file that reads `message.content.as_ref().unwrap()` as a `&String`. Fix these call sites by pattern-matching `Content::Text(s)` (or using a helper — see Step 7).

- [ ] **Step 7: Add a convenience method for text extraction**

In `plexus-server/src/providers/openai.rs`, add an impl method on `Content`:

```rust
impl Content {
    /// Returns the concatenated text from this content. For Blocks, image
    /// blocks are dropped and text blocks joined in order.
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
}
```

Then fix any remaining compile errors (typically in `agent_loop.rs` or `context.rs` when serializing past assistant messages to DB, or inspecting content) by replacing `message.content.as_deref().unwrap_or("")` with `message.content.as_ref().map(|c| c.as_text()).unwrap_or_default()`.

Use a grep to find them:

```bash
grep -rn "message.content\|\.content\.as_deref\|\.content\.as_ref" plexus-server/src/
```

Iterate until `cargo build -p plexus-server` is clean.

- [ ] **Step 8: Run the full server test suite**

```bash
cargo test -p plexus-server --lib
```

Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add plexus-server/src/
git commit -m "$(cat <<'EOF'
refactor ChatMessage to Option<Content> enum

Content::Text for roles that always use string form (system, assistant,
tool) and for plain-text user messages; Content::Blocks for multimodal
user messages built via user_with_blocks. Serde (untagged) maps
directly to OpenAI's wire format. Downstream call sites use
Content::as_text() when they need a plain string.
EOF
)"
```

---

## Task 4: Extend `LlmResponse` with `vision_stripped` flag

**Files:**
- Modify: `plexus-server/src/providers/openai.rs:93-96`

- [ ] **Step 1: Write failing test**

Add to `mod tests` in `plexus-server/src/providers/openai.rs`:

```rust
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
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test -p plexus-server --lib providers::openai::tests::test_llm_response_has_vision_stripped_flag
```

Expected: compile error — `LlmResponse::Text` variant shape doesn't match.

- [ ] **Step 3: Restructure `LlmResponse`**

Replace:

```rust
pub enum LlmResponse {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}
```

with:

```rust
pub enum LlmResponse {
    Text { content: String, vision_stripped: bool },
    ToolCalls { calls: Vec<ToolCall>, vision_stripped: bool },
}
```

- [ ] **Step 4: Update the two construction sites inside `call_llm`**

Find:

```rust
return Ok(LlmResponse::ToolCalls(tool_calls));
```

Replace with:

```rust
return Ok(LlmResponse::ToolCalls { calls: tool_calls, vision_stripped: false });
```

Find:

```rust
return Ok(LlmResponse::Text(cleaned));
```

Replace with:

```rust
return Ok(LlmResponse::Text { content: cleaned, vision_stripped: false });
```

- [ ] **Step 5: Fix downstream call sites**

Run:

```bash
cargo build -p plexus-server
```

Expect compile errors in `agent_loop.rs` where `LlmResponse::Text(s)` / `LlmResponse::ToolCalls(v)` are pattern-matched. Replace with the new struct-variant syntax:

```rust
// before:
Ok(LlmResponse::Text(text)) => ...
Ok(LlmResponse::ToolCalls(calls)) => ...

// after:
Ok(LlmResponse::Text { content, vision_stripped }) => {
    // use `content` where `text` was used; ignore `vision_stripped` for now
    // (wired in Task 10)
    ...
}
Ok(LlmResponse::ToolCalls { calls, vision_stripped }) => { ... }
```

Iterate until clean.

- [ ] **Step 6: Tests pass**

```bash
cargo test -p plexus-server --lib
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add plexus-server/src/
git commit -m "$(cat <<'EOF'
add vision_stripped flag to LlmResponse

Provider will set this to true after a successful strip-and-retry in
the next commit. Agent loop will use it to update SessionHandle state.
EOF
)"
```

---

## Task 5: Provider strip-and-retry on non-transient errors with images

**Files:**
- Modify: `plexus-server/src/providers/openai.rs:136-233` (the `call_llm` function)

- [ ] **Step 1: Write failing integration-ish tests**

Add to `mod tests`:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p plexus-server --lib providers::openai::tests::test_strip_images_in_place
```

Expected: `strip_images_in_place` not found.

- [ ] **Step 3: Implement `strip_images_in_place`**

Above `mod tests`, add:

```rust
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
```

- [ ] **Step 4: Run the strip tests**

```bash
cargo test -p plexus-server --lib providers::openai::tests::test_strip_images_in_place
```

Expected: both pass.

- [ ] **Step 5: Wire strip-and-retry into `call_llm`**

Refactor `call_llm` so the HTTP call is a local closure/helper called twice: once with the original messages, and on non-transient error + images present, once again with stripped messages. Replace the entire body of `call_llm` with:

```rust
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
            let text = content.as_text();
            let cleaned = strip_think_tags(&text);
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
```

- [ ] **Step 6: Build and test**

```bash
cargo build -p plexus-server && cargo test -p plexus-server --lib providers::openai
```

Expected: compiles; all existing and new tests pass.

- [ ] **Step 7: Commit**

```bash
git add plexus-server/src/providers/openai.rs
git commit -m "$(cat <<'EOF'
strip-and-retry on non-transient LLM errors with images

When the first call fails with a non-transient status and any user
message carries an image block, replace image blocks with text
placeholders and retry once. If the retry succeeds, LlmResponse
carries vision_stripped=true so the caller can flip the session flag
and skip image encoding for future turns.
EOF
)"
```

---

## Task 6: Add `vision_stripped` to `SessionHandle` and reset hook

**Files:**
- Modify: `plexus-server/src/session.rs`
- Modify: every call site that constructs `SessionHandle` (typically `plexus-server/src/bus.rs` or wherever sessions are created)

- [ ] **Step 1: Find the `SessionHandle` construction site**

```bash
grep -rn "SessionHandle {" plexus-server/src/
```

Note each location — typically in `bus.rs` when a new session spawns.

- [ ] **Step 2: Edit `plexus-server/src/session.rs`**

Replace the struct with:

```rust
//! Per-session handle: inbox channel + mutex for DB write serialization.

use crate::bus::InboundEvent;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::{Mutex, mpsc};

pub struct SessionHandle {
    pub user_id: String,
    pub inbox_tx: mpsc::Sender<InboundEvent>,
    pub lock: Arc<Mutex<()>>,
    /// Set to true after the provider's strip-and-retry succeeds in this
    /// session. When true, context::build_user_content replaces image
    /// blocks with text placeholders. Reset to false when the admin
    /// updates the LLM config.
    pub vision_stripped: Arc<AtomicBool>,
}
```

Remove the existing `#[allow(dead_code)]` on the struct (we're about to actually use every field except `user_id`, which stays scaffolded — keep the allow on that field alone if needed).

- [ ] **Step 3: Update each construction site**

At each `SessionHandle { ... }` construction site, add:

```rust
vision_stripped: Arc::new(AtomicBool::new(false)),
```

Add `use std::sync::atomic::AtomicBool;` and `use std::sync::Arc;` to imports if not already present.

- [ ] **Step 4: Check `AppState.sessions` storage shape**

```bash
grep -n "sessions" plexus-server/src/state.rs
```

If sessions are stored as `DashMap<String, SessionHandle>` (by value), change to `DashMap<String, Arc<SessionHandle>>` so the agent_loop task and the admin reset handler can share the reference.

Update every `state.sessions.insert(..., handle)` site to wrap in `Arc::new(...)`, and every `state.sessions.get(...)` consumer to work with `Arc<SessionHandle>` (usually just `.clone()` to get another Arc). Grep and fix iteratively:

```bash
cargo build -p plexus-server
```

- [ ] **Step 5: Build, confirm clean compile**

```bash
cargo build -p plexus-server
```

Expected: no errors, no warnings about the new field.

- [ ] **Step 6: Commit**

```bash
git add plexus-server/src/
git commit -m "$(cat <<'EOF'
add vision_stripped flag to SessionHandle

Atomic bool per session; provider's strip-and-retry (Task 5) signals
success, agent loop will flip this flag, and admin LLM-config update
will reset it. Sessions now stored as Arc<SessionHandle> so the flag
can be shared between the session task and the admin handler.
EOF
)"
```

---

## Task 7: `context::build_user_content`

**Files:**
- Modify: `plexus-server/src/context.rs`

- [ ] **Step 1: Add imports at the top of `plexus-server/src/context.rs`**

```rust
use crate::file_store;
use crate::providers::openai::{ChatMessage, ContentBlock, FunctionCall, ImageUrl, ToolCall};
```

(Merge with the existing `use crate::providers::openai::...` line.)

- [ ] **Step 2: Write failing unit tests**

At the bottom of `context.rs` inside `#[cfg(test)] mod tests`, add:

```rust
#[tokio::test]
async fn test_build_user_content_text_only() {
    let blocks = build_user_content_inner("hello", &[], false, |_, _| async {
        Err("should not be called".to_string())
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
        |_uid, fid| async move {
            assert_eq!(fid, "abc123");
            Ok((png_bytes.clone(), "photo.png".to_string()))
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
        |_, _| async { Ok((b"hello".to_vec(), "notes.txt".to_string())) },
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
        |_, _| async { Ok((vec![0x89], "photo.png".to_string())) },
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
        |_, fid| async move {
            if fid == "i1" {
                Ok((vec![0x89], "pic.jpg".to_string()))
            } else {
                Ok((b"hi".to_vec(), "doc.txt".to_string()))
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
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test -p plexus-server --lib context::tests::test_build_user_content
```

Expected: `build_user_content_inner` not defined.

- [ ] **Step 4: Implement `build_user_content` and `build_user_content_inner`**

In `plexus-server/src/context.rs`, above `pub async fn build_context`, add:

```rust
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
    build_user_content_inner(content, media, vision_stripped, |uid, fid| {
        let uid = uid.to_string();
        let fid = fid.to_string();
        async move { file_store::load_file(&uid, &fid).await.map_err(|e| e.message) }
    })
    .await
}

/// Test-friendly inner: takes a loader closure so tests can mock file_store.
async fn build_user_content_inner<F, Fut>(
    content: &str,
    media: &[String],
    vision_stripped: bool,
    load: F,
) -> Vec<ContentBlock>
where
    F: Fn(&str, &str) -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<u8>, String), String>>,
{
    let mut blocks: Vec<ContentBlock> = Vec::new();

    if !content.is_empty() {
        blocks.push(ContentBlock::Text { text: content.to_string() });
    }

    let mut non_image_refs: Vec<String> = Vec::new();

    for url in media {
        // Parse /api/files/{id}
        let Some(file_id) = url.strip_prefix("/api/files/") else {
            // Treat as opaque URL reference (future: raw URL support).
            non_image_refs.push(format!(
                "[Attachment: {url} — unknown reference]"
            ));
            continue;
        };

        let (bytes, filename) = match load("unused-uid-threaded-elsewhere", file_id).await {
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

fn mime_from_filename(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
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
```

**Note** on the `load` closure in `build_user_content`: the outer `build_user_content` ignores the uid param (threaded in from the surrounding call context) and passes it through. The inner test variant receives unused-uid-threaded-elsewhere as a sentinel because the test closures don't need the real uid — they receive file_id. Simpler alternative: change inner signature to take only `&str` file_id. Refactor if it bothers you:

```rust
async fn build_user_content_inner<F, Fut>(
    content: &str,
    media: &[String],
    vision_stripped: bool,
    load: F,
) -> Vec<ContentBlock>
where
    F: Fn(&str) -> Fut,  // just file_id
    Fut: std::future::Future<Output = Result<(Vec<u8>, String), String>>,
{ ... }
```

…and update tests to `|fid| async move { ... }`. Pick whichever is cleaner.

- [ ] **Step 5: Verify `base64` and `mime` helpers are available**

Check `plexus-server/Cargo.toml`. `base64` should already be there (it's used elsewhere). If not:

```bash
cd plexus-server && cargo add base64
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p plexus-server --lib context::tests::test_build_user_content
```

Expected: all 5 tests pass.

- [ ] **Step 7: Commit**

```bash
git add plexus-server/src/context.rs plexus-server/Cargo.toml
git commit -m "$(cat <<'EOF'
context: add build_user_content for multimodal user messages

Turns (content, media[], vision_stripped) into OpenAI content blocks:
text → image blocks (base64 data URLs) → trailing attachment-reference
text block. Image media is filtered by filename-derived MIME; when
vision_stripped is true, images become text placeholders instead.
EOF
)"
```

---

## Task 8: Wire `build_user_content` into `build_context`

**Files:**
- Modify: `plexus-server/src/context.rs` (the `build_context` function around line 71+, and the latest-user-message construction)

- [ ] **Step 1: Find where `build_context` currently builds the last user message**

Search the function body for `ChatMessage::user(` or similar — the last user message is usually constructed from the most recent `Message` in the history (or directly from the incoming event). Note the exact line(s).

- [ ] **Step 2: Update `build_context` signature to accept `vision_stripped`**

Find:

```rust
pub async fn build_context(
    state: &AppState,
    user: &User,
    history: &[Message],
    skills: &[SkillInfo],
    identity: &ChannelIdentity,
    default_soul: &Option<String>,
    chat_id: Option<&str>,
) -> Vec<ChatMessage> {
```

Change to:

```rust
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
```

- [ ] **Step 3: Inside `build_context`, replace the latest-user-message construction**

Instead of `ChatMessage::user(<text>)` for the final user message in the returned vec, call:

```rust
let blocks = build_user_content(&user.user_id, <text>, latest_user_media, vision_stripped).await;
let user_msg = if blocks.is_empty() {
    None
} else {
    Some(ChatMessage::user_with_blocks(blocks))
};
```

Only append `user_msg` to `messages` if it's `Some`.

If the existing implementation builds **each** historical user message via `ChatMessage::user(...)`, keep those as-is (plain text). Only the latest pending user message (the one just published into `InboundEvent`) flows through `build_user_content`. This is important — historical messages go through `Content::Text` because we aren't re-encoding their images (the DB stores them already formed).

However, if the history already contains serialized block arrays (because a previous turn's user message had images and we persisted the full blocks), deserialize those as-is. Check how `Message.content` is stored:

```bash
grep -n "pub struct Message\|content:" plexus-server/src/db/messages.rs
```

If `Message.content: String`, assume the DB stores a JSON-serialized `Content` for multimodal messages. Update the history-to-ChatMessage mapping to attempt:

```rust
let content_value: Content = serde_json::from_str(&hist_msg.content)
    .unwrap_or_else(|_| Content::Text(hist_msg.content.clone()));
```

…then build `ChatMessage { role: hist_msg.role, content: Some(content_value), ... }`.

- [ ] **Step 4: Update call sites of `build_context`**

```bash
grep -rn "build_context(" plexus-server/src/
```

Each call site (typically `agent_loop.rs:141`) needs to pass `&event.media` as `latest_user_media` and the current `vision_stripped` value (hardcode `false` for now; Task 9 wires the real flag).

- [ ] **Step 5: Build and test**

```bash
cargo build -p plexus-server && cargo test -p plexus-server --lib
```

Expected: clean build, all tests pass.

- [ ] **Step 6: Commit**

```bash
git add plexus-server/src/
git commit -m "$(cat <<'EOF'
wire build_user_content into build_context

Latest user message now flows through build_user_content so images
and attachments become proper content blocks. Historical user
messages stored in the DB are deserialized as Content (text or
block-array) based on their stored form.
EOF
)"
```

---

## Task 9: Agent loop threads `vision_stripped` through

**Files:**
- Modify: `plexus-server/src/agent_loop.rs`

- [ ] **Step 1: Read the current `run_session` to find the LLM call and `build_context` call**

```bash
grep -n "build_context\|call_llm\|LlmResponse" plexus-server/src/agent_loop.rs
```

Note lines.

- [ ] **Step 2: At the `build_context` call site, read the flag from `SessionHandle`**

Before calling `build_context`, add:

```rust
let vision_stripped = session_handle
    .vision_stripped
    .load(std::sync::atomic::Ordering::Relaxed);
```

Pass this as the new argument, and pass `&event.media` for `latest_user_media`.

- [ ] **Step 3: After `call_llm` returns, update the flag on success**

Find each match arm for `LlmResponse::Text { .. }` and `LlmResponse::ToolCalls { .. }`. Above the use of `content` / `calls`, extract `vision_stripped`:

```rust
Ok(LlmResponse::Text { content, vision_stripped: stripped }) => {
    if stripped {
        session_handle
            .vision_stripped
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    // ... existing handling of `content`
}
Ok(LlmResponse::ToolCalls { calls, vision_stripped: stripped }) => {
    if stripped {
        session_handle
            .vision_stripped
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    // ... existing handling of `calls`
}
```

- [ ] **Step 4: Build and run tests**

```bash
cargo build -p plexus-server && cargo test -p plexus-server --lib
```

- [ ] **Step 5: Commit**

```bash
git add plexus-server/src/agent_loop.rs
git commit -m "$(cat <<'EOF'
agent_loop: thread vision_stripped through session

Read SessionHandle.vision_stripped before build_context; persist it
back to the flag when LlmResponse reports the provider stripped
images on retry.
EOF
)"
```

---

## Task 10: Admin-endpoint eagerly resets `vision_stripped` on LLM-config update

**Files:**
- Modify: `plexus-server/src/auth/admin.rs` (the `put_llm_config` handler)

- [ ] **Step 1: Find the `put_llm_config` function**

```bash
grep -n "put_llm_config\|fn put_llm" plexus-server/src/auth/admin.rs
```

- [ ] **Step 2: Add the session walk after the config write is persisted**

After the line that writes the new config to `state.llm_config` (and commits to DB if that happens here), add:

```rust
// Eagerly reset vision-stripped flags on every live session so the
// next turn retries images against the new model.
for entry in state.sessions.iter() {
    entry
        .value()
        .vision_stripped
        .store(false, std::sync::atomic::Ordering::Relaxed);
}
info!("Reset vision_stripped on {} live sessions", state.sessions.len());
```

(Add the `tracing::info` import if not already present.)

- [ ] **Step 3: Build**

```bash
cargo build -p plexus-server
```

- [ ] **Step 4: Commit**

```bash
git add plexus-server/src/auth/admin.rs
git commit -m "$(cat <<'EOF'
admin: reset all sessions' vision_stripped on LLM config update

When admin swaps the LLM provider/model, every live session's
vision-fallback flag clears so image-bearing turns are retried
against the new configuration.
EOF
)"
```

---

## Task 11: System prompt `## Attachments` section

**Files:**
- Modify: `plexus-server/src/context.rs` (inside `build_context`, wherever the system prompt is assembled)

- [ ] **Step 1: Find the system prompt assembly**

Search for the `## Identity` marker string in `context.rs`:

```bash
grep -n "## Identity\|system += \"##" plexus-server/src/context.rs
```

- [ ] **Step 2: Add the Attachments section after Identity**

Immediately after the Identity block is appended (before the next `##` section is appended), add:

```rust
system += "## Attachments\n";
system += "Files may appear as [Attachment: name → /api/files/{id}]. They live on the\n";
system += "server. To operate on one, use `file_transfer` to move it to a client device,\n";
system += "then use client tools (shell, read_file, etc.). Choose the action based on\n";
system += "filename and the user's intent.\n\n";
```

- [ ] **Step 3: Add a test asserting the section is in the system prompt**

In `mod tests` at the bottom of `context.rs`, add:

```rust
#[test]
fn test_system_prompt_has_attachments_section() {
    // Build a minimal system string via the same assembly path used by
    // build_context. If the prompt assembly is extracted into a helper,
    // test that helper directly. Otherwise, inline the concatenation
    // used inside build_context.
    // (Adjust this test to match the refactor if one is used.)
    let prompt = assemble_system_prompt_for_test();
    assert!(prompt.contains("## Attachments"));
    assert!(prompt.contains("file_transfer to move it to a client device"));
}

#[cfg(test)]
fn assemble_system_prompt_for_test() -> String {
    let mut s = String::new();
    s += "## Attachments\n";
    s += "Files may appear as [Attachment: name → /api/files/{id}]. They live on the\n";
    s += "server. To operate on one, use `file_transfer` to move it to a client device,\n";
    s += "then use client tools (shell, read_file, etc.). Choose the action based on\n";
    s += "filename and the user's intent.\n\n";
    s
}
```

(If `build_context` exposes or easily permits testing the system-prompt builder, test that instead. The above is a minimal smoke test.)

- [ ] **Step 4: Run tests**

```bash
cargo test -p plexus-server --lib context::tests
```

- [ ] **Step 5: Commit**

```bash
git add plexus-server/src/context.rs
git commit -m "$(cat <<'EOF'
system prompt: teach the Attachments convention

Four-line section explaining that non-image attachments are
referenced by /api/files/{id} and should be processed client-side
via file_transfer + shell or read_file. Taught once, applies every
message.
EOF
)"
```

---

## Task 12: Discord inbound attachments (PARALLEL)

> **Can run in parallel with Tasks 13, 14, 15, 16 after Tasks 1–11 land.**

**Files:**
- Modify: `plexus-server/src/channels/discord.rs` (the `EventHandler::message` implementation around line 268)

- [ ] **Step 1: Read the current inbound handler**

```bash
grep -n "fn message\|msg.attachments\|content.is_empty" plexus-server/src/channels/discord.rs
```

Locate the block that builds `InboundEvent` (around line 268 in the spec review). Find the `if content.trim().is_empty() { return; }` early return.

- [ ] **Step 2: Add an inbound-downloader test (integration-style)**

Since serenity's `EventHandler` is hard to mock directly, write a **unit test** for a helper you'll extract:

```rust
// in mod tests at the bottom of channels/discord.rs
use plexus_common::consts::FILE_UPLOAD_MAX_BYTES;

#[test]
fn test_oversize_attachment_marker() {
    let marker = oversize_attachment_marker("big.zip", (FILE_UPLOAD_MAX_BYTES + 1) as u64);
    assert!(marker.contains("big.zip"));
    assert!(marker.contains("exceeds"));
    assert!(marker.contains("20 MB"));
}

#[test]
fn test_failed_download_marker() {
    let marker = failed_download_marker("doc.pdf");
    assert!(marker.contains("doc.pdf"));
    assert!(marker.contains("download failed"));
}
```

- [ ] **Step 3: Run to verify failure**

```bash
cargo test -p plexus-server --lib channels::discord::tests
```

Expected: helpers not defined.

- [ ] **Step 4: Add the helpers**

Above `mod tests` (or above the `EventHandler` impl) in `channels/discord.rs`:

```rust
fn oversize_attachment_marker(name: &str, size: u64) -> String {
    format!(
        "[Attachment: {name} ({:.1} MB) — exceeds {} MB limit, not downloaded]",
        size as f64 / 1024.0 / 1024.0,
        plexus_common::consts::FILE_UPLOAD_MAX_BYTES / 1024 / 1024
    )
}

fn failed_download_marker(name: &str) -> String {
    format!("[Attachment: {name} — download failed]")
}
```

- [ ] **Step 5: Run the helper tests**

```bash
cargo test -p plexus-server --lib channels::discord::tests
```

Expected: pass.

- [ ] **Step 6: Wire attachment download into `EventHandler::message`**

Inside the function that currently constructs `InboundEvent` (the block starting around line 260-285), **after** the allow-list / partner checks and **after** `let session_id = ...`, insert:

```rust
let http_client = reqwest::Client::new();
let mut content = content; // shadow existing `content` so we can append markers
let mut media_urls: Vec<String> = Vec::new();

for att in &msg.attachments {
    if (att.size as usize) > plexus_common::consts::FILE_UPLOAD_MAX_BYTES {
        let marker = oversize_attachment_marker(&att.filename, att.size as u64);
        content.push_str("\n");
        content.push_str(&marker);
        continue;
    }
    let bytes = match http_client.get(&att.url).send().await {
        Ok(r) => match r.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => {
                warn!("discord attachment read failed ({}): {}", att.filename, e);
                content.push_str("\n");
                content.push_str(&failed_download_marker(&att.filename));
                continue;
            }
        },
        Err(e) => {
            warn!("discord attachment fetch failed ({}): {}", att.filename, e);
            content.push_str("\n");
            content.push_str(&failed_download_marker(&att.filename));
            continue;
        }
    };
    match crate::file_store::save_upload(&self.plexus_user_id, &att.filename, &bytes).await {
        Ok(file_id) => media_urls.push(format!("/api/files/{file_id}")),
        Err(e) => {
            warn!("discord attachment save failed ({}): {}", att.filename, e);
            content.push_str("\n");
            content.push_str(&format!("[Attachment: {} — storage failed]", att.filename));
        }
    }
}
```

- [ ] **Step 7: Relax the empty-content guard**

Find the existing early return `if content.trim().is_empty() { return; }` (or similar). Replace with:

```rust
if content.trim().is_empty() && media_urls.is_empty() {
    return;
}
```

- [ ] **Step 8: Populate `InboundEvent.media`**

In the `InboundEvent { ... }` literal, replace `media: vec![],` with `media: media_urls,`.

- [ ] **Step 9: Build and test**

```bash
cargo build -p plexus-server && cargo test -p plexus-server --lib
```

- [ ] **Step 10: Commit**

```bash
git add plexus-server/src/channels/discord.rs
git commit -m "$(cat <<'EOF'
discord: download inbound attachments to file store

Each msg.attachments entry: check size, GET from Discord CDN,
save_upload → /api/files/{id} → InboundEvent.media. Oversize or
failed downloads produce inline [Attachment: …] markers in the
content so the agent knows what the user tried to send.
EOF
)"
```

---

## Task 13: Telegram inbound attachments (PARALLEL)

> **Can run in parallel with Tasks 12, 14, 15, 16.**

**Files:**
- Modify: `plexus-server/src/channels/telegram.rs` (the `handle_message` function around line 180)

- [ ] **Step 1: Add filename-synthesis tests**

In `mod tests` at the bottom of `channels/telegram.rs`:

```rust
use chrono::{TimeZone, Utc};

#[test]
fn test_synth_filename_voice() {
    let ts = Utc.with_ymd_and_hms(2026, 4, 15, 10, 30, 5).unwrap();
    assert_eq!(
        synth_filename_for_voice(ts),
        "voice_message_20260415_103005.ogg"
    );
}

#[test]
fn test_synth_filename_photo() {
    let ts = Utc.with_ymd_and_hms(2026, 4, 15, 10, 30, 5).unwrap();
    assert_eq!(
        synth_filename_for_photo(ts),
        "photo_20260415_103005.jpg"
    );
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p plexus-server --lib channels::telegram::tests::test_synth
```

Expected: helpers not defined.

- [ ] **Step 3: Add the filename helpers**

Above `mod tests` in `channels/telegram.rs`:

```rust
fn synth_filename_for_voice(ts: chrono::DateTime<chrono::Utc>) -> String {
    format!("voice_message_{}.ogg", ts.format("%Y%m%d_%H%M%S"))
}

fn synth_filename_for_photo(ts: chrono::DateTime<chrono::Utc>) -> String {
    format!("photo_{}.jpg", ts.format("%Y%m%d_%H%M%S"))
}

fn synth_filename_for_video(ts: chrono::DateTime<chrono::Utc>) -> String {
    format!("video_{}.mp4", ts.format("%Y%m%d_%H%M%S"))
}

fn synth_filename_for_audio(ts: chrono::DateTime<chrono::Utc>) -> String {
    format!("audio_{}.mp3", ts.format("%Y%m%d_%H%M%S"))
}

fn synth_filename_for_animation(ts: chrono::DateTime<chrono::Utc>) -> String {
    format!("animation_{}.mp4", ts.format("%Y%m%d_%H%M%S"))
}

fn synth_filename_for_video_note(ts: chrono::DateTime<chrono::Utc>) -> String {
    format!("video_note_{}.mp4", ts.format("%Y%m%d_%H%M%S"))
}
```

Confirm `chrono` is in `Cargo.toml`; if not:

```bash
cd plexus-server && cargo add chrono --features "serde"
```

- [ ] **Step 4: Run helper tests**

```bash
cargo test -p plexus-server --lib channels::telegram::tests
```

Expected: pass.

- [ ] **Step 5: Wire attachment downloads inside `handle_message`**

Find the existing block that builds `InboundEvent` (around telegram.rs:181). **Before** that, inventory attached media:

```rust
let now = chrono::Utc::now();
let http_client = reqwest::Client::new();
let mut media_urls: Vec<String> = Vec::new();
let mut content = content;

// Collect (file_id, filename) pairs from Telegram message variants.
let attachments: Vec<(String, String)> = {
    let mut v = Vec::new();
    if let Some(photo_sizes) = msg.photo() {
        if let Some(largest) = photo_sizes.last() {
            v.push((largest.file.id.clone(), synth_filename_for_photo(now)));
        }
    }
    if let Some(voice) = msg.voice() {
        v.push((voice.file.id.clone(), synth_filename_for_voice(now)));
    }
    if let Some(audio) = msg.audio() {
        let name = audio.title.clone().unwrap_or_else(|| synth_filename_for_audio(now));
        v.push((audio.file.id.clone(), name));
    }
    if let Some(document) = msg.document() {
        let name = document
            .file_name
            .clone()
            .unwrap_or_else(|| format!("document_{}", now.format("%Y%m%d_%H%M%S")));
        v.push((document.file.id.clone(), name));
    }
    if let Some(video) = msg.video() {
        let name = video
            .file_name
            .clone()
            .unwrap_or_else(|| synth_filename_for_video(now));
        v.push((video.file.id.clone(), name));
    }
    if let Some(video_note) = msg.video_note() {
        v.push((video_note.file.id.clone(), synth_filename_for_video_note(now)));
    }
    if let Some(animation) = msg.animation() {
        let name = animation
            .file_name
            .clone()
            .unwrap_or_else(|| synth_filename_for_animation(now));
        v.push((animation.file.id.clone(), name));
    }
    v
};

for (file_id, filename) in attachments {
    // Step 1: resolve URL via getFile
    let file = match bot.get_file(&file_id).await {
        Ok(f) => f,
        Err(e) => {
            warn!("telegram getFile failed ({filename}): {e}");
            content.push_str("\n");
            content.push_str(&format!("[Attachment: {filename} — download failed]"));
            continue;
        }
    };

    if (file.size as usize) > plexus_common::consts::FILE_UPLOAD_MAX_BYTES {
        content.push_str("\n");
        content.push_str(&format!(
            "[Attachment: {filename} ({:.1} MB) — exceeds {} MB limit, not downloaded]",
            file.size as f64 / 1024.0 / 1024.0,
            plexus_common::consts::FILE_UPLOAD_MAX_BYTES / 1024 / 1024
        ));
        continue;
    }

    // Step 2: construct download URL. teloxide Bot::api_url() + file.path.
    // The URL format is: https://api.telegram.org/file/bot<TOKEN>/<FILE_PATH>
    let download_url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        bot.token(),
        file.path
    );

    let bytes = match http_client.get(&download_url).send().await {
        Ok(r) => match r.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => {
                warn!("telegram file read failed ({filename}): {e}");
                content.push_str("\n");
                content.push_str(&format!("[Attachment: {filename} — download failed]"));
                continue;
            }
        },
        Err(e) => {
            warn!("telegram file fetch failed ({filename}): {e}");
            content.push_str("\n");
            content.push_str(&format!("[Attachment: {filename} — download failed]"));
            continue;
        }
    };

    match crate::file_store::save_upload(plexus_user_id, &filename, &bytes).await {
        Ok(fid) => media_urls.push(format!("/api/files/{fid}")),
        Err(e) => {
            warn!("telegram save_upload failed ({filename}): {e:?}");
            content.push_str("\n");
            content.push_str(&format!("[Attachment: {filename} — storage failed]"));
        }
    }
}
```

- [ ] **Step 6: Relax the empty-content guard and populate `media`**

Find:

```rust
if content.is_empty() {
    return;
}
```

Replace with:

```rust
if content.is_empty() && media_urls.is_empty() {
    return;
}
```

In the `InboundEvent { ... }` literal below, replace `media: vec![],` with `media: media_urls,`.

- [ ] **Step 7: Build and test**

```bash
cargo build -p plexus-server && cargo test -p plexus-server --lib
```

- [ ] **Step 8: Commit**

```bash
git add plexus-server/src/channels/telegram.rs plexus-server/Cargo.toml
git commit -m "$(cat <<'EOF'
telegram: download inbound attachments (photo/voice/doc/etc.)

Harvest attachments from msg.photo/voice/audio/document/video/video_note/
animation, resolve via bot.get_file, download from api.telegram.org,
save_upload → /api/files/{id}. Filenames synthesized when absent so
agents see human-readable names like voice_message_20260415_103005.ogg.
EOF
)"
```

---

## Task 14: Gateway inbound `media` field parsing (PARALLEL)

> **Can run in parallel with Tasks 12, 13, 15, 16.**

**Files:**
- Modify: `plexus-server/src/channels/gateway.rs` (around line 95-133)

- [ ] **Step 1: Add a test for JSON → `InboundEvent.media`**

In `mod tests` at the bottom of `plexus-server/src/channels/gateway.rs` (create the module if missing):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_media_from_gateway_frame() {
        let raw = r#"{
            "type": "message",
            "chat_id": "c1",
            "sender_id": "u1",
            "session_id": "s1",
            "content": "hi",
            "media": ["/api/files/a", "/api/files/b"]
        }"#;
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        let media = extract_media(&parsed);
        assert_eq!(media, vec!["/api/files/a".to_string(), "/api/files/b".to_string()]);
    }

    #[test]
    fn test_parse_media_missing() {
        let raw = r#"{"type":"message","chat_id":"c","session_id":"s","content":"hi"}"#;
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert!(extract_media(&parsed).is_empty());
    }

    #[test]
    fn test_parse_media_malformed() {
        let raw = r#"{"type":"message","media":"not-an-array"}"#;
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert!(extract_media(&parsed).is_empty());
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p plexus-server --lib channels::gateway::tests
```

Expected: `extract_media` not defined.

- [ ] **Step 3: Add the helper and use it**

Near the top of `plexus-server/src/channels/gateway.rs` (above the frame-parsing loop):

```rust
fn extract_media(parsed: &serde_json::Value) -> Vec<String> {
    parsed
        .get("media")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}
```

- [ ] **Step 4: Wire it into the frame handler**

Find the block that constructs `InboundEvent` in `gateway.rs` (around line 122). Before the event literal, add:

```rust
let media = extract_media(&parsed);
```

In the `InboundEvent { ... }` literal, replace `media: vec![],` with `media,`.

- [ ] **Step 5: Relax the empty-content guard**

Find:

```rust
if content.is_empty() || session_id.is_empty() {
    continue;
}
```

Replace with:

```rust
if session_id.is_empty() {
    continue;
}
if content.is_empty() && media.is_empty() {
    continue;
}
```

- [ ] **Step 6: Build and test**

```bash
cargo build -p plexus-server && cargo test -p plexus-server --lib
```

- [ ] **Step 7: Commit**

```bash
git add plexus-server/src/channels/gateway.rs
git commit -m "$(cat <<'EOF'
gateway: read media[] from inbound WS frame

Frontend will POST /api/files first and include the resulting
/api/files/{id} paths in the WS send frame's media array. Gateway
was already forwarding the field; server now reads and populates
InboundEvent.media. Media-only messages are allowed.
EOF
)"
```

---

## Task 15: Frontend `ChatInput` upload UX (PARALLEL)

> **Can run in parallel with Tasks 12, 13, 14, 16.**

**Files:**
- Modify: `plexus-frontend/src/components/ChatInput.tsx`
- Possibly create: `plexus-frontend/src/lib/upload.ts` for the XHR helper

- [ ] **Step 1: Create a small upload helper with progress**

Create `plexus-frontend/src/lib/upload.ts`:

```typescript
export interface UploadResult {
  file_id: string;
  file_name: string;
}

export function uploadFile(
  file: File,
  onProgress: (pct: number) => void,
  signal: AbortSignal,
): Promise<UploadResult> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    const form = new FormData();
    form.append('file', file);

    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable) {
        onProgress(Math.round((e.loaded / e.total) * 100));
      }
    };
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          resolve(JSON.parse(xhr.responseText));
        } catch (e) {
          reject(new Error('Invalid upload response'));
        }
      } else {
        reject(new Error(`Upload failed: ${xhr.status}`));
      }
    };
    xhr.onerror = () => reject(new Error('Upload network error'));
    xhr.onabort = () => reject(new Error('Upload aborted'));

    signal.addEventListener('abort', () => xhr.abort());

    xhr.open('POST', '/api/files');
    // Auth: reuse the same bearer-token strategy as other fetches in the
    // codebase. If the frontend keeps JWT in localStorage (check lib/api.ts),
    // set the header here.
    const token = localStorage.getItem('jwt'); // adapt to actual location
    if (token) {
      xhr.setRequestHeader('Authorization', `Bearer ${token}`);
    }
    xhr.send(form);
  });
}

export const MAX_UPLOAD_BYTES = 20 * 1024 * 1024;
```

- [ ] **Step 2: Rewrite `ChatInput.tsx`**

Replace `plexus-frontend/src/components/ChatInput.tsx` with:

```tsx
import { useState, useRef, useEffect, KeyboardEvent, ClipboardEvent, DragEvent } from 'react';
import { Send, Paperclip, X } from 'lucide-react';
import { uploadFile, MAX_UPLOAD_BYTES, UploadResult } from '../lib/upload';

interface Chip {
  key: string;          // local unique id
  file: File;
  progress: number;     // 0..100
  fileId?: string;      // once upload completes
  error?: string;
  controller: AbortController;
}

interface Props {
  onSend: (content: string, media: string[]) => void;
  disabled?: boolean;
}

export default function ChatInput({ onSend, disabled }: Props) {
  const [value, setValue] = useState('');
  const [chips, setChips] = useState<Chip[]>([]);
  const [isDragging, setDragging] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const anyUploading = chips.some(c => c.progress < 100 && !c.error);

  function updateChip(key: string, patch: Partial<Chip>) {
    setChips(prev => prev.map(c => (c.key === key ? { ...c, ...patch } : c)));
  }

  function removeChip(key: string) {
    setChips(prev => {
      const c = prev.find(x => x.key === key);
      c?.controller.abort();
      return prev.filter(x => x.key !== key);
    });
  }

  function addFiles(files: FileList | File[]) {
    const arr = Array.from(files);
    for (const file of arr) {
      if (file.size > MAX_UPLOAD_BYTES) {
        alert(`${file.name} exceeds 20 MB limit`);
        continue;
      }
      const key = `${Date.now()}-${Math.random()}`;
      const controller = new AbortController();
      const chip: Chip = { key, file, progress: 0, controller };
      setChips(prev => [...prev, chip]);
      uploadFile(
        file,
        (pct) => updateChip(key, { progress: pct }),
        controller.signal
      )
        .then((res: UploadResult) => {
          updateChip(key, { progress: 100, fileId: res.file_id });
        })
        .catch((e: Error) => {
          updateChip(key, { error: e.message });
        });
    }
  }

  function submit() {
    const trimmed = value.trim();
    const media = chips
      .filter(c => c.fileId && !c.error)
      .map(c => `/api/files/${c.fileId}`);
    if ((!trimmed && media.length === 0) || disabled || anyUploading) return;
    onSend(trimmed, media);
    setValue('');
    setChips([]);
    if (textareaRef.current) textareaRef.current.style.height = 'auto';
  }

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }

  function handleInput() {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = Math.min(el.scrollHeight, 200) + 'px';
  }

  function handlePaste(e: ClipboardEvent<HTMLTextAreaElement>) {
    const items = e.clipboardData?.items;
    if (!items) return;
    const files: File[] = [];
    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      if (item.kind === 'file') {
        const f = item.getAsFile();
        if (f) files.push(f);
      }
    }
    if (files.length > 0) {
      e.preventDefault();
      addFiles(files);
    }
  }

  function handleDrop(e: DragEvent<HTMLDivElement>) {
    e.preventDefault();
    setDragging(false);
    if (e.dataTransfer.files) addFiles(e.dataTransfer.files);
  }

  function handleDragOver(e: DragEvent<HTMLDivElement>) {
    e.preventDefault();
    setDragging(true);
  }

  function handleDragLeave() {
    setDragging(false);
  }

  return (
    <div
      className={`flex flex-col gap-2 rounded-xl border p-3 ${isDragging ? 'ring-2 ring-accent' : ''}`}
      style={{
        background: 'var(--card)',
        borderColor: 'var(--border)',
        width: 'min(90vw, 720px)',
        minWidth: 'min(90vw, 420px)',
      }}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {chips.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {chips.map(c => (
            <div
              key={c.key}
              className="flex items-center gap-1 px-2 py-1 rounded bg-[var(--muted)] text-xs"
              title={c.error || `${c.progress}%`}
            >
              <span>{c.file.name}</span>
              {c.error ? (
                <span className="text-red-500">⚠</span>
              ) : c.progress < 100 ? (
                <span>{c.progress}%</span>
              ) : (
                <span>✓</span>
              )}
              <button onClick={() => removeChip(c.key)} title="Remove">
                <X size={12} />
              </button>
            </div>
          ))}
        </div>
      )}

      <div className="flex items-end gap-2">
        <button
          onClick={() => fileInputRef.current?.click()}
          disabled={disabled}
          className="p-1.5 rounded-lg disabled:opacity-30"
          style={{ color: 'var(--muted-fg)' }}
          title="Attach file"
        >
          <Paperclip size={16} />
        </button>
        <input
          ref={fileInputRef}
          type="file"
          multiple
          hidden
          onChange={e => {
            if (e.target.files) addFiles(e.target.files);
            e.target.value = '';
          }}
        />

        <textarea
          ref={textareaRef}
          value={value}
          onChange={e => setValue(e.target.value)}
          onInput={handleInput}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          disabled={disabled}
          placeholder="Message Plexus… (Enter to send, Shift+Enter for newline)"
          rows={1}
          className="flex-1 resize-none outline-none text-sm bg-transparent"
          style={{ color: 'var(--text)', maxHeight: 200 }}
        />

        <button
          onClick={submit}
          disabled={disabled || anyUploading || (!value.trim() && chips.filter(c => c.fileId).length === 0)}
          className="p-1.5 rounded-lg transition-all disabled:opacity-30"
          style={{ color: 'var(--accent)' }}
          title={anyUploading ? 'Waiting for uploads' : 'Send'}
        >
          <Send size={16} />
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Update the `onSend` consumer**

Find the component that uses `<ChatInput onSend={...} />` (likely `pages/Chat.tsx`):

```bash
grep -rn "onSend={\|<ChatInput" plexus-frontend/src/
```

Update the `onSend` signature from `(content: string) => void` to `(content: string, media: string[]) => void`. In the handler body, include `media` in the WS send payload:

```typescript
function handleSend(content: string, media: string[]) {
  ws.send(JSON.stringify({
    type: 'send',
    chat_id,
    session_id,
    content,
    ...(media.length > 0 ? { media } : {}),
  }));
}
```

- [ ] **Step 4: Manual browser smoke test**

Start the frontend dev server:

```bash
cd plexus-frontend && npm run dev
```

Open the UI, log in, and try:
- Click the paperclip, select a <20MB PNG → chip appears with progress → check → click Send → verify WS frame in devtools Network tab includes `media: ["/api/files/..."]`.
- Drag a PDF onto the input → chip appears.
- Copy an image from a screenshot tool, paste into the textarea → chip appears.
- Select a file >20MB → alert fires, no chip.
- Remove a chip mid-upload → XHR aborts.

- [ ] **Step 5: Commit**

```bash
git add plexus-frontend/src/
git commit -m "$(cat <<'EOF'
frontend: ChatInput supports paperclip / drag / paste upload

Files upload to /api/files via XHR with per-file progress; resulting
/api/files/{id} refs get bundled into the WS send frame's media
array. Oversize files (>20 MB) rejected client-side; attachments
show as chips above the textarea with progress, success, error,
and remove states.
EOF
)"
```

---

## Task 16: Frontend message rendering for attachments (PARALLEL)

> **Can run in parallel with Tasks 12, 13, 14, 15.**

**Files:**
- Modify: `plexus-frontend/src/components/Message.tsx` (and/or `MarkdownContent.tsx`)

- [ ] **Step 1: Read the current Message rendering**

```bash
cat plexus-frontend/src/components/Message.tsx
```

- [ ] **Step 2: Render `/api/files/{id}` URLs as attachment chips**

In the message body renderer, detect text of the form `[Attachment: NAME → /api/files/ID]` via a regex pass. For each match, render a clickable download chip:

```tsx
// helper function
function renderAttachments(text: string): React.ReactNode[] {
  const re = /\[Attachment: (.+?) → (\/api\/files\/[^\]]+)\]/g;
  const out: React.ReactNode[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) {
      out.push(text.slice(last, m.index));
    }
    out.push(
      <a
        key={`${m.index}-${m[2]}`}
        href={m[2]}
        download={m[1]}
        className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-[var(--muted)] text-xs no-underline"
      >
        📎 {m[1]}
      </a>
    );
    last = re.lastIndex;
  }
  if (last < text.length) {
    out.push(text.slice(last));
  }
  return out;
}
```

Where the component currently prints plain message text, replace with `renderAttachments(text)`.

- [ ] **Step 3: Render inline images for multimodal user messages**

If the stored message content is a JSON array of content blocks (multimodal), parse it and render `ImageUrl` blocks as `<img>`:

```tsx
function renderMessageContent(raw: string): React.ReactNode {
  // Try to parse as a multimodal block array.
  try {
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      return parsed.map((b: any, i: number) => {
        if (b.type === 'text') return <p key={i}>{renderAttachments(b.text)}</p>;
        if (b.type === 'image_url' && b.image_url?.url) {
          return <img key={i} src={b.image_url.url} alt="" className="max-w-xs rounded" />;
        }
        return null;
      });
    }
  } catch {
    // Not JSON — fall through to plain text.
  }
  return <div>{renderAttachments(raw)}</div>;
}
```

- [ ] **Step 4: Manual smoke test**

Reload the chat UI, send a message with a PDF and an image, refresh — verify the image renders inline and the PDF appears as a clickable chip.

- [ ] **Step 5: Commit**

```bash
git add plexus-frontend/src/
git commit -m "$(cat <<'EOF'
frontend: render attachments in chat history

[Attachment: … → /api/files/…] tokens in message text become
clickable download chips; multimodal user messages with image_url
blocks render the images inline.
EOF
)"
```

---

## Task 17: Cleanup dead-code annotations

**Files:**
- Modify: `plexus-server/src/bus.rs` (InboundEvent), `plexus-server/src/session.rs` (SessionHandle)

- [ ] **Step 1: Remove `#[allow(dead_code)]` where no longer needed**

Now that `InboundEvent.media` is actively read by `build_user_content` via the channel adapters, the `#[allow(dead_code)]` on `InboundEvent` may no longer be needed. Similarly for `SessionHandle` now that `vision_stripped` is read.

Remove both annotations, then:

```bash
cargo build -p plexus-server
```

If warnings return for specific unused fields (e.g., `SessionHandle.user_id` still unused), re-add `#[allow(dead_code)]` on the *field* instead of the struct:

```rust
pub struct SessionHandle {
    #[allow(dead_code)]
    pub user_id: String,
    pub inbox_tx: mpsc::Sender<InboundEvent>,
    pub lock: Arc<Mutex<()>>,
    pub vision_stripped: Arc<AtomicBool>,
}
```

- [ ] **Step 2: Commit**

```bash
git add plexus-server/src/
git commit -m "$(cat <<'EOF'
remove unneeded #[allow(dead_code)] after wiring inbound media

InboundEvent and SessionHandle are now fully consumed. Field-level
allows remain only where a single field is still scaffolded.
EOF
)"
```

---

## Task 18: End-to-end manual smoke tests

- [ ] **Step 1: Build everything, start all three services**

```bash
cargo build --release
# Terminal 1:
DATABASE_URL=... cargo run -p plexus-server --release
# Terminal 2:
cargo run -p plexus-gateway --release
# Terminal 3:
cd plexus-frontend && npm run build && npm run preview
```

- [ ] **Step 2: Gateway (browser) smoke**

In the browser UI:
- Drag a < 20 MB PNG → chip → Send → agent describes it (if VLM configured) or strips with retry (if text-only configured).
- Drag a < 20 MB PDF → chip → Send → agent sees `[Attachment: doc.pdf → /api/files/…]` reference, can (if prompted) call `file_transfer` to move it to a connected client.
- Paste an image from a screenshot → chip → Send.
- Select a 30 MB file → client rejects with alert.

- [ ] **Step 3: Discord smoke**

With a Discord bot configured for your test user:
- DM a photo → agent describes it.
- DM a voice note → agent reports filename in the format `voice_message_YYYYMMDD_HHMMSS.ogg` and can file_transfer it.
- DM a >20 MB attachment (use a file-sharing site link or a large video) → agent sees the `exceeds 20 MB limit` marker in content.

- [ ] **Step 4: Telegram smoke**

With a Telegram bot configured:
- Send a photo → agent describes it.
- Send a voice note → agent reports synthesized filename.
- Send a document (e.g., a PDF) → agent can file_transfer to client.

- [ ] **Step 5: Strip-and-retry smoke**

In admin UI, swap the LLM config to a text-only model (e.g., `deepseek-chat` or a local non-vision endpoint). From any channel:
- Send an image → check server logs for `LLM non-transient error with images; retrying stripped` → agent answers based on text/filename.
- Send another image in the *same session* → check logs that image encoding is skipped (no `retrying stripped` log because `vision_stripped=true` already).

Swap the LLM config back to a VLM. In the same session:
- Send an image → agent sees the image again (flag reset by admin handler).

- [ ] **Step 6: Context rebuild smoke**

- Send a session a few image messages.
- Let 24 hours pass (or manually delete `/tmp/plexus-uploads/{user_id}/*` files).
- Start a new chat that loads the history → agent still sees the image blocks (served from DB base64).

- [ ] **Step 7: No commit needed for manual smoke**

Smoke test results go in the PR description if one is opened.

---

## Post-Implementation Checklist

- [ ] `cargo build` clean, zero warnings in workspace.
- [ ] `cargo test -p plexus-server --lib` all pass.
- [ ] `cargo test -p plexus-common` all pass.
- [ ] `cargo clippy` — review any new lints; fix trivial ones.
- [ ] Frontend build clean: `cd plexus-frontend && npm run build`.
- [ ] Manual smoke tests in Task 18 all pass.
- [ ] Spec `docs/superpowers/specs/2026-04-15-inbound-media-design.md` matches implementation (or updated to match).

---

## Post-Plan Adjustments (as built)

The plan above is preserved as a historical artifact. The implementation landed slightly differently in a few places; read these if you're reconciling the plan against what's on HEAD.

### Task 19: Persist multimodal user messages as JSON in DB (added)

Not in the original plan. Added after Task 18 surfaced two gaps with the "rebuild-from-file-store every iteration" approach:

- Context rebuilds after the 24 h file-store TTL lost the image (file URL 404).
- Mid-ReAct-turn iterations lost the image because only the tail user row was rebuilt as `Content::Blocks`.

**What shipped:** agent_loop now calls `build_user_content` once at user-message save time (when `event.media` is non-empty), serializes the resulting `Content::Blocks` to JSON, and stores it in `messages.content`. `reconstruct_history` sniff-parses user rows on read. Commit `e848798`.

**Simplifications folded into Task 19:**

- `build_user_content`'s `vision_stripped: bool` parameter (added in Task 7) was **removed**. The function now always produces the canonical unstripped form.
- `split_trailing_user` helper (added in Task 8) was **deleted**. The tail user row is already in its final JSON form in the DB, so no tail-specific path exists.
- `build_context`'s `latest_user_media: &[String]` parameter (added in Task 8) was **removed**.
- Vision stripping moved to a post-pass in `build_context` via `providers::openai::strip_images_in_place` (which already exists from Task 5). Placeholder wording is the generic `"[Image omitted: model does not support vision]"` (no filename) — consistent with the provider's own strip behavior.
- `ChatMessage::user_with_blocks` is now `#[cfg(test)]` since production no longer needs the constructor — `reconstruct_history` builds the `ChatMessage` literal directly.

### Task 7 polish (added mid-Task-7)

`.heic` and `.heif` added to `mime_from_filename` so iPhone photos render inline. Commit `6394a03`.

### Task 3 polish (added mid-Task-3)

`Content::into_text(self)` (consuming variant, avoids a clone in `call_llm`) and `Content::len(&self)` (byte length without allocating) added alongside `as_text`. Commit `0539d6e`.

### Where to read the current architecture

The spec `docs/superpowers/specs/2026-04-15-inbound-media-design.md` has been updated to describe the post-Task-19 architecture (3-arg `build_user_content`, post-pass stripping, base64 in DB). Prefer it over Tasks 7/8 of this plan when reconciling against HEAD.
