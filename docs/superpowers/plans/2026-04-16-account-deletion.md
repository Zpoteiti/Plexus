# M2 Account Deletion — Spec + Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use `- [ ]` syntax for tracking.

**Goal:** Let users delete their own accounts, and let admins delete any user. The delete must be complete: DB rows, in-memory state (sessions, devices, rate limits), on-disk files, live browser connections, and running channel bots all go away. Re-creating an account with the same email afterwards must be a clean no-history experience.

**Architecture:** A single `delete_user_everywhere(state, user_id)` service function orchestrates the teardown in the correct order (stop bots → evict in-memory → kick browsers → wipe disk → one DB delete that cascades through every child table). Two thin HTTP entry points — `DELETE /api/user` (self-serve, password-gated) and `DELETE /api/admin/users/{user_id}` (admin auth) — both call the same service. FK constraints gain `ON DELETE CASCADE` so the DB row removal propagates automatically. Gateway gains a new `kick_user` WS frame so active browser sessions drop cleanly.

**Tech stack:** axum 0.7 (two new routes), sqlx (migration + one new DB function), password hashing already present in `auth/`, tokio-tungstenite (new `kick_user` frame), React 19 + Tailwind 4 (minimal frontend UI).

**Parent branch:** `M3-gateway-frontend`, based on `96c3c2f` (graceful shutdown + dead-code cleanup).

---

## 1. Overview

Today Plexus has no `delete_user` code path. A user who wants to leave has to either ignore their account (credentials keep working, data persists) or get someone to `DELETE FROM users` by hand — which fails on every FK because no cascade is declared. That's a real M2 gap: a "fully functional server" should support the full account lifecycle.

This spec closes the gap with minimal new surface:
- One service function that handles teardown end-to-end.
- Two endpoints (self-serve + admin) that call it.
- A schema migration turning existing FKs into CASCADE.
- One new WS frame type so live browsers get kicked cleanly.
- A minimal settings-page button in the frontend.

## 2. Goals & Non-Goals

**Goals**

- `DELETE /api/user` with password re-entry deletes the caller's account cleanly.
- `DELETE /api/admin/users/{user_id}` lets an admin delete any user.
- Dependent DB rows (sessions, messages, device_tokens, discord_configs, telegram_configs, cron_jobs) are removed via `ON DELETE CASCADE`. Note: the `skills` table was dropped by Plan A-17; skills now live on disk under `{WORKSPACE_ROOT}/{user_id}/skills/` and are wiped by `wipe_workspace` below.
- Live in-memory state for the deleted user is evicted: sessions, rate-limit entry, devices, tool-schema cache.
- Running Discord / Telegram bots for the deleted user are stopped.
- On-disk workspace directory `{PLEXUS_WORKSPACE_ROOT}/{user_id}/` is recursively wiped (includes SOUL.md, MEMORY.md, HEARTBEAT.md, skills/, uploads/, and anything else the user has created).
- Any live browser WebSocket connections for the deleted user are kicked via a new gateway `kick_user` frame.
- Minimal frontend: a "Delete Account" button in Settings, password-confirmation modal, redirect to login on success.
- Admin deleting their own account is allowed; emits a loud `warn!` log if the caller is `is_admin=true`.

**Non-goals**

- Soft delete / tombstoning / undo window. Hard delete only.
- Audit log to DB. Log via `tracing` is sufficient for M2.
- Bulk-delete ("delete all users older than X"). Single-user endpoints only.
- Admin listing/search UI. Admin can delete via curl / admin panel with a known user_id. Listing is a separate follow-up.
- Restoring "last admin" invariant. If an admin deletes their own account and is the only admin, the system is left with no admin until a new one is bootstrapped. Warned, not prevented.
- Frontend admin delete UI. Admin uses the endpoint directly; a real admin panel is its own ticket.
- External-service revocation (Discord OAuth tokens, Telegram bot tokens). Those are server-side — we delete our copy of the bot token, so our server disconnects. External OAuth/webhook revocation at the vendor is out of scope.

## 3. Design

### 3.1 Deletion order

The service function does teardown in this order — each step is idempotent so a crash mid-delete doesn't leave a half-deleted account:

```
delete_user_everywhere(state, user_id):
  1. channels::discord::stop_bot(user_id)              // stops reading Discord
  2. channels::telegram::stop_bot(user_id)             // stops reading Telegram
  3. channels::gateway::kick_user(state, user_id)      // closes browser WSs
  4. evict_in_memory(state, user_id):
       - state.sessions.retain(|_, h| h.user_id != user_id)
       - state.devices: remove all entries for user_id
       - state.devices_by_user.remove(user_id)
       - state.rate_limiter.remove(user_id)
       - state.tool_schema_cache.remove(user_id)
       - state.pending: remove entries keyed by device_tokens of this user
  5. wipe_workspace(state, user_id):
       - fs::remove_dir_all("{state.config.workspace_root}/{user_id}")
       - ignore NotFound (user may never have been initialized)
       - state.quota.forget_user(user_id)  // drop in-memory quota cache entry
  6. db::users::delete_user(&state.db, user_id)        // CASCADE handles children
```

Each step logs at `info` level on success and `warn` on failure, but **errors in steps 1-5 do not abort the sequence** — we still try the DB delete, because leaving the DB row in place while the filesystem is already wiped is worse than both being gone. The service returns a summary struct listing which steps succeeded.

### 3.2 DB schema — CASCADE migration

Five FK constraints point at `users(user_id)`, plus one at `sessions(session_id)` that transitively needs to cascade. All get `ON DELETE CASCADE`:

| Table | FK field | References | Cascade target |
|---|---|---|---|
| `device_tokens` | `user_id` | `users(user_id)` | CASCADE |
| `sessions` | `user_id` | `users(user_id)` | CASCADE |
| `messages` | `session_id` | `sessions(session_id)` | CASCADE |
| `discord_configs` | `user_id` | `users(user_id)` | CASCADE |
| `telegram_configs` | `user_id` | `users(user_id)` | CASCADE |
| `cron_jobs` | `user_id` | `users(user_id)` | CASCADE |

(The `skills` table was dropped by Plan A-17 — skills are files on disk under `{WORKSPACE_ROOT}/{user_id}/skills/` and are removed by `wipe_workspace` in the service function. No DB cascade needed for skills.)

The migration runs `ALTER TABLE … DROP CONSTRAINT … ADD CONSTRAINT … ON DELETE CASCADE` for each. Idempotent — wrapped in `DO $$ BEGIN … EXCEPTION WHEN undefined_object THEN NULL; END $$` so re-running on a clean DB (where `CREATE TABLE IF NOT EXISTS` already creates CASCADE versions after this migration) is a no-op.

The `CREATE TABLE IF NOT EXISTS` statements in `db::mod::create_tables` are also updated to include `ON DELETE CASCADE` so fresh installs skip the migration path.

### 3.3 Gateway `kick_user` frame

New frame type sent from plexus-server → plexus-gateway:

```json
{"type": "kick_user", "user_id": "<user_id>"}
```

Gateway handler:
- Iterates `state.browsers`.
- For each entry where `conn.user_id == user_id`:
  - Calls `conn.cancel.cancel()` (already used by slow-browser eviction path).
  - Removes the entry from `state.browsers`.
- Logs a count at `info`.

Server-side helper `channels::gateway::kick_user(state, user_id)` emits this frame via the existing `state.gateway_sink`. If the gateway isn't connected, logs a warn and returns — the browser would be disconnected anyway once the gateway restarts and fails to validate its JWT.

### 3.4 Self-serve endpoint `DELETE /api/user`

Request body:
```json
{"password": "..."}
```

Response:
- 204 No Content on success.
- 401 if password doesn't match.
- 404 if the user has already been deleted (race).
- 500 on internal error.

Flow:
1. Extract user_id from JWT.
2. Load the user row; if missing, return 404.
3. Verify `password` against `password_hash` using the same `bcrypt`/`argon2` helper the login path uses (grep existing code — use whatever's there, don't invent).
4. If the caller is `is_admin=true`, emit `warn!("Admin {user_id} is deleting their own account")`.
5. Call `delete_user_everywhere(state, user_id)`.
6. Return 204.

The frontend redirects to `/login` on 204. The JWT remains technically valid for its TTL but every subsequent request fails at user-lookup time.

### 3.5 Admin endpoint `DELETE /api/admin/users/{user_id}`

Auth: admin JWT via the existing `admin_claims(&headers, &state)` helper.

Request body: none.

Response:
- 204 on success.
- 403 if caller is not admin.
- 404 if `user_id` doesn't exist.
- 500 on internal error.

No password re-entry — admin's own JWT is sufficient. Logs `info!("Admin {admin_id} deleting user {target_user_id}")`.

### 3.6 Frontend — minimal delete UI

In `plexus-frontend/src/pages/Settings.tsx`'s `ProfileTab` function. Plan B-15 removed the Soul/Memory textareas; the current ProfileTab has display-name / email / timezone and a "Soul & Memory" pointer paragraph to the Workspace page. Append the Danger Zone section at the bottom of ProfileTab, below everything else.

- Section titled "Danger Zone" at the bottom.
- A red "Delete Account" button.
- Clicking opens a modal:
  - Bold warning text: "This will permanently delete your account, all messages, channels, files, and settings. This cannot be undone."
  - Password input.
  - "Delete Forever" button (disabled while input is empty).
  - "Cancel" button.
- On submit: `api.delete('/api/user', { password })`.
  - On 204: clear auth store, redirect to `/login`.
  - On 401: show "Password incorrect" inline in the modal.
  - On any other error: show the error message inline.

The `api.delete` helper currently doesn't accept a body; `plexus-frontend/src/lib/api.ts` gets extended so DELETE can carry JSON.

### 3.7 Edge cases & invariants

- **User deletes themselves mid-agent-iteration**: the agent loop holds a DB connection and may try to insert a message after the session row is gone. The insert fails with an FK error; agent loop logs and exits. Acceptable "kill fast" model.
- **Concurrent delete requests**: DB's `DELETE FROM users WHERE user_id = $1` is idempotent (second call affects zero rows). Service-layer steps 1-5 also idempotent (stop_bot on a stopped bot is a no-op; remove_dir_all on a missing dir is handled). No locking needed.
- **Last admin deletes themselves**: loud warn, delete proceeds. Re-bootstrap requires direct DB access (`UPDATE users SET is_admin = true WHERE email = ...`).
- **Delete during server shutdown**: graceful-shutdown already drains outbound dispatch; the delete either completes or the request fails with 500 / connection drop. User retries.
- **User has Discord bot running and message arrives during delete**: `stop_bot` runs first, so the bot is already stopped before DB delete. No in-flight Discord messages get published to a non-existent session.
- **Browser was disconnected at delete time**: no one to kick. Next reconnect attempt will fail JWT user-lookup; browser shows login page.

## 4. File structure

