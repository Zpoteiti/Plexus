# M2 Cross-Channel Addressing — Spec + Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use `- [ ]` syntax for tracking.

**Goal:** Let the agent reliably message the owner on **any** configured channel (Discord DM, Telegram DM, Gateway), regardless of which channel the current session runs on. Cron-triggered gateway messages stop dropping when browser UUIDs go stale.

**Architecture:** Gateway outbound switches from `chat_id`-based routing to `user_id`+`session_id`-based routing via a new `session_update` WS frame. Discord adds a per-user DM channel cache resolved on demand. System prompt exposes a compact `## Channels` section that lists only the owner's addressable targets. Non-partner senders are confined to their inbound channel.

**Tech stack:** axum + tokio-tungstenite (gateway protocol), serenity 0.12 (Discord DM creation), sqlx (per-user channel configs), DashMap (browser cache).

**Parent branch:** `M3-gateway-frontend`, based on `e848798` (inbound-media complete).

---

## 1. Overview

PLEXUS today has three channels — Discord, Telegram, Gateway — but the agent can only reply on the channel where the user just spoke. Three real problems surface:

1. **Cron on gateway is broken after reconnect.** Cron stores the session's browser-connection `chat_id` (a per-connect UUID). When the user reloads, that UUID is gone; gateway's router logs "no browser" and drops the message.
2. **"Remind me via Telegram" from Discord is ignored.** The agent has no way to learn the owner's Telegram address; `ChannelIdentity.partner_id` exists but is never surfaced to the prompt.
3. **Security surface.** Non-partner senders could, in principle, ask the agent to relay to the owner's other channels. Nothing currently prevents this at the `message` tool.

This spec closes all three in a single pass.

## 2. Goals & Non-Goals

**Goals**

- Agent can DM the owner on any configured channel via the existing `message` tool.
- Gateway outbound routes by `user_id` + `session_id`; chat_id goes away for outbound. Cron messages survive browser reconnects.
- Browser receives a compact notification frame; fetches content via the existing REST session-history endpoint.
- Cron / proactive deliveries **never interrupt the current session** — they land in their own `session_id` (already the case for cron) and surface in the browser as a new/updated session.
- Non-partner senders cannot trigger outbound on a different channel from their inbound one.
- System prompt lists configured channels and their addressable `chat_id` shapes.

**Non-goals**

- Enumerating Discord guild channels. DM-only. If the owner needs "send to #daily-reports", they can hand-paste the `guild_id/channel_id`.
- Telegram group-chat targeting beyond what inbound caching already gives.
- Broadcast notifications across devices (browser push, etc.) — only connected WS clients get real-time updates; offline browsers see the new session on next reconnect.
- Delivery guarantees / retries for unconfigured target channels. Log-and-drop.
- Browser-side UX for the notification badge / new-session indicator is out of scope for this spec — handled in a follow-up frontend task.

## 3. Design

### 3.1 Protocol change — new `session_update` frame (server → gateway → browsers)

Today the gateway handles exactly one outbound frame: `{"type": "send", "chat_id": "...", ...}` routed by `chat_id`. Add:

```json
{"type": "session_update", "user_id": "<user_id>", "session_id": "<session_id>"}
```

Semantics: a session belonging to `user_id` has new content in the DB. Gateway fans out the frame to every connected browser whose `BrowserConnection.user_id == user_id`. Each browser decides:

- If currently viewing `session_id` → fetch latest messages via existing REST `GET /api/sessions/{id}/messages`.
- Else → show a badge / notification on the session list (frontend-side, out of scope for this spec).

Gateway ignores `session_update` frames if no browsers for that user are connected — the DB already holds the content; the user sees it on next reconnect.

### 3.2 Gateway outbound routing by `user_id`

`plexus-gateway/src/state.rs::BrowserConnection.user_id` is already populated; currently `#[allow(dead_code)]`. After this change:

