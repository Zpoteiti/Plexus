# M2 Autonomy Rewrite — Handoff

**Last updated:** 2026-04-17 — end of the Plan D (dream) execution cycle.
**Branch:** `M3-gateway-frontend` at commit `8f60648`.
**Context:** This document is a mid-milestone handoff. It explains what landed, what the design is, what's next, and where to find the decision trail so another engineer (or a future Claude session) can resume without reading every commit message.

---

## 1. What this effort is

Plexus is an LLM agent server with remote tool execution on user devices. Prior M1/M2 work shipped the core: a ReAct agent loop, cron scheduling, inbound-media, cross-channel addressing, graceful shutdown, and PostgreSQL-backed crash recovery. Two things were still missing to match nanobot's design philosophy:

- **Dream** — periodic memory consolidation and skill discovery.
- **Heartbeat** — periodic task wake-up (the agent checks a user-owned task list every 30 min).

Adding those on top of the existing architecture surfaced a deeper foundation issue: Plexus's per-user state was split across DB columns (`users.memory_text`, `users.soul`), a separate skills directory (`$PLEXUS_SKILLS_DIR/{user_id}`), and ephemeral `/tmp` uploads with a 24 h TTL. Three incompatible shapes made dream's "read conversation history → edit memory → author skills" workflow impossible without a rewrite.

**The outcome of this session:** unify per-user server state under one per-user **workspace tree** (memory + soul + heartbeat list + skills + uploads, all files), ship a coherent 11-tool server toolset for operating on it, and build dream + heartbeat on top of that foundation.

Nanobot inspired the architecture; we are not affiliated. Plexus's docs (specs + plans) are the source of truth going forward.

---

## 2. Status at a glance

