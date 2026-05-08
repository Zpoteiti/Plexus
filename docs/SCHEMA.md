# Plexus — Database Schema

The PostgreSQL schema for `plexus-server`. Lives at `plexus-server/src/db/schema.sql`, applied at startup via `sqlx::raw_sql(include_str!("schema.sql"))` with `IF NOT EXISTS` semantics (ADR-057).

**Ten tables.** Account deletion is a single `DELETE FROM users WHERE id = $1`; every user-referencing FK has `ON DELETE CASCADE` defined inline (ADR-058) — with one explicit exception in `workspaces.created_by` (`ON DELETE SET NULL`, see ADR-108) so a workspace persists for its remaining members when the creator's account is removed.

This doc is the canonical reference for the schema's *shape*. The SQL file is the canonical reference for the schema's *bytes* — when they disagree, the SQL wins, this doc is then updated.

---

## 1. `system_config` — global key-value store

```sql
CREATE TABLE IF NOT EXISTS system_config (
    key         TEXT        PRIMARY KEY,
    value       JSONB       NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

Known keys (admin-editable via `PATCH /api/admin/config`):

| Key | Type | ADR | Purpose |
|---|---|---|---|
| `quota_bytes` | int | ADR-046 | Per-user workspace quota (default 5 GB). |
| `llm_endpoint` | string | ADR-101 | OpenAI-compatible chat-completions base URL. |
| `llm_api_key` | string | ADR-101 | Bearer credential for outbound LLM calls. |
| `llm_model` | string | ADR-101 | Model name passed in request body. |
| `llm_max_context_tokens` | int | ADR-101 | LLM context window in tokens (e.g. `128000` for gpt-4o). Counted with tiktoken-rs (ADR-025). |
| `llm_compaction_threshold_tokens` | int | ADR-028, ADR-101 | Headroom that triggers compaction. Default `16000`. Summary `max_output_tokens` = `threshold − 4000`. |
| `shared_workspace_quota_bytes` | int | ADR-108 | Quota ceiling that any single shared workspace may request at create or rename time. Default 25 GB. |

Deployments may carry additional opaque keys; Plexus reads only the ones it knows about.

---

## 2. `users` — Plexus accounts

```sql
CREATE TABLE IF NOT EXISTS users (
    id             UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    email          TEXT         NOT NULL UNIQUE,
    password_hash  TEXT         NOT NULL,
    name           TEXT         NOT NULL,
    is_admin       BOOLEAN      NOT NULL DEFAULT FALSE,
    created_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);
```

- `password_hash` — argon2 (or bcrypt — implementer's choice within reason). Never returned by any API.
- `is_admin` — true for any user who registered with the `ADMIN_TOKEN` (ADR — multi-admin, no last-admin invariant per ADR-065).
- **No `soul`, `memory_text`, or `ssrf_whitelist` columns** — workspace-file-only per ADR-060.
- **No inline channel fields** — Discord/Telegram live in their own tables (ADR-090).
- **No `bytes_used` column** — workspace usage is computed on demand by `workspace_fs` summing the user's tree on disk (or maintained via a denormalized counter, implementer's choice within `workspace_fs`; not part of the API contract).

---

## 3. `discord_configs` — per-user Discord bot integration

```sql
CREATE TABLE IF NOT EXISTS discord_configs (
    user_id          UUID         PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    bot_token        TEXT         NOT NULL,
    partner_chat_id  TEXT         NOT NULL,
    allow_list       JSONB        NOT NULL DEFAULT '[]'::jsonb,
    created_at       TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);