- New function `route_session_update(state, user_id, session_id)` in `plexus-gateway/src/routing.rs`.
- Iterates `state.browsers` entries; for each with matching `user_id`, dispatches the frame via the existing `OutboundFrame` channel.
- On backpressure (channel full), log a warning and drop for that browser only (don't block others).

### 3.3 Server-side `channels::gateway::deliver`

Current `deliver` emits `{"type": "send", "chat_id": event.chat_id, ...}`. Change to emit:

```rust
serde_json::json!({
    "type": "session_update",
    "user_id": event.user_id,
    "session_id": event.session_id,
})
```

`chat_id` field drops out of outbound — it was only ever a per-connect UUID. `media` and progress metadata also drop: the browser fetches full content via REST after receiving the notification, so the frame stays tiny and consistent regardless of message kind.

The existing `OutboundEvent.media` and `is_progress` fields are **still consumed** by Discord and Telegram `deliver`s — only the gateway's consumption changes.

### 3.4 Discord cross-channel DM

Add a `dm_channel_cache: DashMap<UserId, ChannelId>` on `DiscordHandler`. New helper:

```rust
async fn resolve_chat_id(&self, ctx: &Context, chat_id: &str) -> Option<ChannelId> {
    if let Some(user_id) = chat_id.strip_prefix("dm/") {
        let uid = UserId::new(user_id.parse().ok()?);
        if let Some(cid) = self.dm_channel_cache.get(&uid).map(|r| *r) {
            return Some(cid);
        }
        let dm = uid.create_dm_channel(ctx).await.ok()?;
        let cid = dm.id;
        self.dm_channel_cache.insert(uid, cid);
        return Some(cid);
    }
    // existing guild/channel_id parse logic
    ...
}
```

`deliver` calls `resolve_chat_id(...)` before sending. `dm/<user_id>` paths resolve to DM channels; `<guild_id>/<channel_id>` paths resolve as today. Unknown formats log and drop.

### 3.5 System prompt `## Channels` section

Added to `context.rs::build_context`. Pulls per-user channel configs from the DB and produces something like:

```
## Channels
Your partner is reachable via the `message` tool on these channels:
- discord: chat_id="dm/<partner_discord_id>"
- telegram: chat_id="<partner_telegram_id>"
- gateway: no chat_id needed — messages post to the current session

Current session: channel=discord, chat_id=guild123/channel456
Reply on the current channel unless the partner asks otherwise.
```

Each line appears only if that channel is configured and enabled for this user:

| Channel | Shown when | `chat_id` format |
|---|---|---|
| discord | `discord_configs.enabled = true` for user | `"dm/<partner_discord_id>"` |
| telegram | `telegram_configs.enabled = true` for user | `"<partner_telegram_id>"` |
| gateway | always (every user has gateway) | n/a |

DB queries happen inside `build_context` — one SELECT per configured channel per context build. At 500 concurrent sessions this is trivial cost.

### 3.6 Non-partner security guard in `message` tool

`server_tools/message.rs` reads `channel` and `chat_id` from tool args. If the sender is non-partner (`ctx.identity.as_ref().map_or(false, |i| !i.is_partner)`), the tool must reject any combination where `(channel, chat_id)` is not the incoming session's own. Specifically:

```rust
if let Some(id) = ctx.identity.as_ref() {
    if !id.is_partner && (channel != ctx.channel || chat_id != ctx.chat_id) {
        return (1, "Non-partner senders cannot send across channels".into());
    }
}
```

Partners have no restriction. Cron events have `identity: None` and are treated as partner-equivalent (trusted server-originated).

### 3.7 Out-of-scope notes

**Frontend notification UX.** Handling the `session_update` frame in `plexus-frontend/src/lib/ws.ts` + store + UI badge is a separate task. This spec stops at the gateway emitting the frame correctly; the first frontend landing could do nothing useful with it and everything still works via session-history fetches on session-switch.

**Stale session_update frames when gateway has no browsers.** Fine. The message is already in the DB; next reconnect shows it.

## 4. File structure

| File | Change type |
|---|---|
| `plexus-common/src/protocol.rs` | No change — gateway↔server WS uses untyped `serde_json::Value`; no common types to edit |
| `plexus-gateway/src/state.rs` | Drop `#[allow(dead_code)]` on `BrowserConnection.user_id` |
| `plexus-gateway/src/routing.rs` | New `route_session_update(state, user_id, session_id)` |
| `plexus-gateway/src/ws/plexus.rs` | Handle `type="session_update"` frames from plexus-server |
| `plexus-server/src/channels/gateway.rs` | `deliver` emits `session_update` frame |
| `plexus-server/src/channels/discord.rs` | DM-channel cache + `resolve_chat_id` helper + `deliver` calls resolver |
| `plexus-server/src/server_tools/message.rs` | Non-partner security guard |
| `plexus-server/src/context.rs` | New `## Channels` section in `build_context` (+ small DB helper) |

No DB schema change. No `plexus-common` change.

## 5. Testing strategy

Each task below is test-first. The tests use unit-test granularity inside `#[cfg(test)] mod tests` blocks. Gateway routing + server deliver are tested with JSON assertions on the emitted frame. Discord DM caching is tested via the parser and cache layers (no live Discord API). Non-partner guard is tested with a synthetic `ToolContext`. System prompt is tested by asserting sub-strings in `build_context`'s output for a stubbed user.

End-to-end manual smoke lives in the final task.

## 6. Implementation tasks

Sequential, independent modules, all use TDD. Each task = one commit.

---

### Task 1: Gateway `session_update` routing (receiver side)

**Files:**
- Modify: `plexus-gateway/src/routing.rs`, `plexus-gateway/src/ws/plexus.rs`, `plexus-gateway/src/state.rs`

- [ ] **Step 1: Failing test — routing fans out to matching user_id**

In `plexus-gateway/src/routing.rs`, extend the `#[cfg(test)]` module:

```rust
#[test]
fn test_route_session_update_fans_out_by_user_id() {
    let state = Arc::new(AppState {
        config: test_config(),
        browsers: Arc::new(DashMap::new()),
        plexus: Arc::new(RwLock::new(None)),
        http_client: reqwest::Client::new(),
        shutdown: CancellationToken::new(),
    });

    // Browser A: user=alice
    let (tx_a, mut rx_a) = mpsc::channel::<OutboundFrame>(8);
    state.browsers.insert("chat-a".into(), BrowserConnection {
        tx: tx_a, user_id: "alice".into(), cancel: CancellationToken::new(),
    });
    // Browser B: user=bob
    let (tx_b, mut rx_b) = mpsc::channel::<OutboundFrame>(8);
    state.browsers.insert("chat-b".into(), BrowserConnection {
        tx: tx_b, user_id: "bob".into(), cancel: CancellationToken::new(),
    });
    // Browser C: user=alice, second device
    let (tx_c, mut rx_c) = mpsc::channel::<OutboundFrame>(8);
    state.browsers.insert("chat-c".into(), BrowserConnection {
        tx: tx_c, user_id: "alice".into(), cancel: CancellationToken::new(),
    });

    route_session_update(&state, "alice", "session-42");

    // A and C receive, B does not
    match rx_a.try_recv() {
        Ok(OutboundFrame::SessionUpdate(v)) => {
            assert_eq!(v["type"], "session_update");
            assert_eq!(v["session_id"], "session-42");
        }
        other => panic!("A expected SessionUpdate, got {other:?}"),
    }
    assert!(matches!(rx_c.try_recv(), Ok(OutboundFrame::SessionUpdate(_))));
    assert!(rx_b.try_recv().is_err()); // bob got nothing
}

fn test_config() -> crate::config::Config {
    // build a minimal Config; mirror the style used in existing tests
    ...
}
```

(Use the existing `test_config()` or construct one matching whatever shape the module's tests use.)

- [ ] **Step 2: Run test — must fail**

```bash
cd Plexus && cargo test -p plexus-gateway test_route_session_update_fans_out_by_user_id
```

Expected: `route_session_update` not found; `OutboundFrame::SessionUpdate` variant not found.

- [ ] **Step 3: Add `OutboundFrame::SessionUpdate` variant**

In `plexus-gateway/src/state.rs`, extend:

```rust
pub enum OutboundFrame {
    Message(serde_json::Value),
    Progress(serde_json::Value),
    Error(serde_json::Value),
    Ping,
    SessionUpdate(serde_json::Value),  // NEW
}
```

Update any match on `OutboundFrame` in `plexus-gateway/src/ws/chat.rs` or elsewhere — serialize the `SessionUpdate` variant the same way as `Message` (as text). Grep:

```bash
grep -rn "OutboundFrame::" plexus-gateway/src/
```

- [ ] **Step 4: Drop `#[allow(dead_code)]` from `BrowserConnection`**

Change in `plexus-gateway/src/state.rs`:

```rust
#[derive(Clone)]
pub struct BrowserConnection {
    pub tx: mpsc::Sender<OutboundFrame>,
    pub user_id: String,  // now read by routing
    pub cancel: CancellationToken,
}
```

Remove the `#[allow(dead_code)]` attribute from the struct or field.

- [ ] **Step 5: Implement `route_session_update`**

In `plexus-gateway/src/routing.rs`:

```rust
pub fn route_session_update(state: &Arc<AppState>, user_id: &str, session_id: &str) {
    let frame_json = serde_json::json!({
        "type": "session_update",
        "session_id": session_id,
    });
    let mut fanout_count = 0;
    for entry in state.browsers.iter() {
        if entry.value().user_id != user_id {
            continue;
        }
        let frame = OutboundFrame::SessionUpdate(frame_json.clone());
        if entry.value().tx.try_send(frame).is_err() {
            tracing::warn!("session_update: backpressure for chat_id={}", entry.key());
        } else {
            fanout_count += 1;
        }
    }
    tracing::debug!("session_update user_id={user_id} session_id={session_id} fanout={fanout_count}");
}
```

- [ ] **Step 6: Wire the handler in `ws/plexus.rs`**

Find the existing `match msg_type { "send" => route_send(...), _ => warn ... }` block. Extend:

```rust
"send" => { crate::routing::route_send(&state, &parsed); }
"session_update" => {
    let user_id = parsed.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
    let session_id = parsed.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    if user_id.is_empty() || session_id.is_empty() {
        warn!("session_update frame missing user_id or session_id");
    } else {
        crate::routing::route_session_update(&state, user_id, session_id);
    }
}
_ => warn!("ws_plexus: unknown message type: {msg_type}"),
```

- [ ] **Step 7: Handle serialization in `ws/chat.rs`**

Find the block that serializes `OutboundFrame` values into WS text frames (it already handles `Message`, `Progress`, `Error`, `Ping`). Add the `SessionUpdate` arm — emits the inner JSON value as a `tungstenite::Message::Text`, same as `Message`.

- [ ] **Step 8: Run test — must pass**

```bash
cargo test -p plexus-gateway test_route_session_update_fans_out_by_user_id
```

- [ ] **Step 9: Full gateway test suite**

```bash
cargo test -p plexus-gateway
```

All existing tests must still pass.

- [ ] **Step 10: Commit**

```bash
git add plexus-gateway/src/
git commit -m "$(cat <<'EOF'
gateway: add session_update frame routed by user_id

New OutboundFrame::SessionUpdate variant and route_session_update
function that fans out notification frames to every browser whose
BrowserConnection.user_id matches. Sender side (plexus-server) lands
in the next commit; this commit introduces the receiver and the
frame type.

BrowserConnection.user_id's dead_code annotation drops — routing
now reads it.
EOF
)"
```

---

### Task 2: Server `channels::gateway::deliver` emits `session_update`

**Files:** `plexus-server/src/channels/gateway.rs`

- [ ] **Step 1: Failing test**

In the existing `#[cfg(test)] mod tests` of `plexus-server/src/channels/gateway.rs`:

```rust
#[test]
fn test_deliver_produces_session_update_frame() {
    let event = OutboundEvent {
        channel: "gateway".into(),
        chat_id: Some("stale-chat-id".into()),
        session_id: "session-123".into(),
        user_id: "user-alice".into(),
        content: "hello".into(),
        media: vec![],
        is_progress: false,
        metadata: Default::default(),
    };
    let frame = build_deliver_frame(&event);
    assert_eq!(frame["type"], "session_update");
    assert_eq!(frame["user_id"], "user-alice");
    assert_eq!(frame["session_id"], "session-123");
    // Chat_id is irrelevant and must NOT leak into the outbound frame.
    assert!(frame.get("chat_id").is_none());
    assert!(frame.get("content").is_none());
}
```

(`build_deliver_frame` is a new pure helper extracted from `deliver` to make testing easy; `deliver` remains async and calls it.)

- [ ] **Step 2: Run test — fails (no `build_deliver_frame`)**

```bash
cargo test -p plexus-server channels::gateway::tests::test_deliver_produces_session_update_frame
```

- [ ] **Step 3: Refactor `deliver` to use a pure helper**

Current shape of `deliver`:

```rust
pub async fn deliver(state: &AppState, event: &OutboundEvent) {
    let sink = state.gateway_sink.read().await;
    let Some(sink) = sink.as_ref() else {
        warn!("Gateway: not connected, dropping outbound message");
        return;
    };
    let mut msg = serde_json::json!({ "type": "send", "chat_id": event.chat_id, ... });
    // ... metadata population ...
    let json = serde_json::to_string(&msg).unwrap();
    // ... send to sink ...
}
```

Replace with:

```rust
pub async fn deliver(state: &AppState, event: &OutboundEvent) {
    let sink = state.gateway_sink.read().await;
    let Some(sink) = sink.as_ref() else {
        warn!("Gateway: not connected, dropping outbound message");
        return;
    };
    let msg = build_deliver_frame(event);
    let json = serde_json::to_string(&msg).unwrap();
    let mut s = sink.lock().await;
    if let Err(e) = futures_util::SinkExt::send(
        &mut *s,
        tokio_tungstenite::tungstenite::Message::Text(json.into()),
    )
    .await
    {
        warn!("Gateway: send failed: {e}");
    }
}

/// Build the WS frame sent to plexus-gateway. Always a session_update
/// pointer — browsers fetch content via REST.
fn build_deliver_frame(event: &OutboundEvent) -> serde_json::Value {
    serde_json::json!({
        "type": "session_update",
        "user_id": event.user_id,
        "session_id": event.session_id,
    })
}
```

Drop all metadata / media / progress / chat_id assembly — none of it is needed for the notification.

- [ ] **Step 4: Run the full server test suite**

```bash
cargo test -p plexus-server
```

The new test passes; other tests still pass (the gateway-inbound tests for `extract_media` are unrelated and continue working).

- [ ] **Step 5: Commit**

```bash
git add plexus-server/src/channels/gateway.rs
git commit -m "$(cat <<'EOF'
gateway outbound: emit session_update instead of send

Gateway deliver no longer routes by chat_id (which is a per-connect
UUID and goes stale across browser reconnects). Frame becomes a
user_id+session_id notification; plexus-gateway fans it out to every
browser for that user, and each browser fetches the session content
via the existing REST history endpoint.

Drops chat_id, content, media, and progress metadata from the
outbound — the notification is a pointer, not the payload.
EOF
)"
```

---

### Task 3: Discord DM-channel cache + `resolve_chat_id` helper

**Files:** `plexus-server/src/channels/discord.rs`

- [ ] **Step 1: Failing tests**

In `plexus-server/src/channels/discord.rs::mod tests`:

```rust
#[test]
fn test_resolve_chat_id_parses_dm_form() {
    assert_eq!(parse_chat_id("dm/123456789"), Some(ChatIdKind::Dm(123456789)));
    assert_eq!(parse_chat_id("dm/"), None);
    assert_eq!(parse_chat_id("dm/abc"), None); // non-numeric
}

#[test]
fn test_resolve_chat_id_parses_guild_form() {
    assert_eq!(
        parse_chat_id("111/222"),
        Some(ChatIdKind::Guild { guild_id: 111, channel_id: 222 }),
    );
    assert_eq!(parse_chat_id("111/abc"), None);
    assert_eq!(parse_chat_id("bad-format"), None);
}
```

- [ ] **Step 2: Run tests — fail**

```bash
cargo test -p plexus-server channels::discord::tests::test_resolve_chat_id
```

- [ ] **Step 3: Add `ChatIdKind` enum + `parse_chat_id` helper**

Above `mod tests` in `plexus-server/src/channels/discord.rs`:

```rust
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChatIdKind {
    Dm(u64),
    Guild { guild_id: u64, channel_id: u64 },
}

pub(crate) fn parse_chat_id(s: &str) -> Option<ChatIdKind> {
    if let Some(rest) = s.strip_prefix("dm/") {
        return rest.parse::<u64>().ok().map(ChatIdKind::Dm);
    }
    let mut parts = s.splitn(2, '/');
    let guild_id: u64 = parts.next()?.parse().ok()?;
    let channel_id: u64 = parts.next()?.parse().ok()?;
    Some(ChatIdKind::Guild { guild_id, channel_id })
}
```

- [ ] **Step 4: Tests pass**

```bash
cargo test -p plexus-server channels::discord::tests::test_resolve_chat_id
```

- [ ] **Step 5: Add DM-channel cache to `DiscordHandler`**

Find the `DiscordHandler` struct. Add:

```rust
pub struct DiscordHandler {
    // ... existing fields ...
    pub dm_channel_cache: Arc<DashMap<UserId, ChannelId>>,
}
```

Initialize in `start_bot` alongside the existing `channels` cache: `dm_channel_cache: Arc::new(DashMap::new())`.

- [ ] **Step 6: Update `deliver` to resolve via `parse_chat_id`**

Replace the current chat_id-parsing block in `channels::discord::deliver` (around the `// Parse chat_id to get ChannelId` comment) with:

```rust
let channel_id = match event.chat_id.as_deref() {
    Some(chat_id) => match parse_chat_id(chat_id) {
        Some(ChatIdKind::Guild { channel_id, .. }) => ChannelId::new(channel_id),
        Some(ChatIdKind::Dm(user_id_raw)) => {
            let user_id = UserId::new(user_id_raw);
            // Try cache, else create via http
            let cached = handle.dm_channel_cache.get(&user_id).map(|r| *r);
            match cached {
                Some(cid) => cid,
                None => match user_id.create_dm_channel(&http).await {
                    Ok(dm) => {
                        handle.dm_channel_cache.insert(user_id, dm.id);
                        dm.id
                    }
                    Err(e) => {
                        warn!("Discord: create_dm_channel failed: {e}");
                        return;
                    }
                },
            }
        }
        None => {
            warn!("Discord: invalid chat_id format: {chat_id}");
            return;
        }
    },
    None => { /* existing fallback logic */ }
};
```

Note: serenity's `create_dm_channel` typically takes `&Http` (or `&Context`). `handle.http` is already in scope per the current deliver — reuse it.

- [ ] **Step 7: Build + test**

```bash
cargo build -p plexus-server
cargo test -p plexus-server channels::discord
```

- [ ] **Step 8: Commit**

```bash
git add plexus-server/src/channels/discord.rs
git commit -m "$(cat <<'EOF'
discord: add DM-channel cache and dm/<user_id> chat_id parser

ChatIdKind enum and parse_chat_id helper turn chat_id strings into
typed variants: dm/<user_id> → DM to that user (lazily create + cache
the serenity ChannelId), guild/channel_id → existing guild channel
path. deliver routes accordingly.

This lets the agent reach the owner's DM on Discord even from a
session that started on a different channel.
EOF
)"
```

---

### Task 4: Non-partner security guard in `message` tool

**Files:** `plexus-server/src/server_tools/message.rs`

- [ ] **Step 1: Failing tests**

Add a `#[cfg(test)] mod tests` at the bottom of `plexus-server/src/server_tools/message.rs`. Test the pure guard function:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn identity(is_partner: bool) -> crate::context::ChannelIdentity {
        crate::context::ChannelIdentity {
            sender_name: "s".into(),
            sender_id: "s_id".into(),
            is_partner,
            partner_name: "p".into(),
            partner_id: "p_id".into(),
            channel_type: "discord".into(),
        }
    }

    #[test]
    fn test_guard_allows_partner_cross_channel() {
        let err = check_cross_channel(
            Some(&identity(true)),
            "discord", Some("c1"),
            "telegram", Some("c2"),
        );
        assert!(err.is_none(), "partner should be allowed cross-channel");
    }

    #[test]
    fn test_guard_allows_non_partner_same_channel() {
        let err = check_cross_channel(
            Some(&identity(false)),
            "discord", Some("c1"),
            "discord", Some("c1"),
        );
        assert!(err.is_none(), "non-partner should be allowed on same channel");
    }

    #[test]
    fn test_guard_blocks_non_partner_cross_channel() {
        let err = check_cross_channel(
            Some(&identity(false)),
            "discord", Some("c1"),
            "telegram", Some("c2"),
        );
        assert!(err.is_some(), "non-partner cross-channel must be rejected");
    }

    #[test]
    fn test_guard_allows_cron_no_identity() {
        // Cron events have ctx.identity = None and are trusted.
        let err = check_cross_channel(None, "gateway", Some("s1"), "telegram", Some("c2"));
        assert!(err.is_none());
    }
}
```

- [ ] **Step 2: Run — fail**

```bash
cargo test -p plexus-server server_tools::message
```

- [ ] **Step 3: Implement `check_cross_channel`**

Above `mod tests` in `plexus-server/src/server_tools/message.rs`:

```rust
/// Enforce the security rule that non-partner senders cannot relay
/// to a different channel or chat_id than their inbound one. Returns
/// an error message on violation, None otherwise.
fn check_cross_channel(
    identity: Option<&crate::context::ChannelIdentity>,
    ctx_channel: &str,
    ctx_chat_id: Option<&str>,
    target_channel: &str,
    target_chat_id: Option<&str>,
) -> Option<String> {
    let Some(id) = identity else { return None; }; // cron / server-initiated → trusted
    if id.is_partner { return None; }
    if ctx_channel != target_channel || ctx_chat_id != target_chat_id {
        return Some(
            "Non-partner senders cannot relay messages to a different channel or chat_id".into(),
        );
    }
    None
}
```

- [ ] **Step 4: Call the guard from the existing `message` tool handler**

Find the point in the handler where `channel` and `chat_id` are resolved from args (around line 13-21 per earlier grep). Immediately after:

```rust
if let Some(err) = check_cross_channel(
    ctx.identity.as_ref(),
    &ctx.channel,
    ctx.chat_id.as_deref(),
    &channel,
    chat_id.as_deref(),
) {
    return (1, err);
}
```

(Adapt `ctx.identity` field access to the real `ToolContext` shape — grep if unsure.)

- [ ] **Step 5: Run tests**

```bash
cargo test -p plexus-server
```

All pass including the four new guard tests.

- [ ] **Step 6: Commit**

```bash
git add plexus-server/src/server_tools/message.rs
git commit -m "$(cat <<'EOF'
message tool: block non-partner cross-channel relays

Non-partner senders (authorized but not the owner) can only emit
OutboundEvents on their own inbound channel/chat_id. Partners and
cron (identity=None) are unrestricted.
EOF
)"
```

---

### Task 5: System prompt `## Channels` section

