# Workspace + Autonomy Design Spec

**Date:** 2026-04-17
**Status:** Draft, ready for implementation planning.
**Scope:** Per-user server workspace foundation → dream memory consolidation → heartbeat task wake-up. Plus the frontend Workspace file manager and the nine ADRs these introduce (ADR-35 through ADR-43).
**Parents:** Supersedes Parts A and B of `2026-04-17-m2-closeout-design.md`. The closeout doc is updated to reference this one.

---

## 1. Overview

Plexus M2 currently gives users a ReAct agent with cron, inbound media, cross-channel addressing, compression, and crash recovery. It is missing the two nanobot behaviors that make an agent feel *autonomous*:

- **Dream** — periodic memory consolidation that turns conversation history into structured long-term memory and reusable skills.
- **Heartbeat** — periodic task wake-up that lets the agent pick up and act on user-defined background work.

Adding these on top of the existing architecture surfaced a deeper problem: Plexus's server-side storage is fragmented into three incompatible shapes — DB columns (`users.memory_text`, `users.soul`), disk files (`$PLEXUS_SKILLS_DIR/{user_id}/`), and ephemeral uploads (`/tmp/plexus-uploads/{user_id}/` with 24 h cleanup). None of these is a good host for dream's "read history, edit memory, create skills" workflow.

This spec fixes the foundation first, then builds dream and heartbeat on top.

**What changes at the top level:**

1. **One per-user server workspace.** A tree at `{WORKSPACE_ROOT}/{user_id}/` holds memory, soul, heartbeat tasks, skills, uploads — everything the agent reads or writes server-side.
2. **A unified server toolset.** 11 server tools, all scoped to the user's workspace via path canonicalization. `read_file`, `write_file`, `edit_file`, `delete_file`, `list_dir`, `glob`, `grep`, `file_transfer`, `web_fetch`, `message`, `cron`.
3. **Disk-as-truth for skills and memory.** The `skills` DB table and the `users.memory_text` / `users.soul` columns go away. Context is built by reading files.
4. **A frontend Workspace page** that lets users browse, edit, upload, and delete their own files. Quota is visible and user-managed.
5. **Dream as a protected cron job** (reuses the existing cron infrastructure with a new `kind = 'system'` flag; idle-check at fire time keeps token cost bounded). **Heartbeat as an in-process tick loop** with a virtual-tool decision phase. Both dispatch `EventKind::Dream` / `EventKind::Heartbeat` into the agent loop with dedicated prompt modes.
6. **A shared post-run evaluator** used by cron AND heartbeat. Default-silence gate prevents 4 AM pings; also fixes current cron's unconditional-delivery behavior.

---

## 2. Goals & Non-Goals

**Goals**

- Give the agent one coherent place to read and write server-side state: the workspace.
- Eliminate the 24 h file-store TTL; inbound media persists as long as the user's quota allows.
- Let dream consolidate conversation history into memory and create/edit/delete skills.
- Let heartbeat check `HEARTBEAT.md` periodically, run ripe tasks, and ping the user via external channels only when the output is worth an interruption.
- Give users full visibility and control over their workspace via the frontend.
- Maintain per-user isolation as a hard invariant — no cross-user file access, no path traversal, ever.

**Non-Goals**

- Server-side code execution (shell tool, bwrap jails). Reserved for a later phase when/if the design calls for it. Pure-I/O tools do not need bwrap.
- Shared skills across users, marketplace-style skill distribution, cross-user memory.
- Push notifications to offline browsers (separate spec, deferred).
- Migrating existing deployments' data. Plexus is rebuilding from scratch (see CLAUDE.md); there is no installed user base to migrate.
- Replacing the DB as sole store for *transactional* data (users, sessions, messages, cron jobs). Only memory/soul/skills/uploads move to files.
- Opt-out/disable toggles for dream and heartbeat per user. Admin can set the global heartbeat interval to 0 to disable; dream runs on idle detection and can be globally disabled via a boolean in `system_config`. Per-user overrides are out of scope.

---

## 3. Part 1 — Per-User Workspace

### 3.1 Layout

Every user gets a tree rooted at `{WORKSPACE_ROOT}/{user_id}/`:

```
{WORKSPACE_ROOT}/
└── {user_id}/
    ├── SOUL.md              ← identity/personality, editable by user + dream
    ├── MEMORY.md            ← long-term memory, edited by agent via edit_file
    ├── HEARTBEAT.md         ← user-authored task list + agent-maintained notes
    ├── skills/
    │   ├── create_skill/
    │   │   └── SKILL.md     ← on-demand, seeded from templates at registration
    │   └── {other-user-skills}/
    │       └── SKILL.md
    └── uploads/             ← inbound-media files from channels + user uploads
```

`WORKSPACE_ROOT` is an env var, admin-provisioned. Typical production path: `/var/lib/plexus/workspace/`. Admin is responsible for disk sizing, mount options, and backup of this tree.

### 3.2 Path validation (the sandbox)

Every server tool and every HTTP endpoint that touches a path runs:

