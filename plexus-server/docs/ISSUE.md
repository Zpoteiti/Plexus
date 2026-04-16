# Issues — plexus-server

Open, deferred, and closed issues for the server crate. Updated by `/wrap-up` at the end of each session.

## Open
<!-- Active issues that need attention this session or next -->
- [ ] Implement account deletion (spec: docs/superpowers/plans/2026-04-16-account-deletion.md) — 9 tasks, TDD per task; unblocks users leaving the system (2026-04-16)

## Deferred
<!-- Acknowledged but intentionally postponed — include context and date -->
- [ ] Manual E2E smoke tests for inbound-media (context: Task 18 of plans/2026-04-15-inbound-media.md — Discord/Telegram photo + voice, browser drag+paste, strip-retry, context rebuild. Requires live bots; not a code task, 2026-04-16)
- [ ] Manual E2E smoke tests for cross-channel addressing (context: Task CC-7 of plans/2026-04-16-cross-channel-addressing.md — cron across browser reconnect, Discord→Telegram cross-channel, non-partner guard, live browser notification, 2026-04-16)
- [ ] `SessionHandle.user_id` remains `#[allow(dead_code)]` scaffolding (context: will become load-bearing when account deletion lands — iterating `state.sessions` to evict entries for a deleted user needs this field, 2026-04-16)
- [ ] Mid-ReAct-turn image re-read edge case (context: if an agent needs to "look again" at a user image mid-turn, the rebuilt context now rehydrates from DB JSON so this works post-Task-19; no known concrete failure case today but worth watching, 2026-04-16)
- [ ] Whisper / voice transcription (context: spec decision 2026-04-15 — voice notes save to file store as-is; users wire their own transcription via `file_transfer` to a client with whisper-cpp or similar; no server-side ASR)
- [ ] Admin UI for listing/searching/deleting users (context: admin endpoint `DELETE /api/admin/users/{id}` added by account-deletion plan; full admin panel UX is a separate ticket, 2026-04-16)
- [ ] Last-admin invariant not enforced (context: ADR-33 — admin can delete their own account with a warn log; if they were the only admin, re-bootstrap requires direct DB access. Low-risk for small deployments, 2026-04-16)
- [ ] Discord/Telegram bots and per-session agent loops not gracefully shutdown-aware (context: ADR-34 — current graceful shutdown drains HTTP + 5 background loops; individual bots and session agents drop at runtime teardown. Acceptable for M2, 2026-04-16)

## Closed
<!-- Resolved issues — keep for historical context -->
- [x] Telegram dispatcher `shutdown_token().shutdown()` returned a Future that was never awaited (resolved: now matched and awaited in channels/telegram.rs:113, commit e56780c, 2026-04-15)
- [x] `InboundEvent.sender_id` was set by every channel adapter but never read (resolved: deleted; `ChannelIdentity.sender_id` is the one source of truth, commit e56780c, 2026-04-15)
- [x] `InboundEvent.metadata`, `ChannelIdentity.partner_name`, `ChannelIdentity.partner_id` all populated but never consumed (resolved: deleted; cross-channel addressing uses a DB-query path in `context::load_channel_snapshot` rather than these fields, commit 426c5b5, 2026-04-16)
- [x] `AppState.shutdown` CancellationToken was created but never cancelled/awaited, SIGTERM killed everything mid-flight (resolved: signal handler + `with_graceful_shutdown` + tokio::select! on all 5 background loops, commit 96c3c2f, 2026-04-16)
- [x] Four dead-scaffolded functions with no callers (ServerMcpManager::has_tool, ServerMcpManager::session_count, db::skills::find_by_name, Content::as_text) (resolved: deleted; each is ~5 lines to re-add if a real caller shows up, commit ae07ff4, 2026-04-16)
- [x] OutboundEvent carried `is_progress` and `metadata` fields that no deliver function read (resolved: deleted after gateway::deliver simplified to emit session_update pointer in CC-2, commit 052edb3, 2026-04-16)
- [x] No inbound-media support on any channel (resolved: full pipeline shipped — channel adapters download → file_store → InboundEvent.media → build_user_content produces OpenAI content blocks → persisted as Content::Blocks JSON in DB. Images render inline for VLMs; non-images flow through file_transfer. Commits fda851b..e848798, 2026-04-15/16)
- [x] No cross-channel addressing ("reminder on Telegram from a Discord session" silently fails) (resolved: ADR-31 + ADR-32 — Discord DM cache, Telegram DM via partner_telegram_id, gateway via session_update frame; `## Channels` system-prompt section exposes addressable chat_id shapes. Commits ba56f34..176961f, 2026-04-16)
- [x] 20+ compile warnings from stale scaffolding (resolved across the session — most via deletion, remaining ones via field-level `#[allow(dead_code)]` on SessionHandle.user_id; build is now zero-warning, 2026-04-16)