**Files:** `plexus-server/src/context.rs` (+ small DB reads)

- [ ] **Step 1: Failing tests**

Add to `context.rs::mod tests`:

```rust
#[tokio::test]
async fn test_channels_section_lists_discord_when_enabled() {
    let snapshot = ChannelSnapshot {
        discord_partner_id: Some("owner_dc".into()),
        telegram_partner_id: None,
    };
    let section = render_channels_section(&snapshot);
    assert!(section.contains(r#"chat_id="dm/owner_dc""#));
    assert!(!section.contains("telegram"));
    assert!(section.contains("gateway"));
}

#[tokio::test]
async fn test_channels_section_lists_telegram_when_enabled() {
    let snapshot = ChannelSnapshot {
        discord_partner_id: None,
        telegram_partner_id: Some("owner_tg".into()),
    };
    let section = render_channels_section(&snapshot);
    assert!(!section.contains("discord"));
    assert!(section.contains(r#"chat_id="owner_tg""#));
}

#[tokio::test]
async fn test_channels_section_only_gateway_when_none_configured() {
    let snapshot = ChannelSnapshot {
        discord_partner_id: None,
        telegram_partner_id: None,
    };
    let section = render_channels_section(&snapshot);
    assert!(!section.contains("discord"));
    assert!(!section.contains("telegram"));
    assert!(section.contains("gateway"));
}
```