```

- `user_id` is both PK and FK — at most one Discord config per user.
- `bot_token` is the Discord bot's secret. API never returns it; `GET /api/me/discord` masks it (ADR derives — never logged plain either).
- `partner_chat_id` is the partner human's Discord user ID. Messages from this ID are *not* wrapped (`[untrusted message from <name>]:`); messages from anyone else are (ADR-007).
- `allow_list` — JSONB array of heterogeneous Discord identifiers the partner has authorized to also reach the bot. Each entry is one of:
    - **User ID** (e.g. `"123456789012345678"`) — the named user is allowed to DM the bot or @-mention it in any channel.
    - **Channel ID** — every member of that channel can @-mention the bot in that channel.
    - **Guild ID** — every member of that guild can @-mention the bot in any of its channels.
  Inbound message is allowed if its sender_id matches a User ID entry **OR** its message-context (channel, guild) matches a Channel/Guild ID entry. Allowed messages still get the `[untrusted message from <name>]:` wrap (ADR-007); only the partner is unwrapped. Agent treats allow-list senders as non-partner allowed users (see ADR-074 trust model). Format is positional — entries are stored verbatim as Discord-snowflake-shaped strings; the adapter classifies (user/channel/guild) by API lookup at receive time, not by string form.

---

## 4. `telegram_configs` — per-user Telegram bot integration

```sql
CREATE TABLE IF NOT EXISTS telegram_configs (
    user_id          UUID         PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    bot_token        TEXT         NOT NULL,
    partner_chat_id  TEXT         NOT NULL,
    allow_list       JSONB        NOT NULL DEFAULT '[]'::jsonb,
    created_at       TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);
```

Symmetric to `discord_configs`. `allow_list` follows the same heterogeneous-identifier rule (Telegram terminology):
- **User ID** — the named user can DM the bot.
- **Chat ID** of a group — every member of that group can @-mention the bot in the group.
- **Channel ID** — broadcast-channel admins can post; allowed bot interactions follow Telegram's bot-in-channel API rules.

Match logic identical to Discord — sender_id ∪ chat-context-id checked against the list; allowed messages get the untrusted wrap.

Adding a future channel = adding a `<channel>_configs` table; no `users` migration (ADR-090).

---

## 5. `sessions` — chat sessions per channel-conversation

```sql
CREATE TABLE IF NOT EXISTS sessions (
    id                UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id           UUID         NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_key       TEXT         NOT NULL UNIQUE,
    channel           TEXT         NOT NULL,
    chat_id           TEXT         NOT NULL,
    last_inbound_at   TIMESTAMPTZ,
    cancel_requested  BOOLEAN      NOT NULL DEFAULT FALSE,
    created_at        TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);
```

- `session_key` is the composite identity from ADR-006 — `{channel}:{chat_id}` for external channels, an override (`cron:{job_id}`, `heartbeat:{user_id}`, `web:{...}`) for internal/web sessions. UNIQUE because it's the de-facto lookup key.
- `id` is the internal UUID used as FK target by `messages.session_id`. Most internal code uses `id`; external surfaces use `session_key`.
- `last_inbound_at` — bumped on every new InboundMessage; powers session-list ordering in the UI.
- `cancel_requested` — set true by `POST /api/sessions/{id}/cancel` (ADR-035), observed at the next iteration boundary, then cleared.

---

## 6. `messages` — every assistant/user/tool turn

```sql
CREATE TABLE IF NOT EXISTS messages (
    id                       UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id               UUID         NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role                     TEXT         NOT NULL CHECK (role IN ('user', 'assistant', 'tool')),
    content                  JSONB        NOT NULL,
    is_compaction_summary    BOOLEAN      NOT NULL DEFAULT FALSE,
    created_at               TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_messages_session_created
    ON messages(session_id, created_at);
```

- `content` — JSONB array of OpenAI chat-completions content blocks (ADR-059, ADR-101). Block shapes mirror what the LLM will receive — request body is a pass-through with no translation. **Images** are stored both as `image_url` blocks with base64 data URLs inline AND on disk under server's `.attachments/` (ADR-044). **Non-image files** (PDFs, CSVs, audio, ...) live ONLY on disk under `.attachments/`; the DB carries just the path-text marker block (ADR-027) since OpenAI chat completions has no `file_url` block type.
- `role` — strictly `user`, `assistant`, or `tool` (ADR-089). No synthetic roles. Compaction summaries use `role='assistant'` plus `is_compaction_summary=true`.
- The `idx_messages_session_created` index powers the SSE replay and the `GET /api/sessions/{id}/messages` cursor scan.
- Runtime block (ADR-094) is prepended into the user-row's `content` JSONB at ingress time; not a separate column.

---

## 7. `devices` — per-user client devices

```sql
CREATE TABLE IF NOT EXISTS devices (
    token              TEXT         PRIMARY KEY,
    user_id            UUID         NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name               TEXT         NOT NULL,
    workspace_path     TEXT         NOT NULL,
    fs_policy          TEXT         NOT NULL CHECK (fs_policy IN ('sandbox', 'unrestricted'))
                                    DEFAULT 'sandbox',
    shell_timeout_max  INTEGER      NOT NULL DEFAULT 300,
    ssrf_whitelist     JSONB        NOT NULL DEFAULT '[]'::jsonb,
    mcp_servers        JSONB        NOT NULL DEFAULT '{}'::jsonb,
    created_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, name)
);