| Plan | Scope | Status | Reference |
|---|---|---|---|
| **A** | Per-user server workspace + 11-tool toolset + disk-as-truth skills + templates | ✅ Complete (21 tasks, 35 commits) | `docs/superpowers/plans/2026-04-17-workspace-foundation.md` |
| **C** | Shared post-run evaluator + cron integration + system-cron protection + `ensure_system_cron_job` helper | ✅ Complete (5 tasks, 10 commits) | `docs/superpowers/plans/2026-04-17-shared-evaluator-cron-integration.md` |
| **D** | Dream subsystem (idle-triggered, two-phase, restricted tool allowlist) | ✅ Complete (10 tasks, 11 commits) | `docs/superpowers/plans/2026-04-17-dream-subsystem.md` |
| **E** | Heartbeat subsystem (30-min tick, virtual-tool decision phase, evaluator-gated delivery) | ⬜ Not yet written / not executed | Spec §9 of the autonomy design |
| **B** | Frontend Workspace file manager page + 6 REST endpoints | ⬜ Not yet written / not executed | Spec §7 of the autonomy design |
| **M2 closeout backlog** | Account deletion, admin user-management UI, graceful-shutdown extension, session-list unread badge | ⬜ Queued (account deletion has a plan; others are spec'd) | `docs/superpowers/specs/2026-04-17-m2-closeout-design.md` + `docs/superpowers/plans/2026-04-16-account-deletion.md` |

**Execution order agreed upon:** C → D → E → B → M2 closeout. Rationale: C/D/E are server-side and unblock the user-facing B work. B fixes frontend breakage from Plan A's 410-Gone endpoints. M2 closeout wraps open issues tracked in each crate's `ISSUE.md`.

Build across all crates is clean. **132 passing tests** + **10 `#[ignore]`-gated integration tests** (run with `DATABASE_URL=… cargo test -- --ignored`). No known runtime regressions in the agent loop, cron, inbound-media, or cross-channel addressing paths.

---

## 3. The foundation in plain words

### 3.1 Per-user workspace (Plan A)

Every user now gets a directory at `{PLEXUS_WORKSPACE_ROOT}/{user_id}/`. The agent reads and writes there — and nowhere else on the server. Inside the tree:

- `SOUL.md` — the user-customizable personality/system-prompt prefix.
- `MEMORY.md` — structured long-term memory with stable section headers (`## User Facts`, etc).
- `HEARTBEAT.md` — the user's pending-tasks list.
- `skills/{name}/SKILL.md` — per-user skills with YAML frontmatter. One default skill (`create_skill`) is copied in at registration.
- `uploads/{YYYY-MM-DD}-{hash}-{filename}` — inbound media from Discord / Telegram / Gateway, and anything the agent produces via `write_file` or `file_transfer`.

The old `users.memory_text` / `users.soul` columns, the `skills` DB table, and the `$PLEXUS_SKILLS_DIR` env var are **gone**. Disk is the source of truth for memory, soul, and skills; a `SkillsCache` keyed by `user_id` parses `SKILL.md` frontmatter and invalidates on any write under `skills/`.

**Security posture:** every path the agent touches is resolved via `workspace::resolve_user_path`, which canonicalizes (resolving symlinks) and rejects anything outside `{WORKSPACE_ROOT}/{user_id}/`. No bwrap — bwrap is only needed for executing untrusted code, which Plexus's server explicitly doesn't do. Client devices still sandbox their shell tool with bwrap.

**Quota:** per-user, default 5 GB (admin-configurable via `system_config.workspace_quota_bytes`). Per-upload hard cap = 80% of quota. Users are allowed to briefly exceed 100% (grace window); a soft-lock mode then rejects all writes until deletes bring usage below quota.

### 3.2 The 11-tool server toolset (Plan A)

The agent's server-native tools (executed on Plexus server, not on a client device) are now:

```
read_file  write_file  edit_file  delete_file  list_dir  glob  grep
file_transfer  web_fetch  message  cron
```

All file tools scope paths to `{WORKSPACE_ROOT}/{user_id}/**`. `file_transfer` treats `"server"` as a valid source/destination so the agent can ship workspace files to connected client devices. `message` accepts `from_device="server"` so the agent can attach workspace files to channel replies.

**Deleted in this rewrite:** `save_memory`, `edit_memory`, `read_skill`, `install_skill`. The agent now reads/writes memory via `edit_file("MEMORY.md", …)` and creates skills via `write_file("skills/{name}/SKILL.md", …)` — a much smaller, more composable toolset.

### 3.3 Shared post-run evaluator (Plan C)

Before the dream/heartbeat work could land, we needed a shared notification gate — a small LLM call that decides whether an autonomous agent's final output should actually ping the user. Otherwise cron would spam, and dream/heartbeat would inherit the same anti-pattern.

`plexus-server/src/evaluator.rs::evaluate_notification(state, user_id, final_message, purpose)` is that gate:

- Takes the agent's last assistant message + a purpose label (`"cron job 'daily-standup'"`, `"heartbeat wake-up"`).
- Calls the LLM once with a **virtual** `evaluate_notification(should_notify, reason)` tool (injected inline, not registered in the global tool registry).
- Injects the user's **local time** so the LLM can reason about "it's 4 AM, this can wait."
- Returns `{should_notify: bool, reason: String}`. Silence on every error path — silence is the safe failure mode for notification decisions.

Cron's `publish_final` (introduced in C-2) now gates delivery through the evaluator when `cron_jobs.deliver == true`. A separate `deliver=false` escape hatch skips the evaluator entirely — used by dream's cron job, which never pings channels.

### 3.4 System-cron protection (Plan C)

`cron_jobs.kind TEXT NOT NULL DEFAULT 'user' CHECK (kind IN ('user', 'system'))` — the kind column landed in Plan A's schema phase, and Plan C teaches both the `cron` server tool's `remove` action and the `DELETE /api/cron-jobs/{id}` HTTP endpoint to refuse `kind='system'` jobs. The helper `db::cron::ensure_system_cron_job` is idempotent (partial unique index + `ON CONFLICT DO NOTHING`) so Plan D can call it at registration without worrying about duplicates.

### 3.5 Dream (Plan D)

Dream is a **protected system cron job** (`kind='system'`, `name='dream'`, `cron_expr='0 */2 * * *'`, `deliver=false`) registered per-user at registration. When the cron poller fires a dream job, `dream::handle_dream_fire` runs:

1. **Kill switch.** `system_config.dream_enabled == "false"` → skip.
2. **Idle check.** `users.last_dream_at` vs. `MAX(messages.created_at)` across non-autonomous sessions. No new activity → skip silently (zero LLM cost).
3. **Advance timestamp** *before* running phases. Prevents refire loops; errors don't block future dreams.
4. **Phase 1 — Analysis.** One LLM call with `dream_phase1.md` prompt + `MEMORY.md` + `SOUL.md` + skills index + up to 200 messages since last dream. Output is structured directives: `[MEMORY-ADD]`, `[MEMORY-REMOVE]`, `[SOUL-EDIT]`, `[SKILL-NEW]`, `[SKILL-DELETE]`, or `[NO-OP]`.
5. **Phase 2 — Execution.** If directives non-empty, publish `InboundEvent { kind: EventKind::Dream, session: "dream:{user_id}", content: directives }`. The agent loop routes on `event.kind`:
   - `PromptMode::Dream` → system prompt from `dream_phase2.md` + memory + soul + skills. Channel identity and device list are omitted.
   - `ToolAllowlist::Only(&["read_file", "write_file", "edit_file", "delete_file", "list_dir", "glob", "grep"])` → no `message`, no `cron`, no `web_fetch`, no `file_transfer`. File ops only.

Dream is silent: `deliver=false` makes `publish_final` skip the evaluator entirely. The agent's final message (logged for diagnostics) never reaches a channel.

Prompt templates ship via `include_str!`; admin overrides are stored in `system_config.dream_phase{1,2}_prompt` and load at server boot.

### 3.6 Cross-cutting scaffolding introduced by Plan D

Three enums landed that Plan E will also consume:

- `bus::EventKind { UserTurn | Cron | Dream | Heartbeat }` — the dispatch discriminant on `InboundEvent`. `cron_job_id` is retained but now means "reschedule handle", not "is-cron".
- `context::PromptMode { UserTurn | Dream | Heartbeat }` — threaded through `build_context`. Dream branch is live; Heartbeat still mirrors UserTurn (Plan E finalizes).
- `server_tools::ToolAllowlist { All | Only(&'static [&'static str]) }` — gates tool dispatch in the agent loop. Dream events bind `Only(DREAM_PHASE2_ALLOWLIST)`; all other events bind `All`.

---

## 4. Commits map

All work lives on `M3-gateway-frontend`. Each plan is a contiguous commit range:

| Plan | Range | Count |
|---|---|---|
| Pre-session M2 work (account-deletion plan, cron parity, inbound-media, cross-channel, graceful-shutdown) | … → `8e290fa` | — |
| Plan A — workspace foundation | `e6f1da4` → `2fe90a0` | 35 commits |
| Plan C — evaluator + cron integration | `2464692` → `6643b0c` | 10 commits |
| Plan D — dream subsystem | `5be59f9` → `8f60648` | 11 commits |

Each plan's final commit leaves `cargo build --package plexus-server` clean and the full test suite green. Commits follow the pattern `feat:` / `fix:` / `refactor:` / `polish:` / `docs:` with substantive body paragraphs.

---

## 5. Docs trail

Everything design- or decision-level is written down. Start here:

### Specs (intent + architecture)
- `docs/superpowers/specs/2026-04-17-workspace-and-autonomy-design.md` — **the primary spec.** 15 sections. Covers workspace, toolset, templates, frontend workspace page, dream, heartbeat, schema changes, cross-cutting infra, proposed ADRs, success criteria. This is the single most load-bearing doc for the remaining work.
- `docs/superpowers/specs/2026-04-17-m2-closeout-design.md` — the wrap-up scope: account deletion, admin user-management, graceful-shutdown extension, unread badge. Parts A/B of this doc were superseded by the autonomy spec (dream/heartbeat moved there).
- `docs/superpowers/specs/2026-04-09-m2-server-design.md` — the original M2 server spec. Historical, but still useful for older design decisions.
- `docs/superpowers/specs/2026-04-15-inbound-media-design.md` — inbound-media design. Executed pre-session; informs Plan A's upload refactor.
- `docs/superpowers/specs/2026-04-08-m1-common-client-design.md` / `2026-04-10-m3-gateway-frontend-design.md` — protocol + client + gateway specs. Executed pre-session.

### Plans (execution-level, TDD-style)
- `docs/superpowers/plans/2026-04-17-workspace-foundation.md` — Plan A, 21 tasks + Post-Plan Adjustments footer.
- `docs/superpowers/plans/2026-04-17-shared-evaluator-cron-integration.md` — Plan C, 5 tasks.
- `docs/superpowers/plans/2026-04-17-dream-subsystem.md` — Plan D, 10 tasks.
- `docs/superpowers/plans/2026-04-16-account-deletion.md` — M2 closeout item (not yet executed).
- Older plans for the pre-session M2 work are in the same directory.

### Per-crate docs
- `plexus-server/docs/DECISIONS.md` — ADR log. Latest is ADR-35 (dream), added by D-10. Note: ADR numbering drifted from the autonomy spec's proposed numbering (§12 of that spec proposed ADR-35 through ADR-43; the DB only picked up ADR-35 for dream). A future docs-alignment pass will reconcile.
- `plexus-server/docs/SCHEMA.md` — the database schema. Should be updated to reflect Plan A's column drops + Plan D's `last_dream_at` addition; **currently stale**.
- `plexus-server/docs/API.md` — HTTP endpoints. Updated for Plan A's 410-Gone endpoints and Plan C's DELETE `/api/cron-jobs/{id}` response change.
- `plexus-server/docs/ISSUE.md` — tracked open / deferred / closed issues. D-10 added four Plan D follow-ups.
- `plexus-gateway/docs/` and `plexus-frontend/docs/` — crate-level issue logs and protocol notes.

### README
- `Plexus/README.md` — top-level project intro. Out of date for the workspace rewrite; worth a sweep before M2 ships.

---

## 6. Testing status

**Unit + integration tests:**
- 132 passing in `plexus-server`. Covers all new modules (workspace paths, quota, registration, skills cache, evaluator, dream, ToolAllowlist, context modes).
- 40 passing in `plexus-gateway`.
- 33 passing in `plexus-common`.
- 10 `#[ignore]`-gated DB-integration tests (require `DATABASE_URL`). Run: `cargo test --package plexus-server -- --ignored` with a DB set up.

**What is NOT tested:**
- End-to-end LLM pipelines for dream (Phase 1 output shape, Phase 2 applying directives). Deferred — needs either a mock LLM layer or a staging environment with an API key.
- End-to-end heartbeat flow (doesn't exist yet — Plan E).
- Multi-instance concurrency (two Plexus servers racing on the same DB). Plan C's TOCTOU fix on `ensure_system_cron_job` (partial unique index + `ON CONFLICT DO NOTHING`) is the only place we've proactively closed the race; others are documented as known-acceptable.
- Frontend. `plexus-frontend` has no test harness yet (Vitest never wired up). Manual smoke is the current approach.

**Build cleanliness:** ~3 expected dead-code warnings (`update_timezone`, Heartbeat-consuming stubs). All will be cleared by Plan E + Plan B.

---

## 7. What's next, in order

### Plan E — Heartbeat subsystem

**Spec reference:** `docs/superpowers/specs/2026-04-17-workspace-and-autonomy-design.md` §9.

**The shape:**
- In-process tick loop (not cron — cron's minimum granularity is minutes; heartbeat is fixed 30-min intervals but checked centrally per-server).
- Reads `system_config.heartbeat_interval_seconds` (default 1800, seeded by A-20).
- Per user, every 30 min, reads `HEARTBEAT.md` + current local time, calls the LLM **Phase 1** with a virtual `heartbeat(action: "skip" | "run", tasks: string)` tool.
- `action == "skip"` → done (zero further cost).
- `action == "run"` → publish `InboundEvent { kind: Heartbeat, content: tasks }`. Agent loop routes to `PromptMode::Heartbeat` (the stub from D-8 gets filled in) with **all** tools available (unlike dream's restricted allowlist).
- After Phase 2, the shared evaluator (Plan C's `evaluate_notification`) gates delivery — the 4 AM guard fires here, not for dream (which is always silent).
- Delivery is external-channel-only (Discord / Telegram), never gateway — heartbeat notifications don't interrupt an active browser session.
- User timezone (`users.timezone` — added by Plan A's A-2) feeds the Phase 1 prompt + the evaluator.
- New columns: `users.last_heartbeat_at`.

**Estimated scope:** ~8-10 tasks, similar shape to Plan D.

**Unlocks:** final consumer of `EventKind::Heartbeat`, `PromptMode::Heartbeat`, and the shared evaluator. After E lands, the autonomy design is fully realized server-side.

### Plan B — Frontend Workspace page

**Spec reference:** `docs/superpowers/specs/2026-04-17-workspace-and-autonomy-design.md` §7.

**The shape:**
- New `/settings/workspace` route in the React frontend. Tree view + content pane (markdown render + edit, images inline, binaries as download links).
- **Quota bar** with amber/red thresholds and soft-lock messaging.
- Drag-and-drop upload.
- Quick-access sidebar: SOUL.md / MEMORY.md / HEARTBEAT.md.
- **Deletes** the old Settings → Soul and Settings → Memory text-area editors (currently broken — they hit 410-Gone endpoints from A-17's cutover).
- New `/api/workspace/*` REST endpoints: tree, file, upload, quota, skills.

**Urgency:** frontend currently has a user-visible gap — Settings.tsx calls to `PATCH /api/user/soul` / `PATCH /api/user/memory` / `POST /api/skills/install` all return 410 since A-17. Plan B closes that gap.

**Estimated scope:** ~15 tasks (6 REST endpoints, React page, removal of the old text-area editors, test harness bootstrap). Frontend-heavy. Probably the largest plan of the lot.

### M2 closeout backlog

After Plan B lands, the M2 closeout items are:

- **Account deletion** — plan exists at `2026-04-16-account-deletion.md`, already updated (A-21) to wipe the workspace tree and call `QuotaCache::forget_user`. 9 tasks queued.
- **Admin user-management UI** — `GET /api/admin/users` endpoint + an Admin-page tab. Builds on the admin DELETE endpoint that lands with account deletion.
- **Graceful-shutdown extension** — Discord/Telegram bot poll loops + per-session agent loops join the ADR-34 cancellation fan-out so `SIGTERM` drains cleanly.
- **Session-list unread badge** — small frontend UX fix so non-viewed sessions receiving a `session_update` frame get a visible dot.

Reference: `docs/superpowers/specs/2026-04-17-m2-closeout-design.md`.

---

## 8. Known follow-ups & deferred items

Documented across each crate's `ISSUE.md` and the plan-level Post-Plan Adjustments footers.

**From Plan A:**
- **Frontend Settings.tsx broken** for soul/memory/skills-mutation endpoints. Plan B fixes.
- Docs drift in `plexus-server/docs/SCHEMA.md` and `plexus-server/README.md` — still advertise `save_memory` / `edit_memory` / `read_skill` / `install_skill` as live tools. Docs-sync pass owed.
- `update_timezone` dead-code warning — Plan B's Settings timezone editor will consume it.

**From Plan C:**
- API.md doc sweep owed for all 410'd endpoints (partially done; one round of it shipped at commit `3de3049` for the cron DELETE).
- Test-helper duplication in `state.rs` (`test_minimal` / `test_minimal_with_quota` / `test_minimal_with_outbound` / `test_with_pool`) — could be DRY'd with a single builder.

**From Plan D:**
- End-to-end dream integration tests (need mock LLM or staging).
- Dream session retention policy (no GC yet; `dream:*` rows accumulate).
- Observability for dream degradation (broken LLM silently no-ops every 2h).
- Tail-drop on 200-message cap — Phase 1 caps at 200 messages; currently `last_dream_at` advances to NOW, dropping the 201st+ messages from the window.

**Other:**
- ADR numbering drift (spec proposed ADR-35 → ADR-43; only ADR-35 exists in the DB). Reconcile during docs-sync.
- Plan A's Post-Plan Adjustments footer captured 7 deviations; Plan C and D have similar discrepancies worth footer-tracking during the final review.

---

## 9. Resuming work

To continue from this point, a new engineer or session should:

1. **Skim this doc** (you are reading it).
2. **Read §1–§4 of the autonomy spec** (`docs/superpowers/specs/2026-04-17-workspace-and-autonomy-design.md`) for architecture.
3. **Read Plan D's execution narrative** to understand the dispatch pattern — heartbeat (Plan E) will mirror it closely.
4. **For Plan E:** write the plan via the `superpowers:writing-plans` skill, then execute via `superpowers:subagent-driven-development`. The autonomy spec's §9 + §11 are the source material.
5. **For Plan B:** same flow. The autonomy spec's §7 is the source material. Frontend is heavier — expect longer review cycles.
6. **For the final review pass:** Plan A + C + D + E + B should be reviewed end-to-end by a fresh code-reviewer subagent before declaring M2 done. That's the user's explicit plan ("we will do a final finishing review later").

All branch work is on `M3-gateway-frontend`. No worktree was used — the branch itself is the working surface.

---

## 10. Quick reference

- Latest commit: `8f60648`
- Branch: `M3-gateway-frontend`
- Primary spec: `docs/superpowers/specs/2026-04-17-workspace-and-autonomy-design.md`
- Complete plans: `2026-04-17-workspace-foundation.md`, `2026-04-17-shared-evaluator-cron-integration.md`, `2026-04-17-dream-subsystem.md`
- Build: `cargo build --workspace` — clean.
- Tests: `cargo test --workspace` — 205 passing + 10 ignore-gated.
- Next task: write Plan E (heartbeat subsystem).