- [ ] **Step 2: Tests fail**

```bash
cargo test -p plexus-server context::tests::test_channels_section
```

- [ ] **Step 3: Add `ChannelSnapshot` and `render_channels_section`**

In `context.rs`, above `build_context`:

```rust
/// Per-user channel configuration summary used to render the
/// ## Channels section. `None` fields mean the channel is not
/// configured or not enabled.
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
```

- [ ] **Step 4: Load snapshot from DB inside `build_context`**

Add a small loader:

```rust
async fn load_channel_snapshot(state: &AppState, user_id: &str) -> ChannelSnapshot {
    let discord = crate::db::discord::get(&state.db, user_id).await.ok().flatten()
        .filter(|c| c.enabled)
        .map(|c| c.partner_discord_id);
    let telegram = crate::db::telegram::get(&state.db, user_id).await.ok().flatten()
        .filter(|c| c.enabled)
        .map(|c| c.partner_telegram_id);
    ChannelSnapshot {
        discord_partner_id: discord,
        telegram_partner_id: telegram,
    }
}
```

(Adapt field names — e.g., `partner_discord_id` — to match the actual `DiscordConfig` struct. Grep `plexus-server/src/db/discord.rs` if uncertain.)

- [ ] **Step 5: Wire the section into `build_context`**

