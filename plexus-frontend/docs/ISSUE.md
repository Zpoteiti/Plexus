# Issues — plexus-frontend

Open, deferred, and closed issues for the frontend. Updated by `/wrap-up` at the end of each session.

## Open
<!-- Active issues that need attention this session or next -->
- [ ] **Frontend cleanup pass** — backend side of the post-M2 cleanup is complete; frontend calls need to catch up. See plexus-server/docs/ISSUE.md Open for the full scope (P4.6 image-drop to workspace PUT + base64; P7.5 Settings device-config editor with fs_policy typed-confirm; P9.1 Workspace.tsx `.attachments/` collapse + MEMORY.md/SKILL.md inline; P9.2 Admin Server-MCPs tab; P3.7 route migration from `/api/workspace/file?path=` to `/api/workspace/files/{*path}`).

## Deferred
<!-- Acknowledged but intentionally postponed — include context and date -->
- [ ] No vitest/testing-library harness installed — frontend tests are currently manual-only (context: Tasks 15/16/CC-6/AD-8 all rely on manual smoke; if the surface grows, wiring vitest + react-testing-library would pay off, 2026-04-16)

## Closed
<!-- Resolved issues — keep for historical context -->
- [x] Account deletion UI (Settings → Danger Zone) — resolved: Danger Zone + password-confirm modal shipped in ProfileTab (commit 8c2e211, 2026-04-19)
- [x] Session-list "unread" badge — resolved: Zustand `Session.hasUnread`; session_update handler sets on non-current; Sidebar renders 2x2 accent dot (commit 51d4468, 2026-04-19)
- [x] ChatInput was text-only, no paperclip/drag-and-drop/paste for file attachments (resolved: Task 15 of inbound-media plan — XHR upload helper with progress, chips UI, 20MB client-side guard, commit c473c18, 2026-04-15)
- [x] Message component rendered attachment URLs as "Attachment N" links with no filename or image preview (resolved: Task 16 — MediaItem helper attempts inline `<img>` with onError fallback to a download chip; applies to both user and assistant messages, commit 1b84dc8, 2026-04-15)
- [x] Frontend ignored `session_update` frames from the gateway (resolved: Task CC-6 — chat store handles the frame, `refreshSession(sessionId)` bypasses the restLoadedSessions guard and refetches via REST, commit 176961f, 2026-04-16)