```rust
fn resolve_user_path(workspace_root: &Path, user_id: &str, rel: &str)
    -> Result<PathBuf, FsError>
{
    let user_root = workspace_root.join(user_id);
    let joined = user_root.join(rel);
    let canonical = tokio::fs::canonicalize(&joined).await?; // follows symlinks
    if !canonical.starts_with(&user_root.canonicalize()?) {
        return Err(FsError::Traversal);
    }
    Ok(canonical)
}
```

`canonicalize` resolves symlinks, so a symlink pointing outside the user root fails the prefix check.

For **creates** (write_file, file_transfer inbound, upload) the target doesn't exist yet — canonicalize the *parent* directory and validate that, then resolve the filename relative to it.

No bubblewrap, no clone_newns, no chroot. Pure Rust path validation is sufficient for read/write/edit/delete primitives — bwrap is only needed when executing untrusted code, which this design does not do.

### 3.3 Quota

- Default 5 GB per user. Admin-configurable via `system_config` key `workspace_quota_bytes`.
- **Per-upload hard cap: 80% of total quota** (4 GB on a 5 GB default). Files larger than this are rejected at the edge — gateway upload endpoint, Discord/Telegram channel adapters, `file_transfer` inbound to server.
- **Workspace soft-lock.** Users are allowed to briefly exceed quota. The rule:
  - Uploads and writes are permitted if `upload_size ≤ 80% * quota` — even if the resulting usage exceeds 100%.
  - After any mutation, if `usage > 100% * quota`, the workspace enters **soft-lock mode**: all future writes, edits, uploads, and inbound `file_transfer`s are rejected until usage drops back below 100%. Only `delete_file` and `DELETE /api/workspace/file` operations are permitted.
  - Rationale: lets the agent finish an in-flight task before the user is forced to clean up, rather than hard-blocking mid-operation.
  - Worst case (single user): consecutive uploads each just under 80% cap can push usage to ~1.8× quota before soft-lock catches. With concurrent uploads this climbs further. Admin provisions the workspace filesystem with ~2× sum-of-quotas headroom.
- **Implementation:** maintain `DashMap<user_id, AtomicU64>` usage cache; initialize at server boot by walking each user root, update on every mutating tool. Soft-lock state derives from the cache — no separate flag.
- **Error surface:** over-cap uploads return a clear tool-error output so the agent can relay to the user; HTTP endpoints return `413 Payload Too Large`. Soft-locked writes return a specific error message referencing deletion as the resolution.
- **Frontend UX:** quota bar turns red at ≥100%; a banner explains "Workspace full (102%). Delete files to resume writes."

### 3.4 Deployment hygiene

- Run `plexus-server` as dedicated OS user `plexus:plexus` with write access only to `{WORKSPACE_ROOT}` and its own config. If a future bug ever tried to write `/etc/passwd`, permissions block it.
- File mode `0600` on `write_file`, directory mode `0700`. No exec bit ever set.
- Optional `noexec,nosuid` mount on the workspace filesystem. Belt-and-suspenders — harmless under the no-server-shell design, critical if a future shell tool lands.

### 3.5 Account deletion impact

`wipe_file_store(user_id)` in the account-deletion service becomes `fs::remove_dir_all({WORKSPACE_ROOT}/{user_id}/)` plus a `state.quota.forget_user(user_id)` call to drop the in-memory quota counter. Same semantics, different path. The plan document (`plans/2026-04-16-account-deletion.md`) is updated accordingly; the helper is renamed `wipe_workspace` and calls `QuotaCache::forget_user` after removing the directory.

---

## 4. Part 2 — Server Toolset (11 tools)

### 4.1 Tools

All file tools scope paths to `{WORKSPACE_ROOT}/{user_id}/**`. Paths in tool arguments are **relative** to the user's workspace root.

| # | Tool | Arguments | Summary |
|---|---|---|---|
| 1 | `read_file` | `path: string` | Read a file from the user workspace. Returns text (or base64 for binary detected via content-type sniff). |
| 2 | `write_file` | `path: string, content: string` | Write or overwrite a file. Creates parent dirs. Quota-checked. |
| 3 | `edit_file` | `path: string, old_string: string, new_string: string` | Unique-match surgical edit. Errors if `old_string` appears 0 or >1 times with a message suggesting more context. |
| 4 | `delete_file` | `path: string, recursive?: bool` | Delete a file. For directories, `recursive: true` required. |
| 5 | `list_dir` | `path: string` | List entries. Returns `[{name, is_dir, size_bytes}]`. |
| 6 | `glob` | `pattern: string` | Glob over the workspace (via `globset` crate). Pattern is relative to workspace root. |
| 7 | `grep` | `pattern: string, path_prefix?: string, regex?: bool` | Content search. Output capped at 200 lines. `regex: true` treats pattern as a regex. |
| 8 | `file_transfer` | `from_device: string, to_device: string, file_path: string` | Move bytes between devices and/or the server workspace. `from_device="server"` / `to_device="server"` resolve against the workspace. |
| 9 | `web_fetch` | `url: string` | SSRF-protected URL fetch with readability extraction. Unchanged. |
| 10 | `message` | `content: string, channel: string, chat_id: string, media?: string[], from_device?: string` | Reply/notify on a channel. `from_device="server"` attaches workspace files; `media` paths resolve per that device. |
| 11 | `cron` | `action: "add"|"list"|"remove", ...` | Schedule/list/remove cron jobs. Unchanged. |