Find the system-prompt assembly in `build_context`. After the existing `## Attachments` section (or wherever logically fits — probably right after `## Identity`, before `## Attachments`), add:

```rust
let snap = load_channel_snapshot(state, &user.user_id).await;
system += &render_channels_section(&snap);
system += "\n";
```

Also append the current-session reminder:

```rust
if let Some(id) = identity {
    system += &format!(
        "Current session: channel={}, chat_id={}\n",
        id.channel_type,
        chat_id.unwrap_or("(none)"),
    );
    system += "Reply on the current channel unless the partner asks otherwise.\n\n";
}
```

- [ ] **Step 6: Tests pass**

```bash
cargo test -p plexus-server
```

- [ ] **Step 7: Commit**

```bash
git add plexus-server/src/context.rs
git commit -m "$(cat <<'EOF'
system prompt: add ## Channels section listing partner addresses

Per-user DB snapshot of enabled Discord/Telegram configs produces a
compact list of addressable chat_id shapes: dm/<discord_user_id>,
<telegram_chat_id>, and gateway (no chat_id). Plus a current-session
reminder so the agent replies on the originating channel by default.

Only enabled channels appear — a user with only the gateway sees
just the gateway line.
EOF
)"
```

---

### Task 6: Frontend — handle `session_update` frame