| File | Change |
|---|---|
| `plexus-server/src/db/mod.rs` | Update `CREATE TABLE` statements to include `ON DELETE CASCADE`; add a migration block that converts existing FKs |
| `plexus-server/src/db/users.rs` | Add `pub async fn delete_user(pool, user_id) -> Result<bool, sqlx::Error>` |
| `plexus-server/src/auth/mod.rs` or similar | Confirm/extract a `verify_password(plaintext, hash)` helper already exists |
| `plexus-server/src/channels/gateway.rs` | Add `pub async fn kick_user(state, user_id)` that emits the new frame |
| `plexus-server/src/channels/mod.rs` or new `plexus-server/src/account.rs` | `pub async fn delete_user_everywhere(state, user_id)` service function |
| `plexus-server/src/api.rs` | Add `DELETE /api/user` route + `delete_self` handler |
| `plexus-server/src/auth/admin.rs` | Add `DELETE /api/admin/users/{user_id}` route + handler |
| `plexus-gateway/src/state.rs` | No change — reuses existing `OutboundFrame::SessionUpdate` variant? **No.** Add `OutboundFrame::KickUser(Value)` variant |
| `plexus-gateway/src/routing.rs` | `route_kick_user(state, user_id)` fans out cancel signals |
| `plexus-gateway/src/ws/plexus.rs` | Dispatch `type="kick_user"` frames |
| `plexus-gateway/src/ws/chat.rs` | Close matching browser WSs when frame arrives (or handle via cancel token) |
| `plexus-frontend/src/lib/api.ts` | `api.delete` accepts optional body |
| `plexus-frontend/src/pages/Settings.tsx` | Danger-zone section + button + modal + API call |
| `plexus-frontend/src/store/auth.ts` | Expose a `clearAndRedirect()` helper if not already present |

## 5. Testing strategy

- **DB migration**: a unit test that creates a test user + dependent rows across the cascaded tables (sessions, messages, device_tokens, cron_jobs — plus optionally discord/telegram_configs), calls `delete_user`, asserts the user row is gone and all dependent rows are gone. Also asserts that without CASCADE the delete would fail (negative test — delete a user that has a row in a non-cascade table to catch regressions). Run against a fresh Postgres instance or the ephemeral sqlx test pool.
- **Service function**: unit test with in-memory state (populated `state.sessions`, `state.devices`, a mocked file_store). Assert each step runs; assert the final state is clean.
- **Gateway kick_user**: unit test parallel to `test_route_session_update_fans_out_by_user_id` — insert 3 browsers (2 alice, 1 bob), call `route_kick_user(state, "alice")`, assert only alice browsers get cancelled / removed.
- **Endpoints**: integration tests via the existing test harness. Self-serve: wrong password → 401, right password → 204 + user gone. Admin: non-admin caller → 403, admin caller → 204.
- **Frontend**: manual smoke only (Vitest not installed); covered in Task 9.

## 6. Tasks

Seven implementation tasks + one manual smoke. Each uses TDD; each is one commit.

---

### Task AD-1: Schema — `ON DELETE CASCADE` on user-referencing FKs

**Files:**
- Modify: `plexus-server/src/db/mod.rs`

- [ ] **Step 1: Update `CREATE TABLE` statements**

For each of these in `create_tables`, append `ON DELETE CASCADE` to the FK:

```sql
-- device_tokens
user_id        TEXT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,

-- sessions
user_id        TEXT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,

-- messages
session_id     TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,

-- discord_configs
user_id           TEXT PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE,

-- cron_jobs
user_id         TEXT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,

-- telegram_configs
user_id           TEXT PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE,
```

Note: the `skills` table was dropped by Plan A-17 (see the `DROP TABLE IF EXISTS skills` migration in `db/mod.rs`). Skip it — no CREATE TABLE entry to update.

- [ ] **Step 2: Add an idempotent migration block**

After the `CREATE TABLE` list in `create_tables`, add ALTER statements that convert existing installs. Wrap each in `DO $$ … $$` so it's safe if the constraint was already recreated:

```rust
// Migration: add ON DELETE CASCADE to existing user-referencing FKs.
// Idempotent — re-dropping and re-adding with the same target is safe;
// the DO block swallows errors from constraints that already cascade
// (e.g. fresh DBs created after this migration landed).
let cascade_migrations = [
    ("device_tokens", "device_tokens_user_id_fkey", "user_id", "users(user_id)"),
    ("sessions", "sessions_user_id_fkey", "user_id", "users(user_id)"),
    ("messages", "messages_session_id_fkey", "session_id", "sessions(session_id)"),
    ("discord_configs", "discord_configs_user_id_fkey", "user_id", "users(user_id)"),
    ("cron_jobs", "cron_jobs_user_id_fkey", "user_id", "users(user_id)"),
    ("telegram_configs", "telegram_configs_user_id_fkey", "user_id", "users(user_id)"),
];
// Note: skills is gone (Plan A-17 DROP TABLE); no cascade needed.
for (table, constraint, col, refs) in cascade_migrations {
    let sql = format!(
        "DO $$ BEGIN \
           ALTER TABLE {table} DROP CONSTRAINT IF EXISTS {constraint}; \
           ALTER TABLE {table} ADD CONSTRAINT {constraint} \
             FOREIGN KEY ({col}) REFERENCES {refs} ON DELETE CASCADE; \
         END $$;"
    );
    if let Err(e) = sqlx::query(&sql).execute(pool).await {
        tracing::warn!("Cascade migration for {table}: {e}");
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo build -p plexus-server
cargo test -p plexus-server
```

