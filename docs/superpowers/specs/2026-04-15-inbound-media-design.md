# Inbound Media Design Spec

## Overview

Inbound-media support for PLEXUS: users can send any file ≤20MB via **Discord**, **Telegram**, or the **Gateway** (browser), and the agent receives it. Images arrive as vision content blocks in the LLM user message; non-image files appear as `[Attachment: name → /api/files/{id}]` references, which the agent processes by moving them to a client device via `file_transfer` and operating there.

Behavior is identical across all three channels.

**Scope:** Three channel adapters + provider/context-builder refactor + file-store reuse + frontend upload UI + system-prompt teaching.

**Not in scope:**

- Server-side transcription, OCR, PDF extraction (client-side via `file_transfer` + shell tools).
- Mimetype allowlist — any file ≤20MB is accepted.
- Per-user disk quotas beyond the existing 24 h cleanup.
- Preemptive vision-capability flag on the LLM config — discover at runtime.
- Changes to `file_transfer` client↔client semantics — that path stays uncapped.

---

## 1. Goals & Non-Goals

**Goals**

1. Users can attach files on any channel and the agent acts on them.
2. Images are delivered inline as OpenAI-compatible `image_url` content blocks.
3. Non-image files are referenced by URL into the server file store; the agent pulls them to a client for processing.
4. A strip-and-retry fallback handles LLM configs that do not support vision.
5. Message history survives file-store TTL and source-platform deletion via base64-in-DB persistence.
6. One mental model, one size cap, one filename convention across all channels.

**Non-goals** — see Overview above.

---

## 2. Data Flow

```
  ┌─────────────┐   attachment URL   ┌──────────────────┐
  │  Discord    │──── GET bytes ────►│                  │
  └─────────────┘                    │                  │
  ┌─────────────┐  getFile → URL     │  file_store      │
  │  Telegram   │──── GET bytes ────►│  ::save_upload   │
  └─────────────┘                    │  (20 MB cap,     │
  ┌─────────────┐  POST /api/files   │   per-user dir)  │
  │  Gateway    │──── already       ─►│                  │
  │  (browser)  │  returned file_id  └──────────────────┘
  └─────────────┘                             │
                                              │ /api/files/{id}
                                              ▼
                         InboundEvent { content, media: [urls], … }
                                              │
                                              ▼
                                bus::publish_inbound → session
                                              │
                                              ▼
                  context::build_user_content(state, user, content,
                                              media, vision_stripped)
                     - image mimetype → read bytes → base64 →
                         ContentBlock::ImageUrl { data-URL }
                     - non-image → append "[Attachment: name →
                         /api/files/{id}]\nUse file_transfer to move
                         it to a client device for further processing."
                     - when vision_stripped=true → image blocks become
                         "[Image: name — not displayed, model does not
                          support vision]" text
                                              │
                                              ▼
                              providers::openai::chat_completion
                                              │
                   ┌──────────────────────────┴──────────────────────────┐
                   │                                                     │
             success                                            non-transient error
                                                                         │
                                                           if any user block has ImageUrl:
                                                             strip images → retry once
                                                             on retry success:
                                                               SessionHandle.vision_stripped = true
                                                                         │
                                                                         ▼
                                            db::messages::save (fully-built user content
                                            with base64 inline, so future context rebuilds
                                            are durable against 24 h file-store cleanup)
```

**Invariants**

- Channel adapters never call the LLM. They only download bytes, call `file_store::save_upload`, and populate `InboundEvent.media` with the returned URLs.
- Base64 encoding happens once, at **context-build time**, reading from the file store.
- The 24 h file-store TTL is a hot cache; the DB row is the durable record.
- The agent loop owns the strip-retry decision via `SessionHandle.vision_stripped`.

---

## 3. Type & Protocol Changes

### 3.1 `providers::openai::ChatMessage`

Current (`plexus-server/src/providers/openai.rs:15`):

```rust
pub struct ChatMessage {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
}
```

New:

```rust
pub struct ChatMessage {
    pub role: String,
    pub content: Option<Content>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),              // wire: "hello"
    Blocks(Vec<ContentBlock>), // wire: [{"type": "text", "text": "..."}, ...]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String, // "data:image/png;base64,..." or "https://..."
}
```