### 4.2 Removed tools

Removed from the server tool registry entirely:
- `save_memory` → replaced by `write_file("MEMORY.md", ...)` or `edit_file("MEMORY.md", ...)`.
- `edit_memory` → same.
- `read_skill` → replaced by `read_file("skills/{name}/SKILL.md")`.
- `install_skill` → replaced by `web_fetch(url)` + `write_file("skills/{name}/SKILL.md", content)`.

### 4.3 Tool allowlisting

The tool dispatcher gains an allowlist parameter:

```rust
pub enum ToolAllowlist {
    All,
    Only(&'static [&'static str]),
}
```

Normal user turns and heartbeat Phase 2 use `All`. Dream Phase 2 uses `Only(&["read_file","write_file","edit_file","delete_file","list_dir","glob","grep"])`. A disallowed tool call returns a structured error; the LLM recovers on the next iteration.

### 4.4 Tool schemas

`server_tools/mod.rs::tool_schemas()` rebuilt to emit the 11 schemas. Removed entries deleted. Kept tools (`file_transfer`, `web_fetch`, `message`, `cron`) get documentation updates noting the `"server"` device support where applicable.

---

## 5. Part 3 — Skills: Disk as Truth

### 5.1 Storage

Skills live at `{WORKSPACE_ROOT}/{user_id}/skills/{name}/SKILL.md`. A skill is a directory; the directory name *is* the skill name. Additional files (helpers, scripts, resources) can live alongside `SKILL.md` within the same directory — the existing `read_skill` convention about "additional files" carries over to the `read_file` flow.

### 5.2 Frontmatter

Every `SKILL.md` begins with YAML frontmatter:

```markdown
---
name: git-workflow
description: Standard git flow for feature branches
always_on: false
---

# Git Workflow

...instructions...
```

- `name` — must match the directory name (validated on parse; mismatch is a warning, not a hard error).
- `description` — one-line summary shown in the "Available Skills" index.
- `always_on` — boolean. `true` means the full SKILL.md content is injected into the system prompt on every agent turn for this user. `false` means only `name: description` is indexed; the agent calls `read_file` to load the full content when needed.

### 5.3 Context build

At context build time for user `U`:

1. `glob("{U_root}/skills/*/SKILL.md")`.
2. For each file, parse frontmatter. If invalid, skip with a `warn!` log.
3. Partition into always-on and on-demand.
4. Always-on: concatenate full file contents under `## Always-On Skills` in the system prompt.
5. On-demand: emit a `## Available Skills` index with `- {name}: {description}` lines and an instruction like "Use `read_file` to open the full skill when you need one."

A per-user cache (`DashMap<user_id, SkillsBundle>`) holds the parsed result. It is invalidated when any `write_file`/`edit_file`/`delete_file`/`file_transfer`-to-server tool touches a path under `skills/` for that user. Cold rebuild walks ≤ O(skills) files, typically under 100 per user.

### 5.4 DB table removed

The `skills` table is dropped. Frontmatter on disk is the source of truth. This removes:
- `db/skills.rs` module.
- All skill-related admin endpoints that queried the table. Replaced by a `GET /api/workspace/skills` endpoint that calls the same cache.

### 5.5 Default skill

Ship `server/templates/skills/create_skill/SKILL.md` with the release. On-demand (`always_on: false`). Content teaches the agent: skill directory layout, frontmatter format, naming conventions, `always_on` guidance, when to create a skill (reusable pattern worth preserving) vs not (one-off task).

On user registration, the server recursively copies `server/templates/skills/` into `{user_root}/skills/`. User owns the copy — can edit, delete, leave as-is.

---

## 6. Part 4 — Workspace Templates

### 6.1 Template files shipped with the server

```
server/templates/
├── workspace/
│   ├── SOUL.md         ← baseline soul content (minimal personality scaffold)
│   ├── MEMORY.md       ← baseline with section headers (## User Facts, etc.)
│   └── HEARTBEAT.md    ← baseline with instructions + example task format
└── skills/
    └── create_skill/
        └── SKILL.md
```

### 6.2 Admin defaults in `system_config`

Three new `system_config` keys:

- `default_soul` (already exists — expanded from its current role)
- `default_memory` (new)
- `default_heartbeat` (new)

At server boot: for each key, if missing, seed the value from the corresponding `server/templates/workspace/*.md` file. Idempotent — admin edits persist across boots.

### 6.3 Registration flow

When a user registers (`auth/register`):

1. Create `{WORKSPACE_ROOT}/{user_id}/` with mode `0700`.
2. Read `default_soul`, `default_memory`, `default_heartbeat` from `system_config`. Write each to `{user_root}/SOUL.md`, `MEMORY.md`, `HEARTBEAT.md` respectively (mode 0600).
3. Recursively copy `server/templates/skills/` → `{user_root}/skills/`.
4. Initialize quota cache entry.

All operations are idempotent — registration can be retried if a step fails mid-way without corrupting state.

### 6.4 Admin UI — Workspace Defaults tab