All 152 existing tests still pass (this task has no new tests; the migration correctness is covered by Task AD-2's delete test). If you see a different count, note it in your report — tests may have grown further between plan authoring and execution.

- [ ] **Step 4: Commit**

```
db: add ON DELETE CASCADE to user-referencing FKs

CREATE TABLE statements gain "ON DELETE CASCADE" on every FK pointing
at users(user_id), plus the messages→sessions FK so the cascade flows
all the way. An idempotent DO-block migration converts existing installs
by dropping and re-adding each constraint; the block swallows errors
from constraints already in the desired shape so this is safe to run
repeatedly.

Precondition for Task AD-2 — delete_user can now do one DELETE FROM
users and rely on cascade for every dependent table.
```

---

### Task AD-2: `db::users::delete_user` + test

**Files:**
- Modify: `plexus-server/src/db/users.rs`

- [ ] **Step 1: Failing test**

At the bottom of `users.rs`, add a `#[cfg(test)] mod tests` block (create if missing):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    async fn test_pool() -> PgPool {
        // Reuse the same DATABASE_URL the main server uses; tests need a live DB.
        let url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set for DB tests");
        PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_delete_user_cascades_dependent_rows() {
        let pool = test_pool().await;
        crate::db::init_tables_for_tests(&pool).await; // helper that runs create_tables

        let uid = format!("test-user-{}", uuid::Uuid::new_v4());
        create_user(&pool, &uid, &format!("{uid}@example.com"), "hash", false)
            .await.unwrap();

        // Insert a row in every dependent table
        sqlx::query("INSERT INTO sessions (session_id, user_id) VALUES ($1, $2)")
            .bind(format!("sess-{uid}")).bind(&uid).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO messages (message_id, session_id, role, content) VALUES ($1, $2, 'user', 'hi')")
            .bind(format!("msg-{uid}")).bind(format!("sess-{uid}")).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO device_tokens (token, user_id, device_name) VALUES ($1, $2, 'dev')")
            .bind(format!("tok-{uid}")).bind(&uid).execute(&pool).await.unwrap();
        // cron_jobs: covers the Plan D system-cron case (dream job per user) too.
        sqlx::query(
            "INSERT INTO cron_jobs (job_id, user_id, name, kind, message, channel, chat_id) \
             VALUES ($1, $2, 'test-job', 'user', 'hi', 'gateway', '-')"
        ).bind(format!("cron-{uid}")).bind(&uid).execute(&pool).await.unwrap();
        // … similar for discord_configs, telegram_configs if you want extra coverage —
        //   adapt to the actual required columns. The assertions below cover the
        //   critical cascade paths (sessions, messages via sessions, device_tokens,
        //   cron_jobs); adding more is defensive but not strictly required.

        // Delete
        let affected = delete_user(&pool, &uid).await.unwrap();
        assert!(affected, "delete_user should report success");

        // Assert everything is gone
        let remaining_users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE user_id = $1")
            .bind(&uid).fetch_one(&pool).await.unwrap();
        assert_eq!(remaining_users, 0);

        let remaining_sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id = $1")
            .bind(&uid).fetch_one(&pool).await.unwrap();
        assert_eq!(remaining_sessions, 0);

        let remaining_messages: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE message_id = $1"
        ).bind(format!("msg-{uid}")).fetch_one(&pool).await.unwrap();
        assert_eq!(remaining_messages, 0);

        let remaining_tokens: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM device_tokens WHERE user_id = $1"
        ).bind(&uid).fetch_one(&pool).await.unwrap();
        assert_eq!(remaining_tokens, 0);

        let remaining_cron: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cron_jobs WHERE user_id = $1"
        ).bind(&uid).fetch_one(&pool).await.unwrap();
        assert_eq!(remaining_cron, 0, "cron_jobs should cascade");
    }
}
```

Note: this test requires a live Postgres. If the existing test suite doesn't already do DB integration, mark the test `#[ignore]` and document it runs via `cargo test -- --ignored`. Check how other DB code is tested — grep `#[tokio::test]` in `db/` first.

- [ ] **Step 2: Run — expect failure**

```bash
cargo test -p plexus-server test_delete_user_cascades
```

Expected: `delete_user` not defined.

- [ ] **Step 3: Implement**

In `plexus-server/src/db/users.rs`:

```rust
/// Delete a user and (via ON DELETE CASCADE) every row in every dependent
/// table. Returns true if a row was actually deleted, false if the user_id
/// did not exist.
pub async fn delete_user(pool: &PgPool, user_id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM users WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
```

- [ ] **Step 4: Tests pass**

```bash
cargo test -p plexus-server
```

- [ ] **Step 5: Commit**

```
db: add users::delete_user relying on ON DELETE CASCADE

Single DELETE statement; CASCADE (added in AD-1) takes care of every
dependent table. Returns bool so the caller can distinguish "deleted"
from "already gone".
```

---

### Task AD-3: Gateway `kick_user` frame (receiver side)

**Files:**
- Modify: `plexus-gateway/src/state.rs` — add `OutboundFrame::KickUser(Value)` variant.
- Modify: `plexus-gateway/src/routing.rs` — new `route_kick_user(state, user_id)`.
- Modify: `plexus-gateway/src/ws/plexus.rs` — dispatch `type="kick_user"` frames.

The structure mirrors CC-1 (`session_update`) almost exactly. The key difference: `kick_user` doesn't just notify browsers — it **closes** their connections.

- [ ] **Step 1: Failing test**

In `plexus-gateway/src/routing.rs`, extend the `#[cfg(test)] mod tests`:

```rust
#[test]
fn test_route_kick_user_cancels_matching_browsers() {
    let state = test_state();
    let (tx_a, _rx_a) = mpsc::channel::<OutboundFrame>(8);
    let cancel_a = CancellationToken::new();
    state.browsers.insert("chat-a".into(), BrowserConnection {
        tx: tx_a, user_id: "alice".into(), cancel: cancel_a.clone(),
    });
    let (tx_b, _rx_b) = mpsc::channel::<OutboundFrame>(8);
    let cancel_b = CancellationToken::new();
    state.browsers.insert("chat-b".into(), BrowserConnection {
        tx: tx_b, user_id: "bob".into(), cancel: cancel_b.clone(),
    });

    route_kick_user(&state, "alice");

    assert!(cancel_a.is_cancelled(), "alice's browser should be cancelled");
    assert!(!cancel_b.is_cancelled(), "bob's browser must not be touched");
    assert!(!state.browsers.contains_key("chat-a"), "alice's entry removed");
    assert!(state.browsers.contains_key("chat-b"), "bob's entry remains");
}
```

- [ ] **Step 2: Verify failure**

```bash
cargo test -p plexus-gateway test_route_kick_user
```

- [ ] **Step 3: Add `OutboundFrame::KickUser` variant**

In `plexus-gateway/src/state.rs`:

```rust
pub enum OutboundFrame {
    Message(serde_json::Value),
    Progress(serde_json::Value),
    Error(serde_json::Value),
    Ping,
    SessionUpdate(serde_json::Value),
    KickUser,  // NEW — signal for the chat WS handler to close
}
```

(No inner value needed — the routing function handles the cancel directly; this variant is only sent to the browser's `tx` so the chat loop can emit a close frame on its way out. If you'd rather keep it payloadless, that's fine. Alternative: don't extend `OutboundFrame` at all — just call `conn.cancel.cancel()` and let the existing cancel-aware loop tear down.)

Simpler path (recommended): **do not add `KickUser` to `OutboundFrame`.** `route_kick_user` just cancels + removes. The chat WS loop already observes the cancel token and closes its socket. No new variant needed.

If you take the simpler path, skip the `OutboundFrame` edit.

- [ ] **Step 4: Implement `route_kick_user`**

In `plexus-gateway/src/routing.rs`:

```rust
pub fn route_kick_user(state: &Arc<AppState>, user_id: &str) {
    if user_id.is_empty() {
        tracing::warn!("route_kick_user: empty user_id, skipping");
        return;
    }
    let mut kicked = Vec::new();
    for entry in state.browsers.iter() {
        if entry.value().user_id == user_id {
            entry.value().cancel.cancel();
            kicked.push(entry.key().clone());
        }
    }
    for chat_id in &kicked {
        state.browsers.remove(chat_id);
    }
    tracing::info!("route_kick_user user_id={user_id} kicked={}", kicked.len());
}
```

- [ ] **Step 5: Dispatch `type="kick_user"` from plexus**

In `plexus-gateway/src/ws/plexus.rs`, extend the frame-type `match`:

```rust
"kick_user" => {
    let user_id = parsed.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
    crate::routing::route_kick_user(&state, user_id);
}
```

- [ ] **Step 6: Test passes; full suite passes**

```bash
cargo build -p plexus-gateway
cargo test -p plexus-gateway
```

- [ ] **Step 7: Commit**

```
gateway: add kick_user frame that cancels browser connections

route_kick_user iterates state.browsers and cancels every entry whose
user_id matches, then removes them from the map. Plexus.rs dispatches
"type": "kick_user" frames from plexus-server. Used by the account-
deletion flow to close all live browser WSs for a deleted user.

No new OutboundFrame variant needed — the existing cancel token
plumbing carries the signal to the chat relay loop, which closes
its socket naturally on cancellation.
```

---

### Task AD-4: Server-side gateway helpers — `kick_user` emitter

**Files:**
- Modify: `plexus-server/src/channels/gateway.rs`

- [ ] **Step 1: Failing test**

Add to `channels/gateway.rs::mod tests`:

```rust
#[test]
fn test_build_kick_user_frame() {
    let frame = build_kick_user_frame("user-42");
    assert_eq!(frame["type"], "kick_user");
    assert_eq!(frame["user_id"], "user-42");
}
```

- [ ] **Step 2: Verify fail**

```bash
cargo test -p plexus-server channels::gateway::tests::test_build_kick_user_frame
```

- [ ] **Step 3: Implement emitter**

Below the existing `build_deliver_frame` helper in `channels/gateway.rs`:

```rust
fn build_kick_user_frame(user_id: &str) -> serde_json::Value {
    serde_json::json!({ "type": "kick_user", "user_id": user_id })
}

pub async fn kick_user(state: &AppState, user_id: &str) {
    let sink = state.gateway_sink.read().await;
    let Some(sink) = sink.as_ref() else {
        warn!("Gateway: not connected, cannot kick user {user_id}");
        return;
    };
    let msg = build_kick_user_frame(user_id);
    let json = serde_json::to_string(&msg).unwrap();
    let mut s = sink.lock().await;
    if let Err(e) = futures_util::SinkExt::send(
        &mut *s,
        tokio_tungstenite::tungstenite::Message::Text(json.into()),
    ).await {
        warn!("Gateway: kick_user send failed: {e}");
    }
}
```

- [ ] **Step 4: Test passes**

```bash
cargo test -p plexus-server
```

- [ ] **Step 5: Commit**

```
gateway outbound: add kick_user emitter

Sends a kick_user frame to plexus-gateway, which will cancel every
browser WebSocket owned by the target user_id. Called by the account-
deletion service (next task).
```

---

### Task AD-5: Service function — `delete_user_everywhere`