- `#[serde(untagged)]` makes `Content::Text` serialize as a plain string, `Content::Blocks` as an array — matches the OpenAI API exactly.
- Existing `ChatMessage::system/user/assistant_text/tool_result` constructors keep working; they now wrap in `Content::Text(...)`.
- New constructor: `ChatMessage::user_with_blocks(Vec<ContentBlock>)`.
- User messages built by `context::build_user_content` **always** use `Content::Blocks` (even text-only), so the wire form for role=user is uniform.
- System, assistant, and tool messages continue to use `Content::Text` — those roles' APIs always return/expect plain strings.

### 3.2 `bus::InboundEvent.media`

Already declared as `Vec<String>`. No type change — channel adapters begin populating it instead of passing `vec![]`.

### 3.3 `SessionHandle` additions

```rust
pub struct SessionHandle {
    pub user_id: String,
    pub inbox_tx: mpsc::Sender<InboundEvent>,
    pub lock: Arc<Mutex<()>>,
    pub vision_stripped: Arc<AtomicBool>,  // NEW
}
```

- Defaults `false` at session creation.
- Flipped to `true` by the provider after a successful strip-and-retry.
- Eagerly reset to `false` by the admin LLM-config-update handler (iterates all live sessions).

### 3.4 Constant change

```rust
// plexus-common/src/consts.rs
pub const FILE_UPLOAD_MAX_BYTES: usize = 20 * 1024 * 1024;  // was 25 MB
```

This one change propagates to every in-store path automatically:

- `plexus-gateway` `RequestBodyLimitLayer` (`main.rs:66`)
- `plexus-gateway` proxy body limit (`proxy.rs:64`)
- `plexus-server::file_store::save_upload` (`file_store.rs:16`)
- `file_transfer` tool when `to_device == "server"`

