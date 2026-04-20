# Issues — plexus-frontend

Open, deferred, and closed issues for the frontend. Updated by `/wrap-up` at the end of each session.

## Open
<!-- Active issues that need attention this session or next -->

## Deferred
<!-- Acknowledged but intentionally postponed — include context and date -->
- [ ] No vitest/testing-library harness installed — frontend tests are currently manual-only (context: Tasks 15/16/CC-6/AD-8 all rely on manual smoke; if the surface grows, wiring vitest + react-testing-library would pay off, 2026-04-16)

## Closed
<!-- Resolved issues — keep for historical context -->
- [x] **Frontend cleanup pass (FR2/FR2b/FR3/FR4/FR5)** — chat image-drop now PUTs to workspace `.attachments/` + embeds base64 content blocks; REST-loaded historical messages parse stringified Content::Blocks; Settings device-config editor covers workspace_path/shell_timeout_max/ssrf_whitelist/fs_policy with typed-device-name confirm modal for unrestricted; Workspace.tsx collapses `.attachments/` + renders MEMORY.md/skills/*/SKILL.md as inline markdown with edit mode; Admin.tsx Server MCPs tab with add/edit/remove + env masking. npm typecheck + build green. (commits cf9cf3d..058def2 + a03cf4c, 2026-04-20)
- [x] Account deletion UI (Settings → Danger Zone) — resolved: Danger Zone + password-confirm modal shipped in ProfileTab (commit 8c2e211, 2026-04-19)
- [x] Session-list "unread" badge — resolved: Zustand `Session.hasUnread`; session_update handler sets on non-current; Sidebar renders 2x2 accent dot (commit 51d4468, 2026-04-19)
- [x] ChatInput was text-only, no paperclip/drag-and-drop/paste for file attachments (resolved: Task 15 of inbound-media plan — XHR upload helper with progress, chips UI, 20MB client-side guard, commit c473c18, 2026-04-15)
- [x] Message component rendered attachment URLs as "Attachment N" links with no filename or image preview (resolved: Task 16 — MediaItem helper attempts inline `<img>` with onError fallback to a download chip; applies to both user and assistant messages, commit 1b84dc8, 2026-04-15)
- [x] Frontend ignored `session_update` frames from the gateway (resolved: Task CC-6 — chat store handles the frame, `refreshSession(sessionId)` bypasses the restLoadedSessions guard and refetches via REST, commit 176961f, 2026-04-16)