**Files:**
- Create: `plexus-server/src/account.rs` (new module).
- Modify: `plexus-server/src/main.rs` or `plexus-server/src/lib.rs` to register the module.

A new module keeps the orchestration code together and easy to find. Register with `mod account;` in `main.rs`.

- [ ] **Step 1: Failing test**

In `plexus-server/src/account.rs`:

```rust
//! Account deletion orchestration.
//!
//! A single `delete_user_everywhere` entry point owns the teardown
//! sequence: stop bots, kick browsers, evict in-memory state, wipe
//! files, DB delete (which cascades).

use crate::state::AppState;
use std::sync::Arc;
use tracing::{info, warn};

/// Summary of which teardown steps succeeded for observability.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DeletionSummary {
    pub discord_stopped: bool,
    pub telegram_stopped: bool,
    pub browsers_kicked: bool,
    pub in_memory_evicted: bool,
    pub files_wiped: bool,
    pub db_deleted: bool,
}

pub async fn delete_user_everywhere(
    state: &Arc<AppState>,
    user_id: &str,
) -> DeletionSummary {
    // Implementation comes in Step 3.
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_delete_user_evicts_in_memory_state() {
        // Build a minimal AppState with pre-populated sessions and rate_limiter
        // for user "victim" and user "survivor". Populate file_store at a temp
        // path. Call delete_user_everywhere("victim"). Assert victim's entries
        // are gone from state.sessions / state.rate_limiter, and the file
        // dir is gone. Assert survivor's entries are untouched.
        //
        // Skeleton only — fill in construction of AppState using whatever
        // test harness already exists (grep for other `async fn test_*` in
        // the crate that build a minimal AppState).
        //
        // If no test harness exists, mark this test #[ignore] and cover
        // the integration via Task AD-8 manual smoke.
    }
}
```

- [ ] **Step 2: Verify fail**

```bash
cargo test -p plexus-server account
```

- [ ] **Step 3: Implement the orchestration**

Body of `delete_user_everywhere`:

```rust
pub async fn delete_user_everywhere(
    state: &Arc<AppState>,
    user_id: &str,
) -> DeletionSummary {
    let mut summary = DeletionSummary::default();
    info!("Deleting user {user_id} — starting teardown");

    // 1. Stop channel bots. Each is idempotent.
    crate::channels::discord::stop_bot(user_id).await;
    summary.discord_stopped = true;

    crate::channels::telegram::stop_bot(user_id).await;
    summary.telegram_stopped = true;

    // 2. Kick live browser connections.
    crate::channels::gateway::kick_user(state, user_id).await;
    summary.browsers_kicked = true;

    // 3. Evict in-memory state.
    state.sessions.retain(|_, handle| handle.user_id != user_id);

    // Devices: collect keys first, then remove (DashMap iteration + mutation
    // on the same map is discouraged).
    let device_keys: Vec<String> = state
        .devices
        .iter()
        .filter(|e| e.value().user_id == user_id)
        .map(|e| e.key().clone())
        .collect();
    for key in &device_keys {
        state.devices.remove(key);
        state.pending.remove(key);
    }
    state.devices_by_user.remove(user_id);
    state.rate_limiter.remove(user_id);
    state.tool_schema_cache.remove(user_id);
    summary.in_memory_evicted = true;

    // 4. Wipe workspace.
    wipe_workspace(state, user_id).await;
    summary.files_wiped = true;

    // 5. DB delete — cascades through every dependent table.
    match crate::db::users::delete_user(&state.db, user_id).await {
        Ok(true) => {
            summary.db_deleted = true;
            info!("Deleted user {user_id} (db=success, summary={summary:?})");
        }
        Ok(false) => {
            warn!("User {user_id} already gone from DB (race?)");
        }
        Err(e) => {
            warn!("DB delete failed for {user_id}: {e}");
        }
    }

    summary
}

async fn wipe_workspace(state: &Arc<AppState>, user_id: &str) {
    let path = std::path::Path::new(&state.config.workspace_root).join(user_id);
    match tokio::fs::remove_dir_all(&path).await {
        Ok(()) => tracing::info!(user_id, "workspace wiped"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // User may never have been initialized.
        }
        Err(e) => tracing::warn!(user_id, error = %e, "failed to wipe workspace"),
    }
    state.quota.forget_user(user_id);
}
```

- [ ] **Step 4: Wire into main.rs**

Add `mod account;` at the top of `plexus-server/src/main.rs` alongside the other module declarations.

Verify `state.devices_by_user` and `state.tool_schema_cache` field names match actual state — grep if unsure.

- [ ] **Step 5: Build + test**

```bash
cargo build -p plexus-server
cargo test -p plexus-server
```

- [ ] **Step 6: Commit**

```
account: add delete_user_everywhere service orchestration

Single entry point that stops channel bots, kicks browsers via the
gateway kick_user frame, evicts in-memory state (sessions, devices,
rate limiter, tool schema cache), wipes the per-user workspace
directory (via wipe_workspace helper), then runs the cascade DB delete.

Each step is idempotent; errors on earlier steps do not abort the
sequence — we'd rather have partial teardown than a half-deleted
account with filesystem gone but DB intact. Returns a DeletionSummary
for the caller to log if desired.
```

---

### Task AD-6: Self-serve endpoint `DELETE /api/user`

**Files:**
- Modify: `plexus-server/src/api.rs`

- [ ] **Step 1: Locate the existing password verification helper**

```bash
grep -rn "bcrypt\|argon2\|verify_password\|password_hash" plexus-server/src/
```