`file_transfer` client↔client relay is untouched (no explicit cap; tungstenite's default frame limit applies).

### 3.5 `plexus-common` protocol

**No changes.** The gateway already forwards the `media` field through the WS frame (`plexus-gateway/src/routing.rs:98,107`). The server-side parser in `plexus-server::channels::gateway` will start reading it.

---

## 4. Component Changes

### 4.1 `plexus-common/src/consts.rs`

- Change `FILE_UPLOAD_MAX_BYTES` to 20 MB.

### 4.2 `plexus-server/src/file_store.rs`

- No API changes — existing `save_upload(user_id, filename, bytes) -> Result<file_id, ApiError>` is exactly what channel adapters need.
- Reuses the 20 MB cap.
- Files live at `/tmp/plexus-uploads/{user_id}/{uuid}_{sanitized_filename}`.
- Per-user directory, 24 h cleanup unchanged.

### 4.3 `plexus-server/src/providers/openai.rs`

- Refactor `ChatMessage.content` to `Option<Content>` (Section 3.1).
- Add `Content` / `ContentBlock` / `ImageUrl` types.
- Add `ChatMessage::user_with_blocks(Vec<ContentBlock>)` constructor.
- Modify `chat_completion` (and whatever inner retry loop exists) to:
  1. Detect non-transient error (any status that is not 429 and not 5xx; also include network errors that were already classified non-transient).
  2. If the request payload contained any user message with a `ContentBlock::ImageUrl`:
     - Build a stripped version: replace each `ContentBlock::ImageUrl` in every user message with `ContentBlock::Text { text: "[Image omitted: model does not support vision]" }`.
     - Retry once with the stripped payload.
     - On success, set `response.vision_stripped = true`.
- `LlmResponse` gains a boolean field: `vision_stripped: bool`.

### 4.4 `plexus-server/src/context.rs`

Add:

```rust
pub async fn build_user_content(
    state: &AppState,
    user_id: &str,
    content: &str,
    media: &[String],
    vision_stripped: bool,
) -> Vec<ContentBlock>
```

Behavior:

1. If `content` is non-empty, push `ContentBlock::Text { text: content.into() }`.
2. For each entry in `media`:
   - Parse `/api/files/{id}` from the URL.
   - `file_store::load_file(user_id, file_id).await` → `(bytes, filename)`.
   - Detect mime from filename extension (and optionally content sniff).
   - If mime is an image:
     - If `vision_stripped`, push `ContentBlock::Text { text: "[Image: {filename} — not displayed, model does not support vision]" }`.
     - Else, base64-encode bytes, push `ContentBlock::ImageUrl { image_url: ImageUrl { url: format!("data:{mime};base64,{b64}") } }`.
   - Else (non-image):
     - Defer to a trailing text block (see step 3).
   - On `load_file` error: push `ContentBlock::Text { text: "[Attachment: {filename} — storage read failed]" }`.
3. Collect all non-image attachment references and append a single trailing `ContentBlock::Text`:

   ```
   [Attachment: name1 → /api/files/{id1}]
   Use file_transfer to move it to a client device for further processing.
   [Attachment: name2 → /api/files/{id2}]
   Use file_transfer to move it to a client device for further processing.
   ```

4. Return the accumulated blocks.

`build_context`'s existing signature is unchanged. Internally, when building the latest user message, it calls `build_user_content(...)` and constructs `ChatMessage::user_with_blocks(blocks)` instead of `ChatMessage::user(text)`.

### 4.5 `plexus-server/src/agent_loop.rs`

- Pass `session.vision_stripped.load(Ordering::Relaxed)` into `build_user_content` / `build_context`.
- After each LLM call, if `response.vision_stripped == true`, `session.vision_stripped.store(true, Ordering::Relaxed)`.

### 4.6 `plexus-server/src/channels/discord.rs`

Inside the `EventHandler::message` receiver, **after** existing allow-list/partner checks:

```rust
let mut media_urls: Vec<String> = Vec::new();
for att in &msg.attachments {
    if (att.size as usize) > FILE_UPLOAD_MAX_BYTES {
        media_urls.push(format!(
            "[Attachment: {} ({:.1} MB) — exceeds {} MB limit, not downloaded]",
            att.filename,
            att.size as f64 / 1024.0 / 1024.0,
            FILE_UPLOAD_MAX_BYTES / 1024 / 1024
        ));  // inline marker; see step 2 below on handling
        continue;
    }
    match reqwest::get(&att.url).await {
        Ok(resp) => match resp.bytes().await {
            Ok(bytes) => match file_store::save_upload(&user_id, &att.filename, &bytes).await {
                Ok(file_id) => media_urls.push(format!("/api/files/{file_id}")),
                Err(e) => {
                    warn!("discord attachment save failed: {:?}", e);
                    // append error marker to content (see below)
                }
            },
            Err(e) => warn!("discord attachment read failed: {:?}", e),
        },
        Err(e) => warn!("discord attachment fetch failed: {:?}", e),
    }
}
```

(Pseudocode — actual implementation split between "saved URLs" and "error markers" so the two are fed to the right place.)

Error markers like `[Attachment: ... — download failed]` are **appended to the content string** (not inserted into `media`), so they show up as text in the agent's view.

**Also:** relax the `content.is_empty() { return; }` early return — allow empty-text messages when `media_urls` is non-empty.

### 4.7 `plexus-server/src/channels/telegram.rs`

Similar structure. Telegram's message types spread attachments across multiple accessor methods:

| Telegram message field | Filename handling |
|---|---|
| `msg.photo()` (largest variant) | Synthesize `photo_{YYYYMMDD_HHMMSS}.jpg` |
| `msg.voice()` | Synthesize `voice_message_{YYYYMMDD_HHMMSS}.ogg` |
| `msg.audio()` | Use `title` + `mime_type` if present, else synthesize `audio_{timestamp}.{ext}` |
| `msg.document()` | Use `file_name` if present, else `document_{timestamp}{.ext}` |
| `msg.video()` | Use `file_name` if present, else `video_{timestamp}.mp4` |
| `msg.video_note()` | Synthesize `video_note_{timestamp}.mp4` |
| `msg.animation()` | Use `file_name` if present, else `animation_{timestamp}.mp4` |

Download via `bot.get_file(file_id).await` → HTTPS URL with bot token embedded → `reqwest::get` → bytes → `file_store::save_upload`.

Relax `content.is_empty()` check to allow media-only messages.

### 4.8 `plexus-server/src/channels/gateway.rs`

In the incoming WS frame parser (current `channels/gateway.rs:95-116`), add:

```rust
let media: Vec<String> = parsed
    .get("media")
    .and_then(|m| m.as_array())
    .map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    })
    .unwrap_or_default();
```

Populate `InboundEvent.media` with this vector. Relax `content.is_empty()` check (already has `session_id.is_empty()` as a separate hard check).

### 4.9 `plexus-server/src/auth/admin.rs`

In `put_llm_config` (admin LLM-config-update endpoint), **after** a successful write:

```rust
for entry in state.sessions.iter() {
    entry.value().vision_stripped.store(false, Ordering::Relaxed);
}
```

One line of forward-looking work. Fan-out at 500 sessions is microseconds.

### 4.10 `plexus-server` system prompt

In `context.rs` (or wherever the system prompt is assembled), after the existing `## Identity` block, add:

```
## Attachments
Files may appear as [Attachment: name → /api/files/{id}]. They live on the
server. To operate on one, use `file_transfer` to move it to a client device,
then use client tools (shell, read_file, etc.). Choose the action based on
filename and the user's intent.
```

### 4.11 `plexus-frontend/src/components/ChatInput.tsx`

Augment the existing 70-line component with standard chat-app upload UX:

- **Paperclip button** beside the send button, opens a hidden `<input type="file" multiple>`.
- **Drag-and-drop overlay** on the `<textarea>` container; visual indicator on `dragenter`, drop handler extracts `File` objects.
- **Clipboard paste** handler (`onPaste`): extract `File` objects from `ClipboardEvent.clipboardData.items` whose type starts with `"image/"` or similar.
- **Client-side 20 MB check** before upload — show inline error if exceeded.
- **Upload**: `POST /api/files` as multipart form, using `XMLHttpRequest` (not `fetch`) so we get `upload.onprogress` events.
- **Attachment chips**: list above the textarea, each chip shows `filename` + progress spinner → checkmark + ✕ remove button. Removing cancels in-flight XHR or discards the `file_id`.
- **Send gating**: Send button disabled while any chip is in progress state.
- **On submit**: include `media: ["/api/files/{id}", ...]` alongside `content` in the WS frame.

### 4.12 `plexus-frontend/src/components/Message.tsx` (or equivalent)

- Render incoming user messages with images inline (already the case for plain text; add support for `<img src="/api/files/{id}">` for image attachments).
- Render non-image attachments as download chips / clickable filenames linking to `/api/files/{id}`.

---

## 5. Error Handling & Edge Cases

| Scenario | Handling |
|---|---|
| Discord CDN returns non-200 | Log warn, append `[Attachment: filename — download failed]` to `content`, skip that attachment, continue delivering the rest of the message |
| Telegram `getFile` fails or bot lacks permissions | Same — log warn, append marker, continue |
| File exceeds 20 MB | Skip download entirely, append `[Attachment: filename (N MB) — exceeds 20 MB limit, not downloaded]` to `content` |
| User sends media-only (no text) | Permitted — relax `content.is_empty()` early returns in all 3 channel adapters (Discord, Telegram, Gateway) |
| `file_store::save_upload` disk write fails | Log error, append `[Attachment: filename — storage failed]` to `content`, continue |
| LLM call fails non-transiently with images | Provider strips images, retries once; on success sets `vision_stripped=true` |
| All retries fail | Return the error; user sees standard LLM-error path |
| Context rebuild finds `/api/files/{id}` is 404 (past 24 h) | The user message in DB has base64 blocks inline — use those; file-store URL is informational only after expiry |
| User deletes the source-platform message after we downloaded | Irrelevant — we have our copy in file store + DB |
| Admin swaps LLM config mid-session | Handler resets every session's `vision_stripped` flag; next turn re-encodes images |

---

## 6. Lifecycle

```
  t=0                 channel adapter downloads bytes
                      file_store::save_upload → file on disk, file_id issued
                      InboundEvent { media: ["/api/files/{id}"], ... } published

  t=0 + agent turn    context::build_user_content reads bytes, encodes base64
                      LLM call executes
                      db::messages::save stores user content (base64 inline)

  t < 24 h            file on disk still accessible at /api/files/{id}
                      Used by: frontend re-display, file_transfer tool, follow-up encodings

  t ≥ 24 h            hourly cleanup deletes the on-disk file
                      DB row still has base64 — durable for context rebuilds

  session end         SessionHandle dropped; vision_stripped flag lost (ephemeral)
                      New session starts fresh with vision_stripped=false
```

---

## 7. Testing

**Foundation phases — sequential TDD, each phase a commit:**

1. **`FILE_UPLOAD_MAX_BYTES` → 20 MB.** Tests: assert new constant value; assert `save_upload` rejects `20 MB + 1`; confirm downstream consumers compile and respect it.
2. **`ChatMessage` / `Content` / `ContentBlock` refactor.** Tests: serde roundtrip `Content::Text` → `"hello"`; roundtrip `Content::Blocks([Text, ImageUrl])` → array shape; existing constructors (`system/user/assistant_text/tool_result`) still produce correct wire output; new `user_with_blocks` builds expected blocks.
3. **`context::build_user_content`** — unit tests with a mock `file_store::load_file` returning fixed `(bytes, filename)` tuples. Assert:
   - Text-only → one `Text` block.
   - Text + image → `Text` then `ImageUrl`, correct data-URL prefix.
   - Text + image + non-image → `Text`, `ImageUrl`, trailing `Text` with `[Attachment:...]` line.
   - `vision_stripped=true` → images become `[Image: name — not displayed, model does not support vision]` text.
   - Order is always text → images → attachment-refs.
4. **Provider strip-and-retry.** Mocked HTTP client. Tests:
   - Transient 429 → retries without stripping (backoff unchanged).
   - Non-transient 400 with no images → no strip, returns error.
   - Non-transient 400 with images in any user message → strip, retry, on success returns `vision_stripped=true`.
   - Strip-retry also fails → returns final error, `vision_stripped=false`.

**Parallel subagent phases — dispatched after the foundation lands:**

5. **Discord inbound attachments.** Use serenity's `EventHandler` test scaffolding. Assert: message with 1 image + 1 doc → `InboundEvent.media` has 2 URLs; oversize attachment marked inline; download failure marked inline; media-only (no caption) message delivered.
6. **Telegram inbound attachments.** Similar with teloxide's test helpers. Cover photo / voice / document / animation; assert fallback filename synthesis.
7. **Gateway inbound media field.** Unit-test `channels/gateway.rs` frame parser: frame with `media: ["..."]` → `InboundEvent.media` populated; missing / malformed `media` → empty vec.
8. **Frontend `ChatInput` upload flow.** Vitest + React Testing Library. Tests:
   - File picker → file uploaded → chip shown with filename and completed state → WS frame on send includes `media`.
   - Drag-and-drop a file → chip appears.
   - Clipboard paste image → chip appears.
   - Oversize file selected → inline error, no upload attempted, chip not added.
   - Remove chip (✕) → state cleared; if in flight, XHR aborted.

**End-to-end smoke tests — manual, post-implementation:**

- Real Discord bot: DM a photo → agent describes it. DM a voice note → agent reports filename and can `file_transfer` it to a connected client.
- Real Telegram bot: same for photo + voice.
- Real browser: drag a PDF in, send, agent acknowledges and moves it to a connected client.
- Swap admin LLM config to a text-only model: send an image → confirm strip-retry path fires (check logs, confirm reply is text-only); confirm `SessionHandle.vision_stripped` is now `true`; subsequent image in same session → stripped preemptively; admin updates LLM config back to a VLM → subsequent image in same session → re-encoded.

---

## 8. Out-of-scope follow-ups

These are adjacent ideas deliberately *not* in this milestone but worth noting for future work:

- **Per-user disk quota** beyond the hourly cleanup TTL.
- **Mimetype allowlist** if abuse patterns emerge ("Plexus as 24h cloud storage").
- **Preemptive vision flag** on LLM config (e.g., `llm_config.supports_vision: bool`) to skip the first wasted call per session on known text-only models.
- **Server-side media operations** (ffmpeg thumbnails, PDF text extraction) — currently clients handle this via `file_transfer` + shell.
- **Retained file IDs on DB rows** so the file-store cleanup skips files referenced by persisted history (extending the hot-cache window).
- **Client→client `file_transfer` size cap** — currently uncapped, may want a protective limit later.

---