CREATE INDEX IF NOT EXISTS idx_devices_user_id ON devices(user_id);
```

- `token` is the PK *and* the credential (ADR-091). Stored plaintext — it IS the credential. WS handshake `Authorization: Bearer <token>` is matched directly against this column.
- `name` is the user-facing friendly label. UNIQUE per user, so the URL `PATCH /api/devices/laptop/config` resolves to `(user_id, "laptop")` without ever touching the token.
- `ssrf_whitelist` — JSONB array of `host` or `host:port` strings, exceptions to the client-site `web_fetch` block-list (ADR-052). Capability declaration only — does not stop `exec curl` (ADR-073).
- `mcp_servers` — JSONB map of `<server_name>: McpServerConfig` (see API.yaml). Pushed to the live device via `config_update` frame on change (ADR-050, PROTOCOL.md §3.6).
- **Online state is in-memory only** — no `online` / `last_seen_at` columns; the connected-WS map (`DashMap<token, ConnHandle>`) is the source of truth. The `Device` API response computes them on demand. Three device states per ADR-110: state-1 (online, in-map), state-2 (offline-but-paired, row exists, not in map — listed in `plexus_device` enum so the agent can still attempt and fail loudly), state-3 (deleted — row gone, in-memory entry gone, live WS force-closed, tool registry invalidated; complete wipe with no soft-delete tombstone).
- **No inbound FKs reference `devices`** from other tables. State-3 transition is a single-row DELETE; cascades from `users.id` are the only path that takes multiple device rows out at once (account deletion).
- `workspace_path` default is the literal string `~/plexus/workspace` on every OS (ADR-111) when omitted from `POST /api/devices`. The client expands `~` against its own home dir at startup; the server stores and returns the unexpanded form.

---

## 8. `workspaces` — shared workspace registry (ADR-108)

```sql
CREATE TABLE IF NOT EXISTS workspaces (
    id           UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    name         TEXT         NOT NULL,
    quota_bytes  BIGINT       NOT NULL,
    created_by   UUID         REFERENCES users(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);
```

- `id` — UUID primary key. Drives the on-disk path `{PLEXUS_WORKSPACE_ROOT}/{id}/` and the public-facing `name@suffix` addressing form (ADR-108) where `suffix` is the first 8 hex chars of `id` (auto-extended on collision per ADR-108).
- `name` — display label. **Not unique.** Two unrelated teams may both create a workspace called "Xmas gift". The validator in ADR-109 enforces character rules (no `/`, `\`, `@`, `:`, control chars, etc.), NFC-normalizes, and length-caps at 64 chars.
- `quota_bytes` — capped at `system_config.shared_workspace_quota_bytes` at create and rename time.
- `created_by` — author. **Exception to ADR-058**: uses `ON DELETE SET NULL`, not `CASCADE`. Removing the creator's user account does not delete a workspace that still has other members; `created_by` becomes NULL and the membership rows survive.
- Last-member-leaves auto-deletion (per `DELETE FROM workspace_members WHERE workspace_id = $1`) is enforced in application code (`workspace_fs`), not SQL — when no `workspace_members` rows remain for a `workspaces.id`, the row is deleted and the on-disk directory is removed.

---

## 9. `workspace_members` — shared workspace allow-list (ADR-108)

```sql
CREATE TABLE IF NOT EXISTS workspace_members (
    workspace_id  UUID         NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id       UUID         NOT NULL REFERENCES users(id)      ON DELETE CASCADE,
    joined_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_workspace_members_user ON workspace_members(user_id);
```

- Composite PK is the natural identity (a user is in a workspace at most once).
- Two cascades: deleting a workspace removes all its members; deleting a user removes them from every workspace they joined.
- `idx_workspace_members_user` powers the per-user "list my workspaces" query that runs at every `build_context` to render the system prompt's Workspaces section.

---

## 10. `cron_jobs` — scheduled agent invocations

```sql
CREATE TABLE IF NOT EXISTS cron_jobs (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID         NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    schedule        TEXT         NOT NULL,
    one_shot        BOOLEAN      NOT NULL DEFAULT FALSE,
    description     TEXT         NOT NULL,
    channel         TEXT         NOT NULL,
    chat_id         TEXT         NOT NULL,
    deliver         BOOLEAN      NOT NULL DEFAULT TRUE,
    last_fired_at   TIMESTAMPTZ,
    next_fire_at    TIMESTAMPTZ  NOT NULL,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_cron_jobs_user_id    ON cron_jobs(user_id);
CREATE INDEX IF NOT EXISTS idx_cron_jobs_next_fire  ON cron_jobs(next_fire_at)
    WHERE next_fire_at IS NOT NULL;
```

- `schedule` — normalized cron expression (server parses agent-supplied `every_seconds` / `cron_expr` / `at` into a single canonical form at insert time).
- `one_shot` — true when the agent created the job from a `cron(action="add", at=...)` call (one-time future trigger). Once fired and delivered, the row is deleted.
- `description` — the agent-facing instruction the scheduler will inject into the heartbeat session when the job fires.
- `channel` + `chat_id` — inherited from the creating session per ADR-053. The reply lands where the user originally set up the cron.
- `deliver` — when false, the agent runs but the result doesn't post back to the channel (silent maintenance jobs).
- `next_fire_at` — denormalized for the scheduler index. Recomputed each time the job fires.
- **No `kind` column** — heartbeat is a tick loop, not a cron row, and Dream is deferred (ADR-055, ADR-092).

---

## Constraints summary

- Every user-referencing FK has `ON DELETE CASCADE` (ADR-058) → account deletion is one statement. **Sole exception:** `workspaces.created_by` uses `ON DELETE SET NULL` per ADR-108 so a workspace persists for its remaining members when its creator's account is removed.
- No surrogate "is_active" / "deleted_at" columns — deletes are hard, undo lives in admin's backup strategy.
- No migration framework in v1 (ADR-069). Schema changes during rebuild require dev-DB reset (`scripts/reset-db.sh`). Real-user deployments add `sqlx::migrate!` later.

---

## Indexes summary

| Index | Table | Purpose |
|---|---|---|
| `users_email_key` | users (UNIQUE on `email`) | Login lookup. |
| `idx_sessions_user_id` | sessions | List user's sessions. |
| `sessions_session_key_key` | sessions (UNIQUE on `session_key`) | Channel-message → session lookup. |
| `idx_messages_session_created` | messages (`session_id, created_at`) | History replay + cursor scan. |
| `idx_devices_user_id` | devices | List user's devices. |
| `devices_user_id_name_key` | devices (UNIQUE on `(user_id, name)`) | URL resolution `/api/devices/{name}`. |
| `workspace_members_pkey` | workspace_members (PK on `(workspace_id, user_id)`) | Membership lookup at workspace-fs entry. |
| `idx_workspace_members_user` | workspace_members | Per-user "list my workspaces" for system-prompt rebuild. |
| `idx_cron_jobs_user_id` | cron_jobs | List user's cron jobs. |
| `idx_cron_jobs_next_fire` | cron_jobs (`next_fire_at`) | Scheduler poll. |

---

## Extensions

- `uuid-ossp` or `pgcrypto` for `gen_random_uuid()` — `pgcrypto` is built-in on most PostgreSQL distributions and is the default choice.
- No other extensions in v1.