Admin UI gains a "Workspace Defaults" section (or tab) with three markdown editors:

- Default Soul (replaces the existing "Default Soul" editor)
- Default Memory
- Default Heartbeat

Each saves to the corresponding `system_config` key. Changes apply to *new* registrations only — existing users' own workspace files are never touched by admin-default changes.

---

## 7. Part 5 — Frontend Workspace Page

### 7.1 Route and purpose

New frontend route: `/settings/workspace`. A file-manager UI for the user's own `{WORKSPACE_ROOT}/{user_id}/` tree.

### 7.2 UI layout

**Top bar.** Quota progress bar: `{used} / {total}`. Amber at 80%, red at 95%. Subtext: "Delete files below to free space."

**Left pane (~25% width).** Recursive tree view. Folders are collapsible. File entries show icon + name + size. Click selects.

**Right pane (~75% width).** Depends on selected file type:
- Markdown (`*.md`) → rendered view by default with an "Edit" button that flips to a textarea editor. Save writes via `PUT /api/workspace/file`.
- Plain text, JSON, code, YAML, etc. → text editor directly.
- Image (png/jpg/gif/webp) → inline preview. Download button.
- Other binary → metadata (size, modified date) + Download button. No preview.

**Actions per file (context menu or buttons).** Rename, delete (with confirm modal), download.

**Upload.** Top-right button: click-to-select + drag-and-drop anywhere on the page. Multiple files supported. Shows per-file upload progress. Routes to `POST /api/workspace/upload`.

**Quick-access sidebar section.** At the top of the tree: "📄 Soul", "📄 Memory", "📄 Heartbeat Tasks" — one-click jumps to those files. Everything is the same file editor underneath.

### 7.3 API endpoints

All endpoints require the user JWT. All paths are validated via the same `resolve_user_path` helper the server tools use — single source of truth for the sandbox invariant.

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/workspace/tree` | Full tree `[{path, is_dir, size_bytes, modified_at}]`. Cached per-user; invalidated on any write. |
| GET | `/api/workspace/file?path=...` | Streams bytes. Content-Type inferred from extension + sniff. |
| PUT | `/api/workspace/file?path=...` | Body is raw content. Creates parent dirs. Quota-checked. |
| DELETE | `/api/workspace/file?path=...&recursive=bool` | Delete file or directory. |
| POST | `/api/workspace/upload` | Multipart; one or more files. Quota-checked. |
| GET | `/api/workspace/quota` | `{used_bytes, total_bytes}` |
| GET | `/api/workspace/skills` | Convenience: parsed frontmatter for every skill. Used by Settings → Skills list (renders from disk, not from DB). |

### 7.4 Consolidation of existing Settings

The existing Settings → Soul and Settings → Memory sections are **removed**. Their functionality moves into the Workspace page as quick-access links to `SOUL.md` and `MEMORY.md`. One file-editor component, one mental model.

The Settings → Skills list stays, but re-reads from `/api/workspace/skills` (frontmatter scan) instead of the deleted DB table.

### 7.5 Inbound-media integration

Channel adapters (Discord/Telegram/Gateway) that currently write to `/tmp/plexus-uploads/{user_id}/` will write to `{user_root}/uploads/{YYYY-MM-DD}-{hash}-{filename}` instead. The 24 h cleanup task is removed. Files persist for the life of the user's workspace unless they delete them.

The base64-in-DB durability stashing (ADR-30) becomes optional — with persistent uploads the DB fallback is only belt-and-suspenders against disk loss. Retained for M2 safety; can be revisited later.

---

## 8. Part 6 — Dream Subsystem

### 8.1 Purpose

Periodically consolidate each active user's conversation history into durable long-term memory (`MEMORY.md`, `SOUL.md`) and auto-discover reusable task patterns as new skills. Mirrors `nanobot.agent.memory.Dream` with adaptations for Plexus's multi-user topology.

### 8.2 Architecture: dream is a protected cron job

**Dream reuses the existing cron infrastructure.** It is not a separate tick loop. At user registration, the server creates a protected cron job:

```
{
  name: "dream",
  cron_expr: "0 */2 * * *",          # every 2h
  kind: "system",                    # NEW — protected, user cannot delete
  timezone: <user.timezone>,
  message: "",                       # handler-specific, unused by the normal cron path
  delete_after_run: false,
  enabled: true,
}
```

`kind: "system"` is a new `cron_jobs` column:

```sql
ALTER TABLE cron_jobs
  ADD COLUMN kind TEXT NOT NULL DEFAULT 'user'
  CHECK (kind IN ('user', 'system'));