**Files:** `plexus-frontend/src/lib/ws.ts`, `plexus-frontend/src/store/chat.ts`

- [ ] **Step 1: Read current WS frame handling**

Grep to find how frames are parsed in the frontend:

```bash
grep -n "\"type\"\|msg.type\|onmessage" plexus-frontend/src/lib/ws.ts plexus-frontend/src/store/chat.ts | head -20
```

Identify where `message`, `progress`, `error` etc. are dispatched.

- [ ] **Step 2: Add `session_update` handler**

In whichever file dispatches by frame type, add a branch:

```typescript
case 'session_update': {
    const sessionId = (data as { session_id?: string }).session_id
    if (!sessionId) return
    // If the user is currently viewing this session, refetch its
    // messages. Otherwise, mark it as having new content so the
    // session list can render an indicator.
    void useChatStore.getState().refreshSession(sessionId)
    break
}
```

- [ ] **Step 3: Add `refreshSession` to the chat store**

In `plexus-frontend/src/store/chat.ts`, add a `refreshSession(sessionId: string): Promise<void>` method that fetches `GET /api/sessions/<id>/messages` via the existing `api` wrapper and merges the result into the store. (If the store already has a similar session-load function, reuse it.)

- [ ] **Step 4: Manual browser smoke**

Build the frontend and manually check:
- Trigger a cron fire. Verify the browser receives a `session_update` frame (DevTools → Network → WS → Messages).
- Verify the session in the UI updates (if browser is on that session) or the session list shows a fresh timestamp (if not).

