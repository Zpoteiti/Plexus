# Issues — plexus-gateway

Open, deferred, and closed issues for the gateway crate. Updated by `/wrap-up` at the end of each session.

## Open
<!-- Active issues that need attention this session or next -->

## Deferred
<!-- Acknowledged but intentionally postponed — include context and date -->
- [ ] Push notifications for offline browsers (context: `session_update` only reaches connected clients; offline browsers discover new messages on next login via session history fetch. A PWA service worker would deliver real-time pings, 2026-04-16)

## Closed
<!-- Resolved issues — keep for historical context -->
- [x] Frontend session-list badge indicator for non-active sessions that receive a `session_update` frame (resolved: Zustand `Session` type gained `hasUnread: boolean`; session_update handler sets it on non-current sessions; Sidebar renders a 2x2 accent dot. Commit 51d4468, 2026-04-19)
- [x] Gateway uses transient WS connection UUID as chat_id in InboundEvent instead of stable session_id (resolved: ADR-31 replaces chat_id-based outbound routing with user_id+session_id-based fan-out via a new `session_update` frame; cron delivery now survives browser reconnects. Commits ba56f34..176961f, 2026-04-16)