```

- `user` jobs: behave exactly like today's cron jobs. Agent-creatable via the `cron` tool, user-deletable via API.
- `system` jobs: server-created, user-visible but not user-modifiable. The `cron` tool's `remove` action and the `DELETE /api/cron/{id}` endpoint reject system jobs with a clear error.

System jobs use the same claim-dispatch-reschedule pipeline (ADR-27). The existing cron tick loop finds them, dispatches, and reschedules after execution.

### 8.3 Dispatch: idle check at fire time

When the dream cron job fires, the handler runs a cheap idle check **before** spending any LLM budget:

```rust
async fn handle_dream_fire(user_id: &str) -> Result<()> {
    // 1. Cheap SQL: is there any activity since last dream?
    let recent_activity: Option<DateTime<Utc>> = query_last_non_autonomous_activity(user_id).await?;
    let last_dream = query_last_dream_at(user_id).await?;

    match (recent_activity, last_dream) {
        (Some(activity), Some(dreamt)) if activity <= dreamt => {
            // Nothing new since last dream. Skip silently.
            return Ok(());
        }
        (None, _) => {
            // User has no activity at all. Skip.
            return Ok(());
        }
        _ => {
            // We have unprocessed activity. Proceed.
        }
    }

    // 2. Set last_dream_at now so concurrent dispatches can't double-fire.
    update_last_dream_at(user_id, Utc::now()).await?;

    // 3. Publish InboundEvent — the agent loop does the actual consolidation.
    bus.publish_inbound(InboundEvent {
        kind: EventKind::Dream,
        session_id: format!("dream:{user_id}"),
        user_id: user_id.into(),
        content: include_str!("templates/prompts/dream_phase1.md").into(),
        ..
    }).await?;

    Ok(())
}
```

The idle check is a single indexed query — fires every 2 h even for fully-idle users, but the cost is negligible (zero LLM calls on skip).

`last_non_autonomous_activity` = most recent message across all sessions of the user excluding `session_id LIKE 'dream:%'` and `session_id LIKE 'heartbeat:%'`.

Global kill switch: `system_config.dream_enabled` (default `true`). If `false`, the dream handler early-returns before the idle check.

### 8.4 Data model

```sql
ALTER TABLE users ADD COLUMN last_dream_at TIMESTAMPTZ;
ALTER TABLE cron_jobs ADD COLUMN kind TEXT NOT NULL DEFAULT 'user';
```

No cursor table — `last_dream_at` is the cursor. Messages with `created_at > last_dream_at` are the unprocessed window.

### 8.5 Phase 1 — Analysis

- Session key `dream:{user_id}` — does not surface in the user's session list.
- System prompt: `server/templates/prompts/dream_phase1.md` (ported from `nanobot/templates/agent/dream_phase1.md`, adapted for Plexus file paths).
- User-message content built by `context.rs` in `PromptMode::Dream`:
  - Current `MEMORY.md` content (empty string if file is missing — user may have deleted it; dream writes it back in Phase 2).
  - Current `SOUL.md` content (same treatment — empty if missing).
  - Existing skills index (name + description).
  - The unprocessed message slice (joined, bounded — if > N messages, chunked in multiple dream sessions).
- LLM call with **no tools** — pure text output.
- Output: structured directives. Nanobot's format is:
  - `[FILE] path=MEMORY.md op=edit old=... new=...`
  - `[FILE-REMOVE] path=MEMORY.md match=...`
  - `[SKILL] name=... description=... always_on=false content=...`
- If directives are empty: done, no Phase 2. `last_dream_at` already updated.

### 8.6 Phase 2 — Execution

- Reuse the agent-loop infrastructure with:
  - System prompt: `server/templates/prompts/dream_phase2.md` + the directives from Phase 1 inlined.
  - `PromptMode::Dream` on `context.rs::build_context`.
  - `ToolAllowlist::Only(&["read_file","write_file","edit_file","delete_file","list_dir","glob","grep"])`.
  - Max iterations: 30 (vs. 200 for normal turns).
- The LLM reads the directives and executes them using the file tools. Edits to `MEMORY.md`/`SOUL.md`, creates/edits/deletes skill files. Anything that fails (e.g., `edit_file` unique-match miss) is a recoverable tool error; the LLM retries with more context.
- On completion: mark the processed messages as `compressed = TRUE` (same flag already used by context compression). `last_dream_at` is already set.

### 8.7 Failure handling

`last_dream_at` is advanced *before* Phase 1 runs, not after. A partial failure doesn't re-process the same window. Trade-off: a complete Phase 1/Phase 2 failure means that batch of conversation is never consolidated, but the alternative (retrying indefinitely) is worse — a poisoned batch would block all future dreams.

Phase 2 errors log at `warn!` but do not retry or alert — dream is best-effort. Monitor via logs; a noisy failure pattern signals a prompt bug to investigate.

### 8.8 Session retention

Dream sessions (rows in `sessions` with `session_id LIKE 'dream:%'`) are retained for debugging and admin inspection. They are excluded from the user-facing session list by the existing prefix-filter logic.

---

## 9. Part 7 — Heartbeat Subsystem

### 9.1 Purpose

Every N minutes per user, read `HEARTBEAT.md`, have the LLM decide whether any task is ripe, run it through the agent loop if so, and notify the user on an external channel only if the output is worth the interruption.

### 9.2 Trigger

In-process tick loop — not cron. At server boot and every 60 seconds thereafter, for each user where `NOW() - last_heartbeat_at >= {system_config.heartbeat_interval_seconds}` (default 1800, admin-editable, no per-user override):

1. Set `last_heartbeat_at = NOW()` immediately (prevents refire during Phase 1/2).
2. Skip if the user has no `HEARTBEAT.md` file (user may have deleted the registration template).
3. Skip if the user's heartbeat session already has a turn in flight (check inbox queue depth).
4. Run **Phase 1 directly in the tick task** (below). If Phase 1 returns `skip`, log and done.
5. If Phase 1 returns `run`, publish `InboundEvent { kind: Heartbeat, session_key: "heartbeat:{user_id}", user_content: <tasks string> }` to the bus for Phase 2.

Global kill switch: set `heartbeat_interval_seconds = 0` → tick loop no-ops.

Phase 1 runs outside the agent-loop/session infrastructure (it's a single stateless LLM call that decides whether to spend the heavier Phase 2 budget). Phase 2 runs through the normal per-session agent loop, reusing context/tooling/compression/crash-recovery.

### 9.3 Data model

```sql
ALTER TABLE users
  ADD COLUMN last_heartbeat_at TIMESTAMPTZ,
  ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';