- [ ] **Step 5: Commit**

```bash
git add plexus-frontend/src/
git commit -m "$(cat <<'EOF'
frontend: handle session_update frame from gateway

When a session gets new content server-side (typically from cron or
cross-channel agent-initiated messages), gateway now emits a
session_update pointer. The frontend refetches that session's
messages via REST so the UI stays consistent. Active viewers see
the update inline; non-active sessions can surface a badge (follow-up).
EOF
)"
```

---

### Task 7: Manual E2E smoke tests

- [ ] **Step 1: Cron persistence test**

1. Open the web UI, create a cron job: "remind me in 2 minutes".
2. Close the browser tab.
3. Wait for the cron to fire.
4. Reopen the UI. Navigate to the session list. The cron-triggered session should be present with the reminder content.

- [ ] **Step 2: Cross-channel reply test**

Prerequisite: both Discord and Telegram bots configured for your user.

1. DM the bot on Discord: "send a test message to me on Telegram."
2. The agent should call `message(channel="telegram", chat_id="<your_telegram_id>", content="...")`.
3. Verify the message arrives on Telegram.
4. Verify nothing duplicated on Discord.

- [ ] **Step 3: Non-partner guard test**

1. Configure a non-partner but allowed Discord user in your `allowed_users` list.
2. From that second user, DM the bot: "send a message to the owner's Telegram."
3. The agent should refuse (tool guard returns error 1).
4. Verify no Telegram message arrives.

- [ ] **Step 4: Live-browser notification test**

1. Open two browser tabs viewing different sessions.
2. Trigger a cron fire on session A.
3. The tab viewing session A should refresh automatically.
4. The tab viewing session B should see the session-list update (manual eyeballs for now; follow-up frontend work can add a badge).

- [ ] **Step 5: Gateway outbound with stale chat_id**

1. On the web UI, schedule a cron.
2. Reload the browser (new chat_id is assigned).
3. Wait for the cron to fire.
4. Confirm the message still lands in the session — not dropped.

---

## 7. Out-of-scope follow-ups

- Frontend session-list badge indicator when a non-active session receives a `session_update`.
- Discord guild-channel targeting (paste-a-chat-id workflow). Only needed if a user wants "send daily report to #reports".
- Push notifications for offline browsers (would require a PWA service worker).
- Telegram group targeting beyond what inbound caching already provides.
- Retry policy for `message` calls to unconfigured channels — currently log-and-drop.
