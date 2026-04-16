# Issues — plexus-frontend

Open, deferred, and closed issues for the frontend. Updated by `/wrap-up` at the end of each session.

## Open
<!-- Active issues that need attention this session or next -->
- [ ] Account deletion UI (Settings → Danger Zone) — see plans/2026-04-16-account-deletion.md Task AD-8 (2026-04-16)

## Deferred
<!-- Acknowledged but intentionally postponed — include context and date -->
- [ ] No vitest/testing-library harness installed — frontend tests are currently manual-only (context: Tasks 15/16/CC-6/AD-8 all rely on manual smoke; if the surface grows, wiring vitest + react-testing-library would pay off, 2026-04-16)
- [ ] Session-list "unread" badge when a non-viewed session receives a `session_update` frame (context: `refreshSession` fires regardless of the current view, but there's no visual indicator unless the user is looking at that session; ADR-31 follow-up, 2026-04-16)

## Closed
<!-- Resolved issues — keep for historical context -->
- [x] ChatInput was text-only, no paperclip/drag-and-drop/paste for file attachments (resolved: Task 15 of inbound-media plan — XHR upload helper with progress, chips UI, 20MB client-side guard, commit c473c18, 2026-04-15)
- [x] Message component rendered attachment URLs as "Attachment N" links with no filename or image preview (resolved: Task 16 — MediaItem helper attempts inline `<img>` with onError fallback to a download chip; applies to both user and assistant messages, commit 1b84dc8, 2026-04-15)
- [x] Frontend ignored `session_update` frames from the gateway (resolved: Task CC-6 — chat store handles the frame, `refreshSession(sessionId)` bypasses the restLoadedSessions guard and refetches via REST, commit 176961f, 2026-04-16)