Note the helper's exact signature. If no helper exists, the login handler must verify inline — extract that into a reusable function first, same commit OK.

- [ ] **Step 2: Failing test (integration, optional)**

If the existing endpoint tests use a mock AppState, write a test that:
- Creates a user with password "correct".
- Calls `DELETE /api/user` with `{"password": "wrong"}` → expect 401.
- Calls with `{"password": "correct"}` → expect 204.
- Verify `db::users::find_by_id` now returns None.

If the test harness is heavy / absent, skip and rely on Task AD-8 manual smoke.

- [ ] **Step 3: Add the handler**

In `plexus-server/src/api.rs`:

```rust
#[derive(Deserialize)]
struct DeleteSelfRequest {
    password: String,
}

async fn delete_self(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<DeleteSelfRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let claims = crate::auth::extract_claims(&headers, &state.config.jwt_secret)?;
    let user = crate::db::users::find_by_id(&state.db, &claims.sub)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "User not found"))?;

    // Verify password
    if !crate::auth::verify_password(&body.password, &user.password_hash) {
        return Err(ApiError::new(ErrorCode::Unauthorized, "Invalid password"));
    }

    // Admin self-delete: warn loudly.
    if user.is_admin {
        warn!(
            "Admin {user_id} ({email}) is deleting their own account",
            user_id = user.user_id, email = user.email,
        );
    }

    crate::account::delete_user_everywhere(&state, &user.user_id).await;

    Ok(Json(serde_json::json!({ "message": "Account deleted" })))
}
```

Wire the route into `api_routes()`:

```rust
.route("/api/user", delete(delete_self))
```

Add `delete` to the `axum::routing::{…}` import if not already there.

Adapt `crate::auth::verify_password` to whatever the actual helper is called. If you had to extract one in Step 1, use that.

- [ ] **Step 4: Build + test**

```bash
cargo build -p plexus-server
cargo test -p plexus-server
```

- [ ] **Step 5: Commit**

```
api: add DELETE /api/user self-serve account deletion

Requires password re-entry as a guard against XSS / session hijack
one-click disasters. On success, calls delete_user_everywhere which
handles the full teardown (bots, browsers, in-memory state, files, DB).

Admin self-delete is allowed but emits a loud warn log.
```

---

### Task AD-7: Admin endpoint `DELETE /api/admin/users/{user_id}`

**Files:**
- Modify: `plexus-server/src/auth/admin.rs`

- [ ] **Step 1: Add the handler**

```rust
async fn delete_user_by_admin(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let admin = admin_claims(&headers, &state)?;
    let user = crate::db::users::find_by_id(&state.db, &user_id)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "User not found"))?;

    info!(
        "Admin {admin_id} deleting user {target_id} ({email})",
        admin_id = admin.sub, target_id = user.user_id, email = user.email,
    );

    crate::account::delete_user_everywhere(&state, &user.user_id).await;

    Ok(Json(serde_json::json!({ "message": "User deleted" })))
}
```

Wire into `admin_routes()`:

```rust
.route("/api/admin/users/{user_id}", delete(delete_user_by_admin))
```

Add `Path` and `delete` to the imports at the top of `admin.rs`.

- [ ] **Step 2: Build**

```bash
cargo build -p plexus-server
cargo test -p plexus-server
```

- [ ] **Step 3: Commit**

```
admin: add DELETE /api/admin/users/{user_id}

Admin JWT only. No password re-entry — admin's auth is enough. Reuses
the same delete_user_everywhere service as the self-serve path; logs
the admin id + target email for audit.
```

---

### Task AD-8: Frontend — delete account UI

**Files:**
- Modify: `plexus-frontend/src/lib/api.ts` — allow optional body on DELETE.
- Modify: `plexus-frontend/src/pages/Settings.tsx` (or wherever the Account section lives — grep for existing profile/soul editor).

- [ ] **Step 1: Extend `api.delete` to accept a body**

In `plexus-frontend/src/lib/api.ts`:

```typescript
export const api = {
  get:    <T>(path: string)                => request<T>('GET',    path),
  post:   <T>(path: string, body: unknown) => request<T>('POST',   path, body),
  put:    <T>(path: string, body: unknown) => request<T>('PUT',    path, body),
  patch:  <T>(path: string, body: unknown) => request<T>('PATCH',  path, body),
  delete: <T>(path: string, body?: unknown) => request<T>('DELETE', path, body),
}
```

(The underlying `request` already passes body through when defined.)

- [ ] **Step 2: Add Danger Zone to Settings page**

Open `plexus-frontend/src/pages/Settings.tsx` and locate the `ProfileTab` function. Plan B-15 removed Soul/Memory textareas; ProfileTab now has display-name / email / timezone / a "Soul & Memory" pointer section. Append the Danger Zone section AFTER all existing sections in ProfileTab, as the last child of its returned JSX. Imports at the top of Settings.tsx (e.g. `useAuthStore`, `useNavigate`) — check whether these are already imported before re-adding.

Add:

```tsx
import { useState } from 'react'
import { useAuthStore } from '../store/auth'
import { useNavigate } from 'react-router-dom'
import { api } from '../lib/api'

// … inside the component:
const [showDeleteModal, setShowDeleteModal] = useState(false)
const [deletePassword, setDeletePassword] = useState('')
const [deleteError, setDeleteError] = useState<string | null>(null)
const [deleting, setDeleting] = useState(false)
const navigate = useNavigate()

async function confirmDelete() {
  setDeleting(true)
  setDeleteError(null)
  try {
    await api.delete<{ message: string }>('/api/user', { password: deletePassword })
    useAuthStore.getState().logout()
    navigate('/login')
  } catch (e) {
    setDeleteError((e as Error).message)
    setDeleting(false)
  }
}

// … in the JSX, below the existing settings sections:
<section className="mt-12 border-t pt-8" style={{ borderColor: 'var(--border)' }}>
  <h2 className="text-lg font-semibold text-red-500">Danger Zone</h2>
  <p className="text-sm mt-2" style={{ color: 'var(--muted-fg)' }}>
    Delete your account permanently. This removes all sessions, messages,
    channel configurations, files, and skills. Cannot be undone.
  </p>
  <button
    onClick={() => setShowDeleteModal(true)}
    className="mt-4 px-4 py-2 rounded font-medium"
    style={{ background: '#ef4444', color: 'white' }}
  >
    Delete Account
  </button>
</section>

{showDeleteModal && (
  <div
    className="fixed inset-0 flex items-center justify-center z-50"
    style={{ background: 'rgba(0,0,0,0.5)' }}
  >
    <div
      className="max-w-md w-full rounded-xl p-6 space-y-4"
      style={{ background: 'var(--card)', color: 'var(--text)' }}
    >
      <h3 className="text-lg font-semibold text-red-500">Delete Account?</h3>
      <p className="text-sm">
        This will permanently delete your account, all messages, channels,
        files, and settings. <strong>This cannot be undone.</strong>
      </p>
      <input
        type="password"
        value={deletePassword}
        onChange={e => setDeletePassword(e.target.value)}
        placeholder="Enter your password to confirm"
        className="w-full px-3 py-2 rounded border bg-transparent"
        style={{ borderColor: 'var(--border)', color: 'var(--text)' }}
        autoFocus
      />
      {deleteError && (
        <p className="text-sm text-red-500">{deleteError}</p>
      )}
      <div className="flex justify-end gap-2">
        <button
          onClick={() => {
            setShowDeleteModal(false)
            setDeletePassword('')
            setDeleteError(null)
          }}
          disabled={deleting}
          className="px-4 py-2 rounded"
          style={{ background: 'var(--muted)', color: 'var(--text)' }}
        >
          Cancel
        </button>
        <button
          onClick={confirmDelete}
          disabled={deleting || !deletePassword}
          className="px-4 py-2 rounded font-medium disabled:opacity-40"
          style={{ background: '#ef4444', color: 'white' }}
        >
          {deleting ? 'Deleting…' : 'Delete Forever'}
        </button>
      </div>
    </div>
  </div>
)}
```

Adapt the styling to match existing settings components (the classes above are a reasonable starting point).

- [ ] **Step 3: Build**

```bash
cd plexus-frontend && npm run build
```

- [ ] **Step 4: Commit**

```
frontend: add Delete Account danger zone + confirm modal

Red button in Settings → password-gated modal → DELETE /api/user with
{ password }. On 204 the auth store clears and the user lands on
/login. Incorrect password surfaces the server's 401 error inline.

api.delete now optionally accepts a body; the underlying request
wrapper already supported it — only the exported helper signature
changed.
```

---

### Task AD-9: Manual E2E smoke tests

- [ ] **Step 1: Self-serve happy path**

1. Register a new test user, log in.
2. Populate: start a Discord bot, create a cron job, upload a file via the chat, send a few messages.
3. Open two browser tabs.
4. In one tab, go to Settings → Danger Zone → Delete Account → enter correct password → Delete Forever.
5. Expected: that tab redirects to /login; the other tab's WebSocket drops (server logs "Gateway: route_kick_user user_id=… kicked=2"). The Discord bot goes offline. The workspace directory `{PLEXUS_WORKSPACE_ROOT}/<user_id>/` is gone (includes uploads/, MEMORY.md, etc.). DB: `SELECT COUNT(*) FROM messages WHERE session_id IN (SELECT session_id FROM sessions WHERE user_id = '<user>')` returns 0.

- [ ] **Step 2: Wrong password**

1. Repeat with a fresh test user.
2. Submit the modal with the wrong password.
3. Expected: modal shows "Invalid password"; the user is NOT deleted; existing sessions still work.

- [ ] **Step 3: Admin delete**

1. As admin, hit `DELETE /api/admin/users/<target_user_id>` (curl or an admin tool).
2. Expected: 204; target user is gone; bots for target stopped; admin's own sessions untouched.

- [ ] **Step 4: Admin self-delete warning**

1. As admin, self-serve delete own account.
2. Expected: 204; server logs `WARN Admin <id> (<email>) is deleting their own account`.
3. Log in with any other user — system still works except there's no admin; admin-only endpoints return 403 for everyone.

- [ ] **Step 5: Concurrent race**

1. Open two browser tabs both on Settings.
2. In both, click Delete Account and submit the password at roughly the same time.
3. Expected: one succeeds (204); the other sees 404 "User not found" (raced). No crashes, no half-state.

- [ ] **Step 6: Re-register with same email**

1. After a successful delete, register a new account with the same email.
2. Expected: success — the old account and all its data are gone; the new account starts empty.

---

## 7. Out-of-scope follow-ups

- Admin user-listing UI (search / pagination / bulk actions).
- Soft-delete / grace-period undo window.
- External-service revocation (calling Discord API to remove the bot from guilds, Telegram's `revokeBotToken`, etc.).
- Delete audit log to a dedicated table.
- "Last admin can't delete self" invariant.
- Drain-then-delete flow for mid-agent-iteration safety.
- Bulk deletion / data export ("download my data before I delete") — GDPR-adjacent, separate spec if we need it.