```

User timezone is used by the Phase 1 prompt (real local time, so the LLM can reason about "is now a good time to run this?") and by the evaluator.

### 9.4 Phase 1 — Decision

Inputs:

- Hardcoded short system prompt: "You are a heartbeat agent. Read the task list below and call the `heartbeat` tool with `action: \"skip\"` or `action: \"run\"`. If running, list which tasks should run now as a free-text summary."
- User message: the full `HEARTBEAT.md` content, prefixed with "Current time (user's timezone, {tz}): {local_time}".
- Tools: a single virtual tool `heartbeat(action: "skip"|"run", tasks: string)` — not persisted in the DB schema, injected only in this mode.

Output: the LLM must call the tool. `skip` ends the run; `run` proceeds with the `tasks` string.

### 9.5 Phase 2 — Execution

Reuse the agent-loop infrastructure:

- System prompt: `PromptMode::Heartbeat` — similar to a normal user turn but notes that this is an autonomous wake-up, so "do not ask the user clarifying questions; make reasonable defaults and proceed."
- Tool allowlist: `ToolAllowlist::All`.
- User message: the `tasks` string from Phase 1.
- Max iterations: 200 (same as a normal user turn).

### 9.6 Evaluator (the 4 AM guard) — shared with cron

After Phase 2 completes, before any notification goes out, the **shared evaluator** runs:

- Module: `server/src/evaluator.rs`. **Reused by both heartbeat and cron** — same code, same prompt shape, different context injected.
- Small LLM call (cheap model, short prompt). Uses a virtual tool `evaluate_notification(should_notify: bool, reason: string)` — injected for this one call, not registered in the tool registry (pattern lifted from nanobot's heartbeat virtual-tool).
- Inputs: the final assistant message, the user's current local time, a short description of "what this autonomous run was asked to do."
- Output via virtual tool call: `{should_notify: bool, reason: string}`.
- Prompt guidance: "Return `should_notify: false` if the output is merely a status update, no user action needed, or if the local time is outside typical waking hours (10 PM — 8 AM). Return `should_notify: true` only if the user would genuinely benefit from seeing this now."
- **Default-on-error: silence.** If the evaluator call fails (network error, malformed response), fall back to `should_notify: false`. Explicit in the implementation — nanobot's opposite default (notify-on-error) was observed to cause spam in edge cases.

**Cron integration (bonus benefit of this refactor):** the same evaluator replaces the current unconditional-delivery behavior of cron jobs. A cron job firing now runs Phase 2 (the scheduled message processed by the agent loop), then hands the final assistant message to the evaluator. This is **new behavior for cron** — today cron spam is unconditional; after this, cron respects the same 4-AM guard as heartbeat.

The existing cron tool adds an optional `deliver: bool` field (default `true`) on the job. Setting `deliver: false` skips the evaluator entirely (for purely-file-writing cron jobs that should never ping the user).

If `should_notify: false`: log + discard. Content is still stored in the session for audit/recovery.

### 9.7 Delivery

Heartbeat Phase 2 agent behavior:
- The agent produces a final assistant message as its last action (normal agent loop termination — no tool call on the final step). It does **not** use the `message` tool to deliver — delivery is owned by the evaluator path, not the agent.
- The agent loop detects `ctx.kind == Heartbeat` on final assistant message and invokes the evaluator instead of terminating silently.
- If evaluator returns `notify: true`: the agent-loop post-hook emits `OutboundEvent` directly via channel adapters. Priority: Discord → Telegram → (nothing). **Never the gateway** — heartbeat notifications must not interrupt an active browser session.
- If `notify: false` or no external channel is configured: the final message is stored in the heartbeat session (retained per §9.8) and logged. Future feature: a "Heartbeat Log" page in the frontend that lists past heartbeat outputs.

### 9.8 Retention

Heartbeat sessions (`session_id LIKE 'heartbeat:%'`) are retained. Excluded from the user-facing session list by the prefix filter. Heartbeat messages excluded from dream's input window (section 8.2 SQL) so dream doesn't re-ingest its own ecosystem.

---

## 10. Part 8 — Schema Migration Summary

**New columns:**

```sql
ALTER TABLE users ADD COLUMN last_dream_at TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN last_heartbeat_at TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';

ALTER TABLE cron_jobs ADD COLUMN kind TEXT NOT NULL DEFAULT 'user'
  CHECK (kind IN ('user', 'system'));
-- Note: cron_jobs.deliver already exists; behavior changes from "pass-through to channel"
-- to "skip evaluator, skip delivery entirely" when false.
```

**Dropped columns:**

```sql
ALTER TABLE users DROP COLUMN memory_text;
ALTER TABLE users DROP COLUMN soul;
```

(Both deleted — replaced by workspace files. Since Plexus is rebuilding from scratch, no data migration needed. The per-column DROP statements appear in the initial schema file so fresh installs simply never create them.)

**Dropped tables:**

```sql
DROP TABLE skills;
```

**New `system_config` keys:**
- `default_memory` — seeded from `server/templates/workspace/MEMORY.md` at boot.
- `default_heartbeat` — seeded from `server/templates/workspace/HEARTBEAT.md` at boot.
- `heartbeat_interval_seconds` — integer seconds, default 1800.
- `workspace_quota_bytes` — integer bytes, default `5 * 1024 * 1024 * 1024`.
- `gateway_upload_max_bytes` — per-upload sanity cap for browser uploads, default `1 * 1024 * 1024 * 1024`.
- `dream_enabled` — boolean, default `true`.
- `dream_phase1_prompt`, `dream_phase2_prompt`, `heartbeat_phase1_prompt` — optional admin overrides. Unset → built-in `include_str!` templates are used.

**New invariant:** Every future per-user table or column must CASCADE from `users(user_id)`. Confirmed in the account-deletion plan (`plans/2026-04-16-account-deletion.md`, task AD-2).

---

## 11. Part 9 — Cross-Cutting Infrastructure

### 11.1 `InboundEvent.kind`

The bus discriminates events by a new enum instead of a `cron_job_id: Option` marker:

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EventKind {
    UserTurn,
    Cron,
    Dream,
    Heartbeat,
}

pub struct InboundEvent {
    pub session_id: String,
    pub user_id: String,
    pub kind: EventKind,
    pub cron_job_id: Option<String>,   // retained — cron rescheduling needs it
    // existing fields unchanged
}
```

### 11.2 `PromptMode`

`context::build_context` takes a mode parameter:

```rust
pub enum PromptMode {
    UserTurn,
    Dream,
    Heartbeat,
}

pub async fn build_context(
    state: &Arc<AppState>,
    user_id: &str,
    session_id: &str,
    mode: PromptMode,
) -> Result<Vec<ChatMessage>>;
```

- `UserTurn`: full context (identity, channels, skills always-on + index, connected devices, current time).
- `Dream`: MEMORY.md, SOUL.md, skills index — no channels, no devices. Phase 1 system prompt is `dream_phase1.md`; Phase 2 is `dream_phase2.md`.
- `Heartbeat`: identity, MEMORY.md, skills index — no channels. Used **only for Phase 2**. Phase 1 runs outside `build_context` (short hardcoded prompt, see §9.4) so it never passes through `PromptMode`.

Call-sites in `agent_loop.rs` select the mode from `InboundEvent.kind`.

### 11.3 Tick loops and graceful shutdown

One new long-lived task in `AppState` (dream reuses the existing cron tick loop):
- `heartbeat_tick` — 60-second interval, iterates users, runs Phase-1 virtual-tool decision, dispatches Phase-2 events.

The existing cron tick loop gains awareness of `kind = 'system'` jobs (protect against user deletion; pass the job to the `system_job_handler` router instead of publishing a normal cron InboundEvent when `name == "dream"`).

Both participate in the ADR-34 cancellation fan-out: `tokio::select!` on `shutdown.cancelled()`. On shutdown, they stop dispatching new events; any events already published drain through the normal session path.

### 11.4 Shared evaluator

New module `server/src/evaluator.rs`:

```rust
pub async fn evaluate_notification(
    state: &Arc<AppState>,
    user_id: &str,
    final_message: &str,
    purpose: &str,     // "cron job 'daily-report'" or "heartbeat wake-up"
) -> EvaluationResult;

pub struct EvaluationResult {
    pub should_notify: bool,
    pub reason: String,
}
```

- Single LLM call, virtual `evaluate_notification` tool, user timezone injected, default-silence on error.
- Used by: heartbeat Phase 2 post-hook, cron Phase 2 post-hook (when job's `deliver: true`).
- Heartbeat calls it unconditionally; cron skips the call if `job.deliver == false`.

### 11.5 Prompt templates location

System-owned prompt templates ship in the server binary via `include_str!`:

```
server/src/templates/prompts/
├── dream_phase1.md     ← analysis prompt
├── dream_phase2.md     ← execution prompt
└── heartbeat_phase1.md ← virtual-tool decision prompt
```

Admin can override any of these globally via `system_config` keys `dream_phase1_prompt`, `dream_phase2_prompt`, `heartbeat_phase1_prompt`. If unset, the built-in `include_str!` content is used. Admin-override is intentionally *global*, not per-user — these are system behaviors, not user customization.

---

## 12. Part 10 — ADRs to Draft

These are drafted as part of implementation and added to `plexus-server/docs/DECISIONS.md`:

- **ADR-35** — Per-user server workspace with path-validation sandbox (no bwrap for pure I/O).
- **ADR-36** — File-based memory/soul/heartbeat; `users.memory_text` and `users.soul` columns removed.
- **ADR-37** — Skills as disk-as-truth, frontmatter-driven; `skills` DB table removed.
- **ADR-38** — Workspace templates with admin-configurable defaults via `system_config`.
- **ADR-39** — `EventKind` discriminant on `InboundEvent` and `PromptMode` dispatch in `build_context`.
- **ADR-40** — Dream as a protected system cron job with idle check at fire time (nanobot-style; reuses cron infra, no separate scheduler).
- **ADR-41** — Heartbeat: fixed-interval in-process tick, virtual-tool decision phase, external-channels-only delivery.
- **ADR-42** — Shared post-run evaluator for cron + heartbeat; default-silence on error.
- **ADR-43** — Workspace soft-lock: users may briefly exceed quota; only deletes permitted until usage drops below 100%.

---

## 13. Part 11 — Execution Order

1. **Workspace foundation.** Tools (11), path validation, quota + soft-lock, templates, registration wiring. Kill `save_memory`/`edit_memory`/`read_skill`/`install_skill`. Migrate inbound-media paths. Drop `users.memory_text`/`users.soul`/`skills` table. Ship default `create_skill` skill. This is the largest single chunk — ~the scale of the inbound-media work.
2. **Frontend Workspace page.** API endpoints + React page. Remove Settings → Soul and Settings → Memory editors; quick-links into Workspace replace them. Update Settings → Skills to read from the new `/api/workspace/skills` endpoint.
3. **Shared evaluator + cron integration.** New `evaluator.rs` module. Wire cron post-run through it (gated by `cron_jobs.deliver`). Unblocks both dream and heartbeat. Fixes existing cron-spam behavior as a side effect.
4. **Dream.** Add `cron_jobs.kind` column + system-job protection. Register "dream" system cron job per user at registration. Add `last_dream_at` column. Dream handler with idle check + EventKind::Dream dispatch. `PromptMode::Dream` + tool allowlist + Phase 1 + Phase 2 prompt templates.
5. **Heartbeat.** `last_heartbeat_at` + `timezone` columns. Heartbeat tick loop with Phase 1 virtual tool. Phase 2 via normal agent loop with `PromptMode::Heartbeat`. Evaluator reused from step 3.
6. **Close M2 deferred backlog** (items C–F of `2026-04-17-m2-closeout-design.md`): account deletion (includes the new CASCADE columns), admin user-management UI, graceful shutdown for bots + per-session loops, unread badge.

Items 1 and 2 are coupled — the frontend has to land alongside the backend changes because the Settings → Soul/Memory editors break otherwise. Items 3/4/5 share `EventKind`/`PromptMode`/evaluator plumbing; the evaluator lands first so dream and heartbeat can consume it.

---

## 14. Part 12 — Success Criteria

- A user can upload a file via Discord, go offline, come back the next day, and still find it in their workspace.
- A user can browse, edit, and delete files in their workspace through the frontend. Quota usage is visible at all times.
- Dream fires once per 2-hour idle window per active user. After dream runs, `MEMORY.md` reflects consolidated facts from the recent conversations; dream can create, edit, and delete skills.
- Heartbeat fires every 30 minutes per user. At 3 AM local time, a status-only heartbeat output does *not* trigger a Discord ping. A user-action-required heartbeat output at 10 AM *does*.
- Agent cannot read or write outside `{WORKSPACE_ROOT}/{user_id}/` — every path-traversal attempt (raw, encoded, symlink) is rejected with a clear tool error.
- Every `ISSUE.md` across the four crates has no `## Open` items after this spec + the M2 closeout plans are executed. `## Deferred` items either close or carry a documented "stays deferred" justification.
- `SIGTERM` drains cleanly; tick loops stop dispatching new events; in-flight turns finish within the HTTP grace window.

---

## 15. Revision Log

- **2026-04-17 (draft 1)** — Initial draft. Finalized per-user workspace foundation, 11-tool server toolset, disk-as-truth skills, workspace templates, frontend Workspace page, idle-triggered dream (separate tick loop), fixed-interval heartbeat with evaluator. Brainstorm resolved open questions A1/A2/A3/B1/B2 from the M2 closeout spec; Parts A and B of that doc are now superseded by this spec.
- **2026-04-17 (draft 2)** — Deep nanobot architecture review drove three simplifications:
  - Dream is now a **protected system cron job** (not a separate tick loop) with an idle-check at fire time. Introduces `cron_jobs.kind TEXT` column and system-job protection.
  - **Shared evaluator** for cron + heartbeat (previously only heartbeat). Fixes cron's current unconditional-delivery behavior as a side effect.
  - **Prompt templates** for dream/heartbeat ship in-binary via `include_str!`, with admin-global overrides via `system_config` — not per-user skills.
  - Workspace quota rule refined: soft-lock at 100% rather than rejecting at 80%; users can briefly exceed before being forced to clean up.
  - New ADRs: ADR-42 (shared evaluator), ADR-43 (soft-lock). ADR-40 reframed from "idle-triggered dream" to "dream as protected cron job".
