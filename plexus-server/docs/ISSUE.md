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

### Workspace Frontend (Plan B)

- **File rename endpoint** — spec §7.2 lists rename as a UI action, but §7.3's endpoint list omitted it. v1 requires delete + re-upload. Re-add a `POST /api/workspace/rename` with `{from, to}` if users ask.
- **Frontend test harness** — `plexus-frontend` has no Vitest/Jest/RTL/Playwright setup. Plan B's verification is manual smoke + visual review. Wiring up Vitest + React Testing Library is a post-M2 effort.
- **Bulk file operations** — multi-file select, bulk delete, bulk move. Single-file ops only in v1.
- **File-type coverage for inline preview** — SVG, HEIC, PDF, and video files currently fall through to the binary-metadata branch. Extending inline preview requires type-specific renderers (`<object>` for PDF, `<video>` for video, etc.).
- **Upload "ERROR:" sentinel** — the partial-success response shape uses a string prefix to indicate per-file failure. A cleaner shape would be `{path, size_bytes, error?: string}`. Both server and frontend would need to move together.
- **Server-pushed tree invalidation** — the tree refetches on every mutation initiated by the user in this tab, but an agent write (e.g., dream creating a new skill) doesn't push an invalidation. A WebSocket event or SSE push would let the tree auto-refresh while the page is open.
- **`WorkspaceError::Quota(QuotaError)` variant** — B-4 bridged QuotaError via `Io(Other, "quota: …")` string-prefix, with the HTTP wrapper string-matching on "quota" to route to 422. Add a proper enum variant + explicit match arm.
- **`GET /api/workspace/file` streaming** — B-3 uses `tokio::fs::read` (entire file into memory). For files near the 4 GB per-upload cap, serving via `Body::from(Vec<u8>)` is an OOM risk. Replace with `tokio::fs::File` + streaming body before this endpoint sees large-file traffic.
- **`X-Content-Type-Options: nosniff`** — B-3 omits this defense-in-depth header that `download_file` sets. Add for consistency.

### Heartbeat (Plan E)

- **Heartbeat multi-server deduplication** — the in-process tick loop refires per server. Single-node deployments are unaffected; multi-server needs either a leader-election pattern or a pg advisory lock held across the tick iteration. Tracked for post-M2.
- **Heartbeat session retention / log UI** — `heartbeat:{user_id}` sessions and messages accumulate indefinitely. Spec §9.7 mentions a future "Heartbeat Log" frontend page; no GC policy ships in M2.
- **Heartbeat Phase 2 error retry** — Phase 2 errors log and exit; `last_heartbeat_at` stays advanced. No retry; next window gets a fresh shot. Acceptable as autonomous-best-effort, but noted for observability work.
- **Heartbeat observability** — a consistently-skipping Phase 1 (e.g. broken LLM config) is silent beyond `info!` logs. A metrics-based alert would surface regressions; deferred.
- **Heartbeat delivery-path test coverage** — `publish_final_heartbeat`'s Discord/Telegram precedence paths only have the silent/no-config test (E-6). Covering the notify:true branches requires either a real LLM + real DB fixture or a test-double evaluator abstraction. Deferred.

## Closed
<!-- Resolved issues — keep for historical context -->
- [x] **SCHEMA.md full docs-sync pass** — drifted over Plans A/C/D/E + account deletion. Fixed: users table (added display_name, timezone, last_dream_at, last_heartbeat_at; removed stale soul/memory_text); device_tokens (added workspace_path, shell_timeout, ssrf_whitelist); discord_configs (owner_discord_id → partner_discord_id; corrected nullability); cron_jobs (added kind, deliver, claimed_at, last_status; partial unique index); telegram_configs added (was entirely missing); skills table removed (dropped by A-17); ON DELETE CASCADE noted on all user-referencing FKs (AD-1); system_config known-keys expanded with default_memory, default_heartbeat, heartbeat_phase1_prompt, heartbeat_interval_seconds, dream_enabled, workspace_quota_bytes, dream_phase1_prompt, dream_phase2_prompt. (commit: pending, 2026-04-18)
- [x] Telegram dispatcher `shutdown_token().shutdown()` returned a Future that was never awaited (resolved: now matched and awaited in channels/telegram.rs:113, commit e56780c, 2026-04-15)
- [x] `InboundEvent.sender_id` was set by every channel adapter but never read (resolved: deleted; `ChannelIdentity.sender_id` is the one source of truth, commit e56780c, 2026-04-15)
- [x] `InboundEvent.metadata`, `ChannelIdentity.partner_name`, `ChannelIdentity.partner_id` all populated but never consumed (resolved: deleted; cross-channel addressing uses a DB-query path in `context::load_channel_snapshot` rather than these fields, commit 426c5b5, 2026-04-16)
- [x] `AppState.shutdown` CancellationToken was created but never cancelled/awaited, SIGTERM killed everything mid-flight (resolved: signal handler + `with_graceful_shutdown` + tokio::select! on all 5 background loops, commit 96c3c2f, 2026-04-16)
- [x] Four dead-scaffolded functions with no callers (ServerMcpManager::has_tool, ServerMcpManager::session_count, db::skills::find_by_name, Content::as_text) (resolved: deleted; each is ~5 lines to re-add if a real caller shows up, commit ae07ff4, 2026-04-16)
- [x] OutboundEvent carried `is_progress` and `metadata` fields that no deliver function read (resolved: deleted after gateway::deliver simplified to emit session_update pointer in CC-2, commit 052edb3, 2026-04-16)
- [x] No inbound-media support on any channel (resolved: full pipeline shipped — channel adapters download → file_store → InboundEvent.media → build_user_content produces OpenAI content blocks → persisted as Content::Blocks JSON in DB. Images render inline for VLMs; non-images flow through file_transfer. Commits fda851b..e848798, 2026-04-15/16)
- [x] No cross-channel addressing ("reminder on Telegram from a Discord session" silently fails) (resolved: ADR-31 + ADR-32 — Discord DM cache, Telegram DM via partner_telegram_id, gateway via session_update frame; `## Channels` system-prompt section exposes addressable chat_id shapes. Commits ba56f34..176961f, 2026-04-16)
- [x] 20+ compile warnings from stale scaffolding (resolved across the session — most via deletion, remaining ones via field-level `#[allow(dead_code)]` on SessionHandle.user_id; build is now zero-warning, 2026-04-16)

## Plan D follow-ups (deferred)

- [ ] **Dream end-to-end integration testing.** Unit tests cover the idle-check short-circuit, the allowlist matrix, and the `PromptMode::Dream` context-builder output shape. True end-to-end tests (full Phase 1 + Phase 2 against a real LLM, verifying directives are applied to `MEMORY.md`/skills) are deferred — they need either a mock LLM fixture or a staging environment with an API key.
- [ ] **Dream session retention / GC.** `dream:*` sessions + their messages accumulate in the DB. No retention policy yet. If table size becomes a concern at scale, a periodic "prune dream sessions older than N days" task is the likely answer.
- [ ] **Observability for dream degradation.** Today a broken LLM causes dream to silently skip with a `warn!` every 2h per user. Consider adding a health-check endpoint exposing `dream_phase1_last_successful_at` per user, or a prometheus counter, so operators notice.
- [ ] **Tail-drop on 200-message cap.** `PHASE1_MESSAGE_CAP = 200` drops the 201st..Nth messages of a window from consolidation; `last_dream_at` advances to NOW so those messages are outside the next window too. Consider advancing to the 200th message's `created_at` instead of NOW when the cap fires.
