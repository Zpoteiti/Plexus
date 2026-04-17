# Workspace Foundation Implementation Plan (Plan A of 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Spec reference:** The full design lives at `/home/yucheng/Documents/GitHub/Plexus/docs/superpowers/specs/2026-04-17-workspace-and-autonomy-design.md`. **Always read the spec if this plan's context seems incomplete** — the spec carries the "why" and decisions behind every choice here.
>
> **This is Plan A of 5.** Remaining plans (not yet written): **B** frontend Workspace page; **C** shared evaluator + cron integration; **D** dream subsystem; **E** heartbeat subsystem. All sequenced after this one.

**Goal:** Introduce the per-user server workspace foundation — one tree per user holding memory/soul/heartbeat/skills/uploads — and a new 11-tool server toolset scoped to it. Removes the `skills` DB table and `users.memory_text`/`users.soul` columns in favor of workspace files.

**Architecture:** Every user gets `{PLEXUS_WORKSPACE_ROOT}/{user_id}/`. Path-validation sandbox (canonicalize + prefix check, no bwrap). Per-user quota cached in memory, with soft-lock at 100%. Seven new file tools (`read_file`/`write_file`/`edit_file`/`delete_file`/`list_dir`/`glob`/`grep`) plus updated `file_transfer` and `message` for server-device semantics. Templates ship in-binary and seed `system_config` at boot; user registration copies the defaults into each new workspace. Skills move to disk-as-truth with a cached frontmatter index.

**Tech stack:** Rust 1.85 (edition 2024), tokio + tokio::fs, axum 0.7, sqlx (PostgreSQL), dashmap, globset, regex, tracing.

**Parent branch:** current `M3-gateway-frontend`, based on the commit that contains account-deletion + graceful-shutdown work (96c3c2f or its successor).

---

## 1. Overview

Today Plexus stores three kinds of per-user data in three incompatible shapes:
- Memory and soul as `TEXT` columns on `users`.
- Skills as rows in the `skills` table, mirrored to disk files under `$PLEXUS_SKILLS_DIR/{user_id}/`.
- Inbound-media uploads as files under `/tmp/plexus-uploads/{user_id}/`, cleaned after 24 h.

This fragmentation blocks dream (needs to read history + edit memory + create skills under one consistent surface), complicates backups, and forces an ephemeral upload model. This plan unifies everything under `{PLEXUS_WORKSPACE_ROOT}/{user_id}/` and gives the agent a minimal, consistent file toolset to operate on it.

Plexus is rebuilding from scratch (see CLAUDE.md); no production data migration is required. Schema changes happen in-place in `db/mod.rs`.

## 2. Goals & Non-Goals

See §2 of the spec. Plan A does **not** cover:
- Frontend Workspace page (Plan B).
- Dream (Plan D), heartbeat (Plan E), shared evaluator (Plan C).
- Timezone-aware UI — the `users.timezone` column is added here but the Settings UI field to edit it lands with Plan B.
- Per-user workspace backup story — admin concern; document in deployment guide in a follow-up.

## 3. File Structure

### New files

| File | Responsibility |
|---|---|
| `plexus-server/src/workspace/mod.rs` | Module entry; re-exports `resolve_user_path`, `QuotaCache`, `WorkspaceError` |
| `plexus-server/src/workspace/paths.rs` | `resolve_user_path`, `resolve_user_path_for_create` — path validation |
| `plexus-server/src/workspace/quota.rs` | `QuotaCache` (DashMap-backed), `check_upload`, `check_write`, `record_delta` |
| `plexus-server/src/workspace/registration.rs` | `initialize_user_workspace(user_id)` — creates tree + copies templates |
| `plexus-server/src/server_tools/file_ops.rs` | The 7 file tools: read/write/edit/delete + list_dir/glob/grep |
| `plexus-server/src/context/skills_cache.rs` | In-process cache of parsed `SKILL.md` frontmatter, keyed by user_id |
| `plexus-server/templates/workspace/SOUL.md` | Baseline soul template |
| `plexus-server/templates/workspace/MEMORY.md` | Baseline memory with section headers |
| `plexus-server/templates/workspace/HEARTBEAT.md` | Baseline heartbeat tasks file with instructions |
| `plexus-server/templates/skills/create_skill/SKILL.md` | Default on-demand skill teaching skill authoring |

### Modified files

| File | Change |
|---|---|
| `plexus-server/src/config.rs` | Add `workspace_root: String`; remove `skills_dir` (workspace_root/{user_id}/skills/ replaces it); add `gateway_upload_max_bytes` |
| `plexus-server/src/db/mod.rs` | Remove `users.memory_text` and `users.soul` from CREATE TABLE; remove `CREATE TABLE skills`; add `users.timezone`; add `cron_jobs.kind TEXT NOT NULL DEFAULT 'user'`; remove the `ALTER TABLE users ADD COLUMN ... memory_text` migration; add system_config boot-seed keys |
| `plexus-server/src/db/skills.rs` | **Delete file** (module removed) |
| `plexus-server/src/db/users.rs` | Remove `update_memory`, `update_soul`; add `update_timezone`, `get_timezone` |
| `plexus-server/src/db/mod.rs` (pub mods) | Remove `pub mod skills;` |
| `plexus-server/src/server_tools/mod.rs` | Rebuild `SERVER_TOOL_NAMES` and `tool_schemas()`; delete `save_memory`/`edit_memory`/`read_skill`/`install_skill` entries; add 7 file-ops; add `read_file`/`write_file`/`edit_file`/`delete_file`/`list_dir`/`glob`/`grep` dispatches |
| `plexus-server/src/server_tools/memory.rs` | **Delete file** |
| `plexus-server/src/server_tools/skills.rs` | **Delete file** |
| `plexus-server/src/server_tools/file_transfer.rs` | Support `from_device="server"` and `to_device="server"` resolving against the user workspace |
| `plexus-server/src/server_tools/message.rs` | `from_device="server"` resolves media paths against the user workspace |
| `plexus-server/src/context.rs` | Read `MEMORY.md` and `SOUL.md` from the user workspace; read skills via `skills_cache` instead of DB |
| `plexus-server/src/auth/mod.rs` (register handler) | After inserting the user row, call `workspace::registration::initialize_user_workspace(user_id)` |
| `plexus-server/src/auth/api.rs` (or wherever soul/memory endpoints live) | Remove `PUT /api/user/soul` + `PUT /api/user/memory` body handlers (they wrote to DB columns). Replace with `PUT /api/workspace/file` pathing to `SOUL.md` / `MEMORY.md` — but the `/api/workspace/*` endpoints are Plan B. In Plan A, temporarily disable the old endpoints (return 410 Gone) so admin UI doesn't silently write dead columns |
| `plexus-server/src/file_store.rs` | Refactor to write uploads to `{workspace_root}/{user_id}/uploads/` instead of `/tmp/plexus-uploads/{user_id}/`; remove the 24h cleanup task |
| `plexus-server/src/main.rs` | Remove `file_store::spawn_cleanup_task` call; add `workspace::initialize_quota_cache` call at boot |
| `plexus-server/src/channels/discord/mod.rs` | Attachment downloads go to workspace uploads path |
| `plexus-server/src/channels/telegram.rs` | Same |
| `plexus-gateway/src/proxy.rs` or similar | Body-limit layer uses `system_config.gateway_upload_max_bytes` instead of `FILE_UPLOAD_MAX_BYTES` |
| `plexus-common/src/consts.rs` | Remove `FILE_UPLOAD_MAX_BYTES` constant |
| `plexus-server/src/account.rs` (added by account-deletion plan) | Update `wipe_file_store` to `remove_dir_all({workspace_root}/{user_id}/)` |

### Tests

| File | Scope |
|---|---|
| `plexus-server/src/workspace/paths.rs` (inline `#[cfg(test)]`) | Path traversal, symlink escape, parent-doesn't-exist, relative resolution |
| `plexus-server/src/workspace/quota.rs` (inline) | Per-upload cap, soft-lock threshold, delta accounting |
| `plexus-server/src/workspace/registration.rs` (inline) | Template copy succeeds, files readable after |
| `plexus-server/src/server_tools/file_ops.rs` (inline) | One test per tool: happy path + sandbox rejection |
| `plexus-server/src/context/skills_cache.rs` (inline) | Frontmatter parse, cache invalidation |
| `plexus-server/tests/workspace_integration.rs` (new) | End-to-end: register user → verify workspace tree → call each file tool → assert results |

## 4. Testing Strategy

- **Unit tests** inline via `#[cfg(test)] mod tests` for pure-function modules (`paths`, `quota`, `skills_cache`).
- **Integration tests** for tool dispatch and registration, using the existing sqlx test-pool pattern. Each integration test is wrapped in `#[sqlx::test]` or the equivalent Plexus pattern; check an existing test file for the setup.
- **No mocks for the filesystem.** Use `tempfile::TempDir` for tests that need an isolated workspace root; each test sets `config.workspace_root` to its own tempdir. This catches canonicalize-related bugs that would be masked by a mocked fs.
- **Quota tests** verify both happy paths and edge cases: exactly-at-100%, exactly-at-80%-per-upload, soft-lock activation, soft-lock release after delete.

## 5. Tasks

21 tasks, all TDD. Each task is one commit.

---

### Task A-1: Add `workspace_root` and `gateway_upload_max_bytes` to server config

**Files:**
- Modify: `plexus-server/src/config.rs`
- Modify: `plexus-server/.env.example`

- [ ] **Step 1: Extend `ServerConfig`**

```rust
// plexus-server/src/config.rs
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub database_url: String,
    pub admin_token: String,
    pub jwt_secret: String,
    pub server_port: u16,
    pub gateway_ws_url: String,
    pub gateway_token: String,
    pub workspace_root: String,     // NEW — replaces skills_dir
}

impl ServerConfig {
    pub fn from_env() -> Self {
        Self {
            database_url: env_required("DATABASE_URL"),
            admin_token: env_required("ADMIN_TOKEN"),
            jwt_secret: env_required("JWT_SECRET"),
            server_port: env_required("SERVER_PORT")
                .parse()
                .expect("SERVER_PORT must be a number"),
            gateway_ws_url: env_required("PLEXUS_GATEWAY_WS_URL"),
            gateway_token: env_required("PLEXUS_GATEWAY_TOKEN"),
            workspace_root: std::env::var("PLEXUS_WORKSPACE_ROOT").unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                format!("{home}/.plexus/workspace")
            }),
        }
    }
}
```

- [ ] **Step 2: Update `.env.example`**

Remove `PLEXUS_SKILLS_DIR=...` line. Add:
```
PLEXUS_WORKSPACE_ROOT=/var/lib/plexus/workspace
```

- [ ] **Step 3: Build to catch direct `config.skills_dir` references**

Run: `cargo build --package plexus-server 2>&1 | head -50`

Every compile error points at a `config.skills_dir` access to replace with `config.workspace_root`. Note each site in a scratchpad — they'll be fixed in later tasks as part of updating the relevant module.

For **this task** only, add a temporary compatibility shim at the bottom of `config.rs`:

```rust
impl ServerConfig {
    /// TEMPORARY: returns `{workspace_root}/{user_id}/skills`.
    /// Removed in Task A-17 once all callers have migrated.
    pub fn legacy_skills_dir_for_user(&self, user_id: &str) -> String {
        format!("{}/{user_id}/skills", self.workspace_root)
    }
}
```

And in every site that was `&state.config.skills_dir`, replace with `&state.config.legacy_skills_dir_for_user(user_id)` so the build stays green.

- [ ] **Step 4: Verify build passes**

Run: `cargo build --package plexus-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add plexus-server/src/config.rs plexus-server/.env.example
git add plexus-server/src/*.rs plexus-server/src/**/*.rs  # shim'd callers
git commit -m "refactor: replace PLEXUS_SKILLS_DIR with PLEXUS_WORKSPACE_ROOT

Introduces the workspace root env var. Adds a temporary
legacy_skills_dir_for_user shim so existing code continues to
build while subsequent tasks migrate skill access to the
workspace-scoped path."
```

---

### Task A-2: Schema updates — additive only (timezone, cron_jobs.kind)

**Plan revision note (2026-04-17):** Originally this task also dropped `users.memory_text`, `users.soul`, and the `skills` table. Those destructive changes have been moved into Task A-17 so the build stays green between every pair of commits through Tasks A-3 … A-16. This task is purely additive.

**Files:**
- Modify: `plexus-server/src/db/mod.rs`

- [ ] **Step 1: Add `timezone` to `users` CREATE TABLE**

Find the `users` CREATE TABLE block in `db/mod.rs`. Leave `soul TEXT` and `memory_text TEXT NOT NULL DEFAULT ''` in place (Task A-17 drops them). Add `timezone TEXT NOT NULL DEFAULT 'UTC',` before `created_at`:

```sql
CREATE TABLE IF NOT EXISTS users (
    user_id        TEXT PRIMARY KEY,
    email          TEXT UNIQUE NOT NULL,
    password_hash  TEXT NOT NULL DEFAULT '',
    is_admin       BOOLEAN DEFAULT FALSE,
    display_name   TEXT,
    soul           TEXT,
    memory_text    TEXT NOT NULL DEFAULT '',
    timezone       TEXT NOT NULL DEFAULT 'UTC',
    created_at     TIMESTAMPTZ DEFAULT NOW()
)
```

Add a migration line below the existing migrations:
```
"ALTER TABLE users ADD COLUMN IF NOT EXISTS timezone TEXT NOT NULL DEFAULT 'UTC'",
```

- [ ] **Step 2: Add `kind` column to `cron_jobs`**

Update the `cron_jobs` CREATE TABLE to include a `kind` column with a CHECK constraint:

```sql
CREATE TABLE IF NOT EXISTS cron_jobs (
    job_id          TEXT PRIMARY KEY,
    user_id         TEXT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL DEFAULT 'user' CHECK (kind IN ('user', 'system')),
    enabled         BOOLEAN DEFAULT TRUE,
    cron_expr       TEXT,
    every_seconds   INTEGER,
    timezone        TEXT DEFAULT 'UTC',
    message         TEXT NOT NULL,
    channel         TEXT NOT NULL,
    chat_id         TEXT NOT NULL,
    delete_after_run BOOLEAN DEFAULT FALSE,
    deliver         BOOLEAN DEFAULT TRUE,
    next_run_at     TIMESTAMPTZ,
    last_run_at     TIMESTAMPTZ,
    run_count       INTEGER DEFAULT 0,
    created_at      TIMESTAMPTZ DEFAULT NOW()
)
```

Add the migration line below the existing `cron_jobs` migrations:
```
"ALTER TABLE cron_jobs ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'user'",
```

- [ ] **Step 3: Verify build + schema**

Run: `cargo build --package plexus-server`
Expected: PASS with zero new warnings.

Start the server briefly to confirm `db::init_db` runs the new `ALTER TABLE` migrations without error (optional — only if you have a local Postgres configured).

- [ ] **Step 4: Commit**

```bash
git add plexus-server/src/db/mod.rs
git commit -m "feat(schema): add users.timezone and cron_jobs.kind columns

Additive schema prep for the workspace-and-autonomy design.
timezone drives heartbeat's local-time evaluator (Plan E).
cron_jobs.kind distinguishes system-protected jobs (dream) from
user jobs (Plan D). Destructive drops land with Task A-17."
```

---

### Task A-3: Path validation helper (`workspace/paths.rs`)

**Files:**
- Create: `plexus-server/src/workspace/mod.rs`
- Create: `plexus-server/src/workspace/paths.rs`

- [ ] **Step 1: Create module skeleton**

```rust
// plexus-server/src/workspace/mod.rs
pub mod paths;
pub mod quota;
pub mod registration;

pub use paths::{resolve_user_path, resolve_user_path_for_create, WorkspaceError};
pub use quota::QuotaCache;
```

In `plexus-server/src/lib.rs` or `main.rs` (wherever `pub mod` declarations live), add:
```rust
pub mod workspace;
```

- [ ] **Step 2: Write failing test for path validation**

```rust
// plexus-server/src/workspace/paths.rs
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("path traversal attempt: {0}")]
    Traversal(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolve a relative user-workspace path against `{workspace_root}/{user_id}/`.
/// Canonicalizes (resolves symlinks) and rejects paths escaping the user root.
/// The target must exist; use `resolve_user_path_for_create` for paths that don't yet exist.
pub async fn resolve_user_path(
    workspace_root: &Path,
    user_id: &str,
    relative: &str,
) -> Result<PathBuf, WorkspaceError> {
    let user_root = workspace_root.join(user_id);
    let joined = user_root.join(relative);
    let canonical = tokio::fs::canonicalize(&joined).await?;
    let user_root_canonical = tokio::fs::canonicalize(&user_root).await?;
    if !canonical.starts_with(&user_root_canonical) {
        return Err(WorkspaceError::Traversal(relative.into()));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_traversal_via_dotdot_rejected() {
        let root = TempDir::new().unwrap();
        let user_dir = root.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        let victim_dir = root.path().join("bob");
        tokio::fs::create_dir_all(&victim_dir).await.unwrap();
        tokio::fs::write(victim_dir.join("secret.txt"), b"secret").await.unwrap();

        let result = resolve_user_path(root.path(), "alice", "../bob/secret.txt").await;
        assert!(matches!(result, Err(WorkspaceError::Traversal(_))));
    }
}
```

- [ ] **Step 3: Run test — expect PASS (straightforward implementation)**

Run: `cargo test --package plexus-server workspace::paths::tests -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Add tests for symlink escape, parent-doesn't-exist, happy path**

Append to `tests`:

```rust
    #[tokio::test]
    async fn test_symlink_escape_rejected() {
        let root = TempDir::new().unwrap();
        let user_dir = root.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        let outside = root.path().join("outside.txt");
        tokio::fs::write(&outside, b"outside").await.unwrap();
        tokio::fs::symlink(&outside, user_dir.join("escape")).await.unwrap();

        let result = resolve_user_path(root.path(), "alice", "escape").await;
        assert!(matches!(result, Err(WorkspaceError::Traversal(_))));
    }

    #[tokio::test]
    async fn test_happy_path_resolves() {
        let root = TempDir::new().unwrap();
        let user_dir = root.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();
        tokio::fs::write(user_dir.join("MEMORY.md"), b"hi").await.unwrap();

        let result = resolve_user_path(root.path(), "alice", "MEMORY.md").await.unwrap();
        assert_eq!(result, user_dir.canonicalize().unwrap().join("MEMORY.md"));
    }
```

- [ ] **Step 5: Add `resolve_user_path_for_create` for paths that don't exist yet**

```rust
/// Same as `resolve_user_path`, but the final component is permitted to not exist.
/// Canonicalizes the deepest existing ancestor and joins the remainder, validating that
/// no component uses `..` to escape after canonicalization.
pub async fn resolve_user_path_for_create(
    workspace_root: &Path,
    user_id: &str,
    relative: &str,
) -> Result<PathBuf, WorkspaceError> {
    let user_root = workspace_root.join(user_id);
    let user_root_canonical = tokio::fs::canonicalize(&user_root).await?;

    let joined = user_root.join(relative);
    let mut ancestor = joined.as_path();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !ancestor.exists() {
        tail.push(ancestor.file_name().ok_or_else(|| WorkspaceError::Traversal(relative.into()))?.to_owned());
        ancestor = ancestor.parent().ok_or_else(|| WorkspaceError::Traversal(relative.into()))?;
    }
    let canonical_ancestor = tokio::fs::canonicalize(ancestor).await?;
    if !canonical_ancestor.starts_with(&user_root_canonical) {
        return Err(WorkspaceError::Traversal(relative.into()));
    }
    let mut result = canonical_ancestor;
    for component in tail.into_iter().rev() {
        if component == std::ffi::OsStr::new("..") || component == std::ffi::OsStr::new(".") {
            return Err(WorkspaceError::Traversal(relative.into()));
        }
        result.push(component);
    }
    Ok(result)
}

#[cfg(test)]
mod create_tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_create_deep_path_allowed() {
        let root = TempDir::new().unwrap();
        let user_dir = root.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let result = resolve_user_path_for_create(root.path(), "alice", "skills/git/SKILL.md").await.unwrap();
        let expected = user_dir.canonicalize().unwrap().join("skills").join("git").join("SKILL.md");
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn test_create_dotdot_rejected() {
        let root = TempDir::new().unwrap();
        let user_dir = root.path().join("alice");
        tokio::fs::create_dir_all(&user_dir).await.unwrap();

        let result = resolve_user_path_for_create(root.path(), "alice", "../etc/passwd").await;
        assert!(matches!(result, Err(WorkspaceError::Traversal(_))));
    }
}
```

- [ ] **Step 6: Run all tests**

Run: `cargo test --package plexus-server workspace::paths`
Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
git add plexus-server/src/workspace/
git commit -m "feat: add workspace path-validation sandbox

resolve_user_path canonicalizes and checks prefix; symlinks
resolving outside the user root are rejected. A separate
resolve_user_path_for_create handles paths whose final segments
do not yet exist."
```

---

### Task A-4: Quota tracking (`workspace/quota.rs`)

**Files:**
- Create: `plexus-server/src/workspace/quota.rs`

- [ ] **Step 1: Write failing test for per-upload cap**

```rust
// plexus-server/src/workspace/quota.rs
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub struct QuotaCache {
    /// user_id -> current usage in bytes
    usage: DashMap<String, Arc<AtomicU64>>,
    /// Total quota per user in bytes.
    quota_bytes: u64,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum QuotaError {
    #[error("upload exceeds per-upload cap ({0} bytes; cap {1} bytes)")]
    UploadTooLarge(u64, u64),
    #[error("workspace is soft-locked (usage {0} > quota {1}); delete files to continue")]
    SoftLocked(u64, u64),
    #[error("upload would exceed hard ceiling ({0} + {1} > {2})")]
    HardCeiling(u64, u64, u64),
}

impl QuotaCache {
    pub fn new(quota_bytes: u64) -> Self {
        Self {
            usage: DashMap::new(),
            quota_bytes,
        }
    }

    pub fn per_upload_cap(&self) -> u64 {
        self.quota_bytes * 4 / 5   // 80%
    }

    fn usage_for(&self, user_id: &str) -> Arc<AtomicU64> {
        self.usage
            .entry(user_id.to_string())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)))
            .clone()
    }

    /// Check an incoming upload. Returns Ok if allowed; reserves the bytes by
    /// incrementing the usage counter atomically.
    pub fn check_and_reserve_upload(&self, user_id: &str, bytes: u64) -> Result<(), QuotaError> {
        if bytes > self.per_upload_cap() {
            return Err(QuotaError::UploadTooLarge(bytes, self.per_upload_cap()));
        }
        let counter = self.usage_for(user_id);
        let current = counter.load(Ordering::SeqCst);
        if current > self.quota_bytes {
            return Err(QuotaError::SoftLocked(current, self.quota_bytes));
        }
        let new_usage = current + bytes;
        // Allow the upload even if it pushes over 100% (grace window);
        // soft-lock activates on the *next* write attempt.
        counter.store(new_usage, Ordering::SeqCst);
        Ok(())
    }

    pub fn record_delete(&self, user_id: &str, bytes_freed: u64) {
        let counter = self.usage_for(user_id);
        counter.fetch_sub(bytes_freed.min(counter.load(Ordering::SeqCst)), Ordering::SeqCst);
    }

    pub fn current_usage(&self, user_id: &str) -> u64 {
        self.usage_for(user_id).load(Ordering::SeqCst)
    }

    pub fn quota_bytes(&self) -> u64 {
        self.quota_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upload_exceeding_per_upload_cap_rejected() {
        let q = QuotaCache::new(5_000_000_000);   // 5 GB
        // cap = 4 GB
        let result = q.check_and_reserve_upload("alice", 4_500_000_000);
        assert!(matches!(result, Err(QuotaError::UploadTooLarge(_, _))));
    }

    #[test]
    fn test_upload_at_per_upload_cap_allowed() {
        let q = QuotaCache::new(5_000_000_000);
        let result = q.check_and_reserve_upload("alice", 4_000_000_000);
        assert!(result.is_ok());
    }

    #[test]
    fn test_grace_window_allows_exceeding_100_percent() {
        let q = QuotaCache::new(5_000_000_000);
        q.check_and_reserve_upload("alice", 4_000_000_000).unwrap();  // 80%
        // Next upload would push to 160% — but allow (grace).
        let result = q.check_and_reserve_upload("alice", 4_000_000_000);
        assert!(result.is_ok());
        assert_eq!(q.current_usage("alice"), 8_000_000_000);
    }

    #[test]
    fn test_soft_lock_after_over_quota() {
        let q = QuotaCache::new(5_000_000_000);
        q.check_and_reserve_upload("alice", 4_000_000_000).unwrap();
        q.check_and_reserve_upload("alice", 4_000_000_000).unwrap();  // now over
        // Any further upload attempt rejects until deletes drop usage.
        let result = q.check_and_reserve_upload("alice", 100);
        assert!(matches!(result, Err(QuotaError::SoftLocked(_, _))));
    }

    #[test]
    fn test_delete_releases_soft_lock() {
        let q = QuotaCache::new(5_000_000_000);
        q.check_and_reserve_upload("alice", 4_000_000_000).unwrap();
        q.check_and_reserve_upload("alice", 4_000_000_000).unwrap();
        assert!(matches!(
            q.check_and_reserve_upload("alice", 100),
            Err(QuotaError::SoftLocked(_, _))
        ));
        q.record_delete("alice", 4_000_000_000);  // drop to 4 GB
        let result = q.check_and_reserve_upload("alice", 100);
        assert!(result.is_ok());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --package plexus-server workspace::quota`
Expected: all PASS.

- [ ] **Step 3: Add the boot-time initialization function**

```rust
// Append to workspace/quota.rs
impl QuotaCache {
    /// Walks the workspace root and primes the usage cache for every existing user dir.
    /// Call once at server startup.
    pub async fn initialize_from_disk(
        &self,
        workspace_root: &std::path::Path,
    ) -> std::io::Result<()> {
        let mut entries = tokio::fs::read_dir(workspace_root).await?;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let user_id = entry.file_name().to_string_lossy().to_string();
            let bytes = walk_dir_bytes(&entry.path()).await?;
            self.usage_for(&user_id).store(bytes, Ordering::SeqCst);
        }
        Ok(())
    }
}

async fn walk_dir_bytes(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let ft = entry.file_type().await?;
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                total += entry.metadata().await?.len();
            }
        }
    }
    Ok(total)
}
```

- [ ] **Step 4: Commit**

```bash
git add plexus-server/src/workspace/quota.rs
git commit -m "feat: per-user workspace quota with soft-lock

QuotaCache tracks usage in memory (DashMap). Per-upload cap is
80% of quota; usage may briefly exceed 100% (grace window) but
any further write attempt is rejected with SoftLocked until
deletes bring usage back below quota."
```

---

### Task A-5: Ship workspace + skill templates

**Files:**
- Create: `plexus-server/templates/workspace/SOUL.md`
- Create: `plexus-server/templates/workspace/MEMORY.md`
- Create: `plexus-server/templates/workspace/HEARTBEAT.md`
- Create: `plexus-server/templates/skills/create_skill/SKILL.md`

- [ ] **Step 1: Write `SOUL.md` baseline**

```markdown
# Soul

You are a helpful assistant. Be concise, direct, and honest.

This is the baseline soul. Edit this file to shape the agent's
personality, tone, and defaults. The agent will adopt whatever is
written here as part of its system prompt on every turn.
```

- [ ] **Step 2: Write `MEMORY.md` baseline with section headers**

```markdown
# Memory

Long-term memory maintained by dream and by the agent during normal
conversations. Stable section headers below give dream predictable
anchors when doing surgical edits.

## User Facts

<!-- Who the user is: name, role, preferences, timezone, relationships. -->

## Active Projects

<!-- Ongoing work the agent is helping with. Delete entries that have completed or gone stale. -->

## Completed

<!-- Recently finished items worth remembering briefly; dream prunes old entries. -->

## Notes

<!-- Free-form observations, preferences, gotchas. -->
```

- [ ] **Step 3: Write `HEARTBEAT.md` baseline**

```markdown
# Heartbeat Tasks

Tasks the agent checks every 30 minutes. Add lines here when you
want the agent to check in on something in the background.

## Format

Each task is one line:

- [ ] <description>  <!-- optional: due: 2026-04-18 10:00 UTC -->

When a task completes, the agent may mark it `- [x]` or remove it.

## Examples

- [ ] Remind me to drink water every day at 3pm
- [ ] Check the deploy status at 9am Monday
- [ ] Summarize unread email each morning
```

- [ ] **Step 4: Write `create_skill/SKILL.md`**

```markdown
---
name: create_skill
description: Create a new reusable skill file in your own workspace.
always_on: false
---

# Creating Skills

When you notice a reusable pattern — a workflow you've done more than
once, a troubleshooting sequence, a set of conventions for a project —
save it as a skill.

## Location

Every skill lives at `skills/{name}/SKILL.md` inside the user's
workspace. The directory name is the skill name.

## Structure

Every SKILL.md starts with YAML frontmatter:

```
---
name: <matches the directory>
description: <one-line summary, shown in the skill index>
always_on: <true or false>
---

<instructions>
```

- `always_on: true` means the full skill content is injected into
  every system prompt. Use sparingly — consumes context budget.
- `always_on: false` means only the name+description is indexed.
  The agent reads the full file via `read_file` when needed.

## When to create one

- A workflow you've repeated 2+ times for the same user.
- A domain they care about (their codebase conventions, their
  writing style, their team's meeting rhythm).
- A troubleshooting procedure worth preserving.

## When NOT to create one

- One-off tasks.
- Things that belong in `MEMORY.md` (facts about the user) rather
  than reusable instructions.
- Work-in-progress — finish the task, then decide if a skill is
  earned.

## How to create one

```
write_file("skills/my_skill_name/SKILL.md", "---\nname: my_skill_name\ndescription: ...\nalways_on: false\n---\n\n...")
```

Use `create_dir` (implicit via `write_file` auto-creating parents).
Pick a short `snake_case` name. Keep descriptions to one line.
```

- [ ] **Step 5: Commit**

```bash
git add plexus-server/templates/
git commit -m "feat: ship workspace and skill templates

Baseline SOUL.md, MEMORY.md, HEARTBEAT.md (workspace root files)
and a default create_skill on-demand skill that teaches the
agent how to author new skills."
```

---

### Task A-6: User registration copies templates into workspace

**Files:**
- Create: `plexus-server/src/workspace/registration.rs`
- Modify: `plexus-server/src/auth/mod.rs` (or wherever `register_handler` lives)

- [ ] **Step 1: Write failing integration test**

```rust
// plexus-server/src/workspace/registration.rs
use crate::db::system_config;
use sqlx::PgPool;
use std::path::Path;

pub async fn initialize_user_workspace(
    pool: &PgPool,
    workspace_root: &Path,
    user_id: &str,
) -> std::io::Result<()> {
    let user_root = workspace_root.join(user_id);
    tokio::fs::create_dir_all(&user_root).await?;
    // 0700 permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&user_root, std::fs::Permissions::from_mode(0o700)).await?;
    }

    // Copy baseline files from system_config (or inline defaults if missing)
    for (key, filename, default) in [
        ("default_soul", "SOUL.md", include_str!("../../templates/workspace/SOUL.md")),
        ("default_memory", "MEMORY.md", include_str!("../../templates/workspace/MEMORY.md")),
        ("default_heartbeat", "HEARTBEAT.md", include_str!("../../templates/workspace/HEARTBEAT.md")),
    ] {
        let content = system_config::get(pool, key).await.unwrap_or_else(|| default.to_string());
        tokio::fs::write(user_root.join(filename), content.as_bytes()).await?;
    }

    // Recursively copy templates/skills/ into the user's skills/
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/skills");
    let dst = user_root.join("skills");
    copy_dir_recursive(&src, &dst).await?;

    // Create uploads/
    tokio::fs::create_dir_all(user_root.join("uploads")).await?;

    Ok(())
}

async fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dst).await?;
    let mut entries = tokio::fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let file_type = entry.file_type().await?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if file_type.is_dir() {
            Box::pin(copy_dir_recursive(&from, &to)).await?;
        } else if file_type.is_file() {
            tokio::fs::copy(&from, &to).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use sqlx::PgPool;

    #[sqlx::test]
    async fn test_initialize_creates_tree(pool: PgPool) {
        let root = TempDir::new().unwrap();
        initialize_user_workspace(&pool, root.path(), "alice").await.unwrap();

        let user_dir = root.path().join("alice");
        assert!(user_dir.join("SOUL.md").exists());
        assert!(user_dir.join("MEMORY.md").exists());
        assert!(user_dir.join("HEARTBEAT.md").exists());
        assert!(user_dir.join("skills/create_skill/SKILL.md").exists());
        assert!(user_dir.join("uploads").exists());

        // Verify template content is present
        let memory = tokio::fs::read_to_string(user_dir.join("MEMORY.md")).await.unwrap();
        assert!(memory.contains("## User Facts"));
    }
}
```

- [ ] **Step 2: Add `db::system_config::get` if it doesn't exist**

Grep `db/system_config.rs` (or wherever system_config helpers live):

```bash
grep -rn "pub async fn get" plexus-server/src/db/
```

If there's no `get(pool, key) -> Option<String>` helper, add one:

```rust
// plexus-server/src/db/system_config.rs (create if missing)
use sqlx::PgPool;

pub async fn get(pool: &PgPool, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM system_config WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

pub async fn upsert(pool: &PgPool, key: &str, value: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO system_config (key, value) VALUES ($1, $2)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 3: Run test**

Run: `cargo test --package plexus-server workspace::registration`
Expected: PASS.

- [ ] **Step 4: Wire into register_handler**

Find the register handler (likely `plexus-server/src/auth/mod.rs`). After `db::users::insert(...)` returns success, call:

```rust
if let Err(e) = crate::workspace::registration::initialize_user_workspace(
    &state.db,
    std::path::Path::new(&state.config.workspace_root),
    &new_user.user_id,
).await {
    tracing::warn!(error = %e, user_id = %new_user.user_id, "failed to initialize workspace");
    // Non-fatal: the registration itself succeeded. The agent may fail on first turn
    // until the workspace exists. Admin can intervene.
}
```

- [ ] **Step 5: Commit**

```bash
git add plexus-server/src/workspace/registration.rs plexus-server/src/auth/mod.rs plexus-server/src/db/system_config.rs
git commit -m "feat: user registration initializes per-user workspace

Copies SOUL.md / MEMORY.md / HEARTBEAT.md from system_config
defaults (or shipped templates if unset) and the create_skill
default skill into each new user's workspace."
```

---

### Task A-7: `read_file` server tool

**Files:**
- Create: `plexus-server/src/server_tools/file_ops.rs`
- Modify: `plexus-server/src/server_tools/mod.rs`

- [ ] **Step 1: Write failing test**

```rust
// plexus-server/src/server_tools/file_ops.rs
use crate::state::AppState;
use crate::workspace::{resolve_user_path, WorkspaceError};
use serde_json::Value;
use std::sync::Arc;

pub async fn read_file(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let path = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return (1, "Missing required parameter: path".into()),
    };

    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let resolved = match resolve_user_path(ws_root, user_id, path).await {
        Ok(p) => p,
        Err(WorkspaceError::Traversal(_)) => return (1, "Path escapes user workspace".into()),
        Err(e) => return (1, format!("Resolve error: {e}")),
    };

    match tokio::fs::read_to_string(&resolved).await {
        Ok(content) => (0, content),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            // Binary file — return size + mime hint instead of raw bytes
            let meta = tokio::fs::metadata(&resolved).await.ok();
            let size = meta.map(|m| m.len()).unwrap_or(0);
            (0, format!("[Binary file, {size} bytes. Use file_transfer to move to a client device.]"))
        }
        Err(e) => (1, format!("Read error: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_file_happy_path() {
        // Use the Plexus test-state helper — grep for how other tool tests construct AppState
        // For now, sketch the assertion:
        // let state = test_app_state_with_workspace(temp_dir);
        // let args = serde_json::json!({"path": "hello.txt"});
        // let (code, out) = read_file(&state, "alice", &args).await;
        // assert_eq!(code, 0);
        // assert_eq!(out, "hello\n");
    }
}
```

- [ ] **Step 2: Look up the AppState test-construction pattern**

Grep:
```bash
grep -rn "test_app_state\|#\[sqlx::test\]" plexus-server/src/ | head -10
```

Use whichever helper other tool tests use. If none exists, add a minimal `test_app_state(workspace_root: &Path, pool: PgPool) -> Arc<AppState>` helper in `plexus-server/src/state.rs` under `#[cfg(test)]`.

- [ ] **Step 3: Flesh out the test with actual assertions**

```rust
#[sqlx::test]
async fn test_read_file_happy_path(pool: PgPool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let user_dir = tmp.path().join("alice");
    tokio::fs::create_dir_all(&user_dir).await.unwrap();
    tokio::fs::write(user_dir.join("hello.txt"), b"hello\n").await.unwrap();

    let state = crate::state::test_app_state(tmp.path(), pool);
    let args = serde_json::json!({"path": "hello.txt"});
    let (code, out) = read_file(&state, "alice", &args).await;
    assert_eq!(code, 0);
    assert_eq!(out, "hello\n");
}

#[sqlx::test]
async fn test_read_file_traversal_rejected(pool: PgPool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let user_dir = tmp.path().join("alice");
    tokio::fs::create_dir_all(&user_dir).await.unwrap();
    let other = tmp.path().join("bob");
    tokio::fs::create_dir_all(&other).await.unwrap();
    tokio::fs::write(other.join("secret"), b"s").await.unwrap();

    let state = crate::state::test_app_state(tmp.path(), pool);
    let args = serde_json::json!({"path": "../bob/secret"});
    let (code, out) = read_file(&state, "alice", &args).await;
    assert_eq!(code, 1);
    assert!(out.contains("escapes"));
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --package plexus-server server_tools::file_ops`
Expected: PASS.

- [ ] **Step 5: Register in `server_tools/mod.rs`**

In the `SERVER_TOOL_NAMES` array add `"read_file"`. In `tool_schemas()` add:

```rust
serde_json::json!({
    "type": "function",
    "function": {
        "name": "read_file",
        "description": "Read a file from your server workspace (relative path).",
        "parameters": {
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        }
    }
}),
```

In `execute()` match arm:

```rust
"read_file" => file_ops::read_file(state, &ctx.user_id, &arguments).await,
```

Don't forget `pub mod file_ops;` at the top of `server_tools/mod.rs`.

- [ ] **Step 6: Commit**

```bash
git add plexus-server/src/server_tools/file_ops.rs plexus-server/src/server_tools/mod.rs plexus-server/src/state.rs
git commit -m "feat: add read_file server tool (workspace-scoped)"
```

---

### Task A-8: `write_file` server tool + quota enforcement

**Files:**
- Modify: `plexus-server/src/server_tools/file_ops.rs`
- Modify: `plexus-server/src/server_tools/mod.rs`

- [ ] **Step 1: Write failing test for happy path + quota reject**

```rust
// Append to file_ops.rs
pub async fn write_file(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let path = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return (1, "Missing required parameter: path".into()),
    };
    let content = match args.get("content").and_then(Value::as_str) {
        Some(c) => c,
        None => return (1, "Missing required parameter: content".into()),
    };

    let bytes = content.as_bytes();
    // Quota check (treat write as an upload of content length)
    if let Err(e) = state.quota.check_and_reserve_upload(user_id, bytes.len() as u64) {
        return (1, format!("Quota: {e}"));
    }

    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let resolved = match crate::workspace::resolve_user_path_for_create(ws_root, user_id, path).await {
        Ok(p) => p,
        Err(crate::workspace::WorkspaceError::Traversal(_)) => {
            state.quota.record_delete(user_id, bytes.len() as u64);  // unreserve
            return (1, "Path escapes user workspace".into());
        }
        Err(e) => {
            state.quota.record_delete(user_id, bytes.len() as u64);
            return (1, format!("Resolve error: {e}"));
        }
    };

    if let Some(parent) = resolved.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            state.quota.record_delete(user_id, bytes.len() as u64);
            return (1, format!("Create dir: {e}"));
        }
    }

    // If overwriting, subtract old size from reservation
    let old_size = tokio::fs::metadata(&resolved).await.map(|m| m.len()).unwrap_or(0);
    if old_size > 0 {
        state.quota.record_delete(user_id, old_size);
    }

    match tokio::fs::write(&resolved, bytes).await {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = tokio::fs::set_permissions(&resolved, std::fs::Permissions::from_mode(0o600)).await;
            }
            // Invalidate skills cache if this was a skill write
            if path.starts_with("skills/") {
                state.skills_cache.invalidate(user_id);
            }
            (0, format!("Wrote {} bytes to {}", bytes.len(), path))
        }
        Err(e) => {
            state.quota.record_delete(user_id, bytes.len() as u64);
            (1, format!("Write error: {e}"))
        }
    }
}

#[cfg(test)]
#[sqlx::test]
async fn test_write_file_creates_parent_dirs(pool: sqlx::PgPool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let user_dir = tmp.path().join("alice");
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    let state = crate::state::test_app_state(tmp.path(), pool);
    let args = serde_json::json!({"path": "a/b/c/x.txt", "content": "hi"});
    let (code, _) = write_file(&state, "alice", &args).await;
    assert_eq!(code, 0);
    assert!(user_dir.join("a/b/c/x.txt").exists());
}

#[sqlx::test]
async fn test_write_file_quota_over_per_upload_cap(pool: sqlx::PgPool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let user_dir = tmp.path().join("alice");
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    // Create a tiny quota: 1000 bytes → per-upload cap 800 bytes
    let state = crate::state::test_app_state_with_quota(tmp.path(), pool, 1000);
    let huge = "x".repeat(900);
    let args = serde_json::json!({"path": "big.txt", "content": huge});
    let (code, out) = write_file(&state, "alice", &args).await;
    assert_eq!(code, 1);
    assert!(out.to_lowercase().contains("quota"));
}
```

- [ ] **Step 2: Add `test_app_state_with_quota` helper**

In `plexus-server/src/state.rs` under `#[cfg(test)]` add:

```rust
pub fn test_app_state_with_quota(workspace_root: &Path, pool: PgPool, quota_bytes: u64) -> Arc<AppState> {
    // Same as test_app_state but with a custom quota.
    // Build the AppState the same way the real main.rs does but with a tempdir root and small quota.
    unimplemented!("wire up the same minimal AppState your test helper already produces")
}
```

Replace `unimplemented!` with the actual construction (copy from `test_app_state`, just swap the quota).

- [ ] **Step 3: Add the `skills_cache` field to `AppState`**

In `plexus-server/src/state.rs`:

```rust
pub struct AppState {
    pub config: ServerConfig,
    pub db: PgPool,
    pub sessions: DashMap<String, SessionHandle>,
    pub devices: DashMap<String, DeviceInfo>,
    // ... existing fields ...
    pub quota: crate::workspace::QuotaCache,     // NEW
    pub skills_cache: crate::context::skills_cache::SkillsCache,  // NEW — Task A-15 adds the impl
}
```

For now, introduce a placeholder `SkillsCache` in `context/skills_cache.rs` (we'll flesh out the real logic in Task A-15):

```rust
// plexus-server/src/context/skills_cache.rs
use dashmap::DashMap;

#[derive(Default)]
pub struct SkillsCache {
    // user_id -> serialized (for now) — the actual schema lands in A-15
    _inner: DashMap<String, ()>,
}

impl SkillsCache {
    pub fn new() -> Self { Self::default() }
    pub fn invalidate(&self, _user_id: &str) { /* no-op placeholder */ }
}
```

Register the placeholder module: in `plexus-server/src/context.rs` or wherever context-related mods are declared, add `pub mod skills_cache;`.

- [ ] **Step 4: Run tests, register tool, commit**

Run: `cargo test --package plexus-server server_tools::file_ops::write_file`
Expected: PASS.

Register in `server_tools/mod.rs`:
- Add `"write_file"` to `SERVER_TOOL_NAMES`.
- Schema:
  ```rust
  serde_json::json!({
      "type": "function",
      "function": {
          "name": "write_file",
          "description": "Write (or overwrite) a file in your server workspace. Creates parent directories as needed. Quota-checked.",
          "parameters": {
              "type": "object",
              "properties": {
                  "path": { "type": "string" },
                  "content": { "type": "string" }
              },
              "required": ["path", "content"]
          }
      }
  }),
  ```
- Dispatch: `"write_file" => file_ops::write_file(state, &ctx.user_id, &arguments).await,`.

Commit:
```bash
git add plexus-server/src/server_tools/file_ops.rs plexus-server/src/server_tools/mod.rs plexus-server/src/state.rs plexus-server/src/context/skills_cache.rs
git commit -m "feat: add write_file server tool with quota enforcement"
```

---

### Task A-9: `edit_file` tool with unique-match semantics

**Files:**
- Modify: `plexus-server/src/server_tools/file_ops.rs`
- Modify: `plexus-server/src/server_tools/mod.rs`

- [ ] **Step 1: Write failing tests for happy + ambiguity**

```rust
pub async fn edit_file(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let path = args.get("path").and_then(Value::as_str).ok_or_else(|| "missing path");
    let old = args.get("old_string").and_then(Value::as_str);
    let new = args.get("new_string").and_then(Value::as_str);
    let (path, old, new) = match (path, old, new) {
        (Ok(p), Some(o), Some(n)) => (p, o, n),
        _ => return (1, "Missing required parameters: path, old_string, new_string".into()),
    };

    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let resolved = match crate::workspace::resolve_user_path(ws_root, user_id, path).await {
        Ok(p) => p,
        Err(crate::workspace::WorkspaceError::Traversal(_)) =>
            return (1, "Path escapes user workspace".into()),
        Err(e) => return (1, format!("Resolve error: {e}")),
    };

    let current = match tokio::fs::read_to_string(&resolved).await {
        Ok(c) => c,
        Err(e) => return (1, format!("Read error: {e}")),
    };

    let count = current.matches(old).count();
    if count == 0 {
        return (1, format!(
            "old_string not found in {path}. Include surrounding context to disambiguate."
        ));
    }
    if count > 1 {
        return (1, format!(
            "old_string appears {count} times in {path}. Include more surrounding context to make the match unique."
        ));
    }

    let updated = current.replacen(old, new, 1);
    let new_size = updated.as_bytes().len() as u64;
    let old_size = current.as_bytes().len() as u64;

    // Quota check the delta (only if growing)
    if new_size > old_size {
        let delta = new_size - old_size;
        if let Err(e) = state.quota.check_and_reserve_upload(user_id, delta) {
            return (1, format!("Quota: {e}"));
        }
    } else if new_size < old_size {
        state.quota.record_delete(user_id, old_size - new_size);
    }

    if let Err(e) = tokio::fs::write(&resolved, updated.as_bytes()).await {
        return (1, format!("Write error: {e}"));
    }
    if path.starts_with("skills/") {
        state.skills_cache.invalidate(user_id);
    }
    (0, format!("Edited {path}"))
}

#[cfg(test)]
#[sqlx::test]
async fn test_edit_file_unique_match_succeeds(pool: sqlx::PgPool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let user_dir = tmp.path().join("alice");
    tokio::fs::create_dir_all(&user_dir).await.unwrap();
    tokio::fs::write(user_dir.join("m.txt"), "hello world\nfoo bar\n").await.unwrap();

    let state = crate::state::test_app_state(tmp.path(), pool);
    let args = serde_json::json!({
        "path": "m.txt",
        "old_string": "foo bar",
        "new_string": "foo baz"
    });
    let (code, _) = edit_file(&state, "alice", &args).await;
    assert_eq!(code, 0);
    let after = tokio::fs::read_to_string(user_dir.join("m.txt")).await.unwrap();
    assert!(after.contains("foo baz"));
}

#[sqlx::test]
async fn test_edit_file_ambiguous_match_errors(pool: sqlx::PgPool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let user_dir = tmp.path().join("alice");
    tokio::fs::create_dir_all(&user_dir).await.unwrap();
    tokio::fs::write(user_dir.join("m.txt"), "abc\nabc\n").await.unwrap();

    let state = crate::state::test_app_state(tmp.path(), pool);
    let args = serde_json::json!({"path": "m.txt", "old_string": "abc", "new_string": "xyz"});
    let (code, out) = edit_file(&state, "alice", &args).await;
    assert_eq!(code, 1);
    assert!(out.contains("2 times") || out.contains("appears"));
}
```

- [ ] **Step 2: Run tests, register tool, commit**

Run: `cargo test --package plexus-server server_tools::file_ops::edit_file`. PASS.

Register `edit_file` in schema + dispatch (schema notes: "Unique-match surgical edit. old_string must appear exactly once.").

Commit:
```bash
git add -u
git commit -m "feat: add edit_file server tool (unique-match semantics)"
```

---

### Task A-10: `delete_file` tool

**Files:**
- Modify: `plexus-server/src/server_tools/file_ops.rs`
- Modify: `plexus-server/src/server_tools/mod.rs`

- [ ] **Step 1: Tests + implementation**

```rust
pub async fn delete_file(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let path = args.get("path").and_then(Value::as_str);
    let recursive = args.get("recursive").and_then(Value::as_bool).unwrap_or(false);
    let path = match path {
        Some(p) => p,
        None => return (1, "Missing required parameter: path".into()),
    };

    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let resolved = match crate::workspace::resolve_user_path(ws_root, user_id, path).await {
        Ok(p) => p,
        Err(crate::workspace::WorkspaceError::Traversal(_)) =>
            return (1, "Path escapes user workspace".into()),
        Err(e) => return (1, format!("Resolve error: {e}")),
    };

    let meta = match tokio::fs::metadata(&resolved).await {
        Ok(m) => m,
        Err(e) => return (1, format!("Stat error: {e}")),
    };

    if meta.is_dir() {
        if !recursive {
            return (1, "Path is a directory. Pass recursive: true to delete it.".into());
        }
        // Compute bytes freed before deleting
        let bytes = crate::workspace::quota::walk_dir_bytes(&resolved).await.unwrap_or(0);
        if let Err(e) = tokio::fs::remove_dir_all(&resolved).await {
            return (1, format!("Delete error: {e}"));
        }
        state.quota.record_delete(user_id, bytes);
    } else {
        let bytes = meta.len();
        if let Err(e) = tokio::fs::remove_file(&resolved).await {
            return (1, format!("Delete error: {e}"));
        }
        state.quota.record_delete(user_id, bytes);
    }

    if path.starts_with("skills/") {
        state.skills_cache.invalidate(user_id);
    }
    (0, format!("Deleted {path}"))
}

#[cfg(test)]
#[sqlx::test]
async fn test_delete_file_frees_quota(pool: sqlx::PgPool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let user_dir = tmp.path().join("alice");
    tokio::fs::create_dir_all(&user_dir).await.unwrap();
    tokio::fs::write(user_dir.join("x.txt"), b"12345").await.unwrap();

    let state = crate::state::test_app_state(tmp.path(), pool);
    state.quota.check_and_reserve_upload("alice", 5).unwrap();
    assert_eq!(state.quota.current_usage("alice"), 5);

    let args = serde_json::json!({"path": "x.txt"});
    let (code, _) = delete_file(&state, "alice", &args).await;
    assert_eq!(code, 0);
    assert_eq!(state.quota.current_usage("alice"), 0);
}

#[sqlx::test]
async fn test_delete_dir_requires_recursive(pool: sqlx::PgPool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let user_dir = tmp.path().join("alice").join("sub");
    tokio::fs::create_dir_all(&user_dir).await.unwrap();

    let state = crate::state::test_app_state(tmp.path(), pool);
    let args = serde_json::json!({"path": "sub"});
    let (code, out) = delete_file(&state, "alice", &args).await;
    assert_eq!(code, 1);
    assert!(out.to_lowercase().contains("recursive"));
}
```

- [ ] **Step 2: Make `walk_dir_bytes` pub(crate)**

In `workspace/quota.rs` change `async fn walk_dir_bytes` → `pub(crate) async fn walk_dir_bytes` so `file_ops` can call it.

- [ ] **Step 3: Run tests, register, commit**

Run: `cargo test --package plexus-server server_tools::file_ops::delete_file`. PASS.

Register in schema + dispatch. Commit.

---

### Task A-11: `list_dir` tool

**Files:**
- Modify: `plexus-server/src/server_tools/file_ops.rs`
- Modify: `plexus-server/src/server_tools/mod.rs`

- [ ] **Step 1: Implementation + test**

```rust
pub async fn list_dir(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let resolved = match crate::workspace::resolve_user_path(ws_root, user_id, path).await {
        Ok(p) => p,
        Err(crate::workspace::WorkspaceError::Traversal(_)) =>
            return (1, "Path escapes user workspace".into()),
        Err(e) => return (1, format!("Resolve error: {e}")),
    };

    let mut entries = match tokio::fs::read_dir(&resolved).await {
        Ok(e) => e,
        Err(e) => return (1, format!("List error: {e}")),
    };

    let mut rows = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        let ft = entry.file_type().await.ok();
        let is_dir = ft.map(|t| t.is_dir()).unwrap_or(false);
        let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
        rows.push(serde_json::json!({
            "name": name,
            "is_dir": is_dir,
            "size_bytes": size,
        }));
    }
    (0, serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".into()))
}

#[cfg(test)]
#[sqlx::test]
async fn test_list_dir_shows_children(pool: sqlx::PgPool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let user_dir = tmp.path().join("alice");
    tokio::fs::create_dir_all(user_dir.join("skills/git")).await.unwrap();
    tokio::fs::write(user_dir.join("x.txt"), b"hi").await.unwrap();

    let state = crate::state::test_app_state(tmp.path(), pool);
    let args = serde_json::json!({"path": "."});
    let (code, out) = list_dir(&state, "alice", &args).await;
    assert_eq!(code, 0);
    assert!(out.contains("x.txt"));
    assert!(out.contains("skills"));
}
```

- [ ] **Step 2: Run, register, commit**

---

### Task A-12: `glob` tool

**Files:**
- Modify: `plexus-server/Cargo.toml` (add `globset`)
- Modify: `plexus-server/src/server_tools/file_ops.rs`
- Modify: `plexus-server/src/server_tools/mod.rs`

- [ ] **Step 1: Add `globset` dep**

In `plexus-server/Cargo.toml` under `[dependencies]`:
```toml
globset = "0.4"
walkdir = "2"
```

- [ ] **Step 2: Implementation + test**

```rust
pub async fn glob(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let pattern = match args.get("pattern").and_then(Value::as_str) {
        Some(p) => p,
        None => return (1, "Missing required parameter: pattern".into()),
    };

    let user_root = std::path::Path::new(&state.config.workspace_root).join(user_id);
    let user_root = match tokio::fs::canonicalize(&user_root).await {
        Ok(p) => p,
        Err(e) => return (1, format!("User root: {e}")),
    };

    let glob = match globset::Glob::new(pattern) {
        Ok(g) => g.compile_matcher(),
        Err(e) => return (1, format!("Bad pattern: {e}")),
    };

    let user_root_clone = user_root.clone();
    let matches = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        for entry in walkdir::WalkDir::new(&user_root_clone).into_iter().filter_map(Result::ok) {
            let rel = entry.path().strip_prefix(&user_root_clone).unwrap_or(entry.path());
            if glob.is_match(rel) {
                out.push(rel.display().to_string());
            }
            if out.len() >= 500 { break; }
        }
        out
    }).await.unwrap_or_default();

    (0, matches.join("\n"))
}

#[cfg(test)]
#[sqlx::test]
async fn test_glob_matches_skills(pool: sqlx::PgPool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let user_dir = tmp.path().join("alice");
    tokio::fs::create_dir_all(user_dir.join("skills/git")).await.unwrap();
    tokio::fs::write(user_dir.join("skills/git/SKILL.md"), b"x").await.unwrap();
    tokio::fs::create_dir_all(user_dir.join("skills/memory")).await.unwrap();
    tokio::fs::write(user_dir.join("skills/memory/SKILL.md"), b"y").await.unwrap();

    let state = crate::state::test_app_state(tmp.path(), pool);
    let args = serde_json::json!({"pattern": "skills/*/SKILL.md"});
    let (code, out) = glob(&state, "alice", &args).await;
    assert_eq!(code, 0);
    assert!(out.contains("git/SKILL.md"));
    assert!(out.contains("memory/SKILL.md"));
}
```

- [ ] **Step 3: Register, commit**

---

### Task A-13: `grep` tool

**Files:**
- Modify: `plexus-server/src/server_tools/file_ops.rs`
- Modify: `plexus-server/src/server_tools/mod.rs`

- [ ] **Step 1: Add `regex` dep if missing**

```toml
regex = "1"
```

- [ ] **Step 2: Implementation + test**

```rust
pub async fn grep(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let pattern = match args.get("pattern").and_then(Value::as_str) {
        Some(p) => p,
        None => return (1, "Missing required parameter: pattern".into()),
    };
    let path_prefix = args.get("path_prefix").and_then(Value::as_str).unwrap_or("");
    let use_regex = args.get("regex").and_then(Value::as_bool).unwrap_or(false);

    let user_root = std::path::Path::new(&state.config.workspace_root).join(user_id);
    let user_root = match tokio::fs::canonicalize(&user_root).await {
        Ok(p) => p,
        Err(e) => return (1, format!("User root: {e}")),
    };
    let search_root = if path_prefix.is_empty() {
        user_root.clone()
    } else {
        match crate::workspace::resolve_user_path(&user_root.parent().unwrap(), user_id, path_prefix).await {
            Ok(p) => p,
            Err(e) => return (1, format!("Path: {e}")),
        }
    };

    let matcher: Box<dyn Fn(&str) -> bool + Send> = if use_regex {
        match regex::Regex::new(pattern) {
            Ok(r) => Box::new(move |line: &str| r.is_match(line)),
            Err(e) => return (1, format!("Bad regex: {e}")),
        }
    } else {
        let needle = pattern.to_string();
        Box::new(move |line: &str| line.contains(&needle))
    };

    let user_root_clone = user_root.clone();
    let results = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        for entry in walkdir::WalkDir::new(&search_root).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_file() { continue; }
            let Ok(content) = std::fs::read_to_string(entry.path()) else { continue; };
            let rel = entry.path().strip_prefix(&user_root_clone).unwrap_or(entry.path());
            for (i, line) in content.lines().enumerate() {
                if matcher(line) {
                    out.push(format!("{}:{}: {}", rel.display(), i + 1, line));
                    if out.len() >= 200 { return out; }
                }
            }
        }
        out
    }).await.unwrap_or_default();

    (0, results.join("\n"))
}

#[cfg(test)]
#[sqlx::test]
async fn test_grep_finds_substring(pool: sqlx::PgPool) {
    let tmp = tempfile::TempDir::new().unwrap();
    let user_dir = tmp.path().join("alice");
    tokio::fs::create_dir_all(&user_dir).await.unwrap();
    tokio::fs::write(user_dir.join("a.md"), "hello\nworld\n").await.unwrap();
    tokio::fs::write(user_dir.join("b.md"), "foo\nbar\n").await.unwrap();

    let state = crate::state::test_app_state(tmp.path(), pool);
    let args = serde_json::json!({"pattern": "world"});
    let (code, out) = grep(&state, "alice", &args).await;
    assert_eq!(code, 0);
    assert!(out.contains("a.md:2"));
    assert!(!out.contains("b.md"));
}
```

- [ ] **Step 3: Register, commit**

---

### Task A-14: Update `file_transfer` to accept `from_device="server"` / `to_device="server"`

**Files:**
- Modify: `plexus-server/src/server_tools/file_transfer.rs`

- [ ] **Step 1: Read the current implementation**

```bash
grep -n 'from_device\|to_device\|file_path' plexus-server/src/server_tools/file_transfer.rs
```

Understand the current flow: it looks up device tokens, uses WS oneshot to fetch/push bytes between two client devices.

- [ ] **Step 2: Add the server branch**

Where `from_device` is resolved, add a branch:

```rust
if from_device == "server" {
    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let resolved = crate::workspace::resolve_user_path(ws_root, user_id, file_path).await
        .map_err(|e| format!("Source path: {e}"))?;
    // Read bytes from the server workspace, send to to_device (existing client-push path)
    let bytes = tokio::fs::read(&resolved).await.map_err(|e| format!("Read: {e}"))?;
    // ... existing "push to to_device" code takes bytes + filename ...
}
```

Same for `to_device == "server"`:

```rust
if to_device == "server" {
    let bytes = /* fetch from from_device using existing WS path */;
    // Quota-check
    state.quota.check_and_reserve_upload(user_id, bytes.len() as u64)
        .map_err(|e| format!("Quota: {e}"))?;
    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let target_rel = format!("uploads/{}", extract_filename(file_path));
    let resolved = crate::workspace::resolve_user_path_for_create(ws_root, user_id, &target_rel).await
        .map_err(|e| format!("Target path: {e}"))?;
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    tokio::fs::write(&resolved, &bytes).await.map_err(|e| format!("Write: {e}"))?;
}
```

Adapt to whatever the existing code looks like — this is a sketch.

- [ ] **Step 3: Test**

Add a test that exercises `from_device="server"` (simplest case — just a local file read + mock client push, or assert that resolve_user_path is called).

- [ ] **Step 4: Commit**

```bash
git add plexus-server/src/server_tools/file_transfer.rs
git commit -m "feat: file_transfer supports server as a device"
```

---

### Task A-15: Update `message` tool to accept `from_device="server"` for media

**Files:**
- Modify: `plexus-server/src/server_tools/message.rs`

- [ ] **Step 1: Read current implementation**

```bash
grep -n 'from_device\|media' plexus-server/src/server_tools/message.rs
```

- [ ] **Step 2: Add server-device branch**

Where `media` paths are resolved, add:

```rust
if from_device.as_deref() == Some("server") {
    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let mut resolved_paths = Vec::new();
    for rel in media.iter() {
        let abs = crate::workspace::resolve_user_path(ws_root, user_id, rel).await
            .map_err(|e| format!("Media {rel}: {e}"))?;
        resolved_paths.push(abs);
    }
    // Pass resolved_paths to the existing channel-adapter attach logic
}
```

Commit.

---

### Task A-16: Skills cache — disk-as-truth, frontmatter-driven

**Files:**
- Rewrite: `plexus-server/src/context/skills_cache.rs`

- [ ] **Step 1: Flesh out the cache type**

```rust
// plexus-server/src/context/skills_cache.rs
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    pub always_on: bool,
    pub path: String,  // relative path under user root, e.g., "skills/git_workflow/SKILL.md"
}

#[derive(Debug, Clone)]
pub struct SkillsBundle {
    pub always_on: Vec<(SkillFrontmatter, String)>,  // (meta, full SKILL.md content)
    pub on_demand: Vec<SkillFrontmatter>,
}

#[derive(Default)]
pub struct SkillsCache {
    entries: DashMap<String, Arc<SkillsBundle>>,
}

impl SkillsCache {
    pub fn new() -> Self { Self::default() }

    pub fn invalidate(&self, user_id: &str) {
        self.entries.remove(user_id);
    }

    pub async fn get_or_load(&self, user_id: &str, workspace_root: &std::path::Path) -> Arc<SkillsBundle> {
        if let Some(existing) = self.entries.get(user_id) {
            return existing.clone();
        }
        let bundle = load_skills(workspace_root, user_id).await;
        let bundle = Arc::new(bundle);
        self.entries.insert(user_id.to_string(), bundle.clone());
        bundle
    }
}

async fn load_skills(workspace_root: &std::path::Path, user_id: &str) -> SkillsBundle {
    let skills_root = workspace_root.join(user_id).join("skills");
    let mut always_on = Vec::new();
    let mut on_demand = Vec::new();

    let Ok(mut entries) = tokio::fs::read_dir(&skills_root).await else {
        return SkillsBundle { always_on, on_demand };
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        if !entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) { continue; }
        let skill_name = entry.file_name().to_string_lossy().to_string();
        let skill_md = entry.path().join("SKILL.md");
        let Ok(content) = tokio::fs::read_to_string(&skill_md).await else { continue; };
        let Ok((name, description, always_on_flag)) = parse_frontmatter(&content) else {
            tracing::warn!(user_id = %user_id, skill = %skill_name, "invalid SKILL.md frontmatter, skipping");
            continue;
        };
        let meta = SkillFrontmatter {
            name: if name.is_empty() { skill_name } else { name },
            description,
            always_on: always_on_flag,
            path: format!("skills/{}/SKILL.md", entry.file_name().to_string_lossy()),
        };
        if always_on_flag {
            always_on.push((meta, content));
        } else {
            on_demand.push(meta);
        }
    }

    SkillsBundle { always_on, on_demand }
}

fn parse_frontmatter(content: &str) -> Result<(String, String, bool), String> {
    let content = content.trim();
    if !content.starts_with("---") {
        return Err("missing frontmatter".into());
    }
    let rest = &content[3..];
    let end = rest.find("---").ok_or("missing closing ---")?;
    let fm = &rest[..end];

    let mut name = String::new();
    let mut description = String::new();
    let mut always_on = false;
    for line in fm.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") { name = v.trim().to_string(); }
        else if let Some(v) = line.strip_prefix("description:") { description = v.trim().to_string(); }
        else if let Some(v) = line.strip_prefix("always_on:") { always_on = v.trim() == "true"; }
    }
    Ok((name, description, always_on))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_load_skills_partitions_by_always_on() {
        let root = TempDir::new().unwrap();
        let skills = root.path().join("alice/skills");
        tokio::fs::create_dir_all(skills.join("a")).await.unwrap();
        tokio::fs::create_dir_all(skills.join("b")).await.unwrap();
        tokio::fs::write(skills.join("a/SKILL.md"),
            "---\nname: a\ndescription: desc a\nalways_on: true\n---\n\nfull a").await.unwrap();
        tokio::fs::write(skills.join("b/SKILL.md"),
            "---\nname: b\ndescription: desc b\nalways_on: false\n---\n\nfull b").await.unwrap();

        let bundle = load_skills(root.path(), "alice").await;
        assert_eq!(bundle.always_on.len(), 1);
        assert_eq!(bundle.always_on[0].0.name, "a");
        assert_eq!(bundle.on_demand.len(), 1);
        assert_eq!(bundle.on_demand[0].name, "b");
    }
}
```

- [ ] **Step 2: Run test**

Run: `cargo test --package plexus-server context::skills_cache`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add plexus-server/src/context/skills_cache.rs
git commit -m "feat: skills cache reads frontmatter from disk

Replaces the DB-backed skills table. Disk is the source of truth;
context builder reads SKILL.md frontmatter via globbed walk with
an in-memory cache keyed by user_id."
```

---

### Task A-17: Remove old tools + context builder reads memory/soul from workspace + destructive schema drops

**Plan revision note (2026-04-17):** This task absorbs the destructive schema drops originally scoped to A-2 (dropping `users.memory_text`, `users.soul`, and the `skills` table, plus deleting `plexus-server/src/db/skills.rs` and removing `pub mod skills;` from `db/mod.rs`). They land here so the build stays green end-to-end.

**Files:**
- Delete: `plexus-server/src/server_tools/memory.rs`
- Delete: `plexus-server/src/server_tools/skills.rs`
- Delete: `plexus-server/src/db/skills.rs`
- Modify: `plexus-server/src/server_tools/mod.rs`
- Modify: `plexus-server/src/context.rs`
- Modify: `plexus-server/src/db/users.rs`
- Modify: `plexus-server/src/db/mod.rs`

- [ ] **Step 1: Remove `save_memory`, `edit_memory`, `read_skill`, `install_skill`**

In `server_tools/mod.rs`:
- Remove `pub mod memory;` and `pub mod skills;`.
- Remove from `SERVER_TOOL_NAMES`.
- Remove from `tool_schemas()`.
- Remove from `execute()` match.

Delete the files:
```bash
git rm plexus-server/src/server_tools/memory.rs
git rm plexus-server/src/server_tools/skills.rs
```

- [ ] **Step 2: Update `db/users.rs`**

Remove `update_memory`, `update_soul`, `get_memory`, `get_soul` functions. Add `update_timezone`, `get_timezone`.

```rust
pub async fn update_timezone(pool: &PgPool, user_id: &str, tz: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE users SET timezone = $1 WHERE user_id = $2")
        .bind(tz)
        .bind(user_id)
        .execute(pool)
        .await
        .map(|_| ())
}

pub async fn get_timezone(pool: &PgPool, user_id: &str) -> sqlx::Result<String> {
    sqlx::query_scalar::<_, String>("SELECT timezone FROM users WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
}
```

- [ ] **Step 3: Update `context.rs` — read MEMORY/SOUL from files**

Find `build_context`. Where it reads `users.soul` and `users.memory_text`:

```rust
// OLD:
// let soul = db::users::get_soul(&state.db, user_id).await?;
// let memory = db::users::get_memory(&state.db, user_id).await?;

// NEW:
let ws_root = std::path::Path::new(&state.config.workspace_root);
let user_root = ws_root.join(user_id);
let soul = tokio::fs::read_to_string(user_root.join("SOUL.md"))
    .await
    .unwrap_or_default();
let memory = tokio::fs::read_to_string(user_root.join("MEMORY.md"))
    .await
    .unwrap_or_default();
```

Where it reads the skills index, replace with:

```rust
let bundle = state.skills_cache
    .get_or_load(user_id, std::path::Path::new(&state.config.workspace_root))
    .await;
// Use bundle.always_on and bundle.on_demand to build the `## Always-On Skills` and
// `## Available Skills` sections exactly as before. The frontmatter fields are the
// same; only the source changed.
```

Remove any `db::skills::*` calls throughout the file.

- [ ] **Step 4: Fix remaining compile errors**

Every other place that called `db::skills`, `db::users::update_memory`, etc. needs updating.

```bash
cargo build --package plexus-server 2>&1 | grep -E "error|warning" | head -30
```

Fix each one. Admin API endpoints that wrote to `users.memory_text` / `users.soul` — return 410 Gone (the Plan B Workspace API replaces them):

```rust
// In auth/api.rs — if there's a PUT /api/user/memory or PUT /api/user/soul handler:
pub async fn update_memory(_: ..., _: ...) -> impl IntoResponse {
    (StatusCode::GONE, "Use PUT /api/workspace/file?path=MEMORY.md (see Plan B).").into_response()
}
```

- [ ] **Step 5: Drop schema — `users.memory_text`, `users.soul`, and `skills` table**

Now that no code references these columns or the `skills` table, it's safe to drop them. In `plexus-server/src/db/mod.rs`:

1. Remove `soul TEXT,` and `memory_text TEXT NOT NULL DEFAULT '',` rows from the `users` CREATE TABLE block.
2. Remove the `"ALTER TABLE users ADD COLUMN IF NOT EXISTS memory_text ..."` migration line if present.
3. Add two new migration lines (idempotent — safe on fresh installs too):
   ```
   "ALTER TABLE users DROP COLUMN IF EXISTS memory_text",
   "ALTER TABLE users DROP COLUMN IF EXISTS soul",
   ```
4. Delete the full `CREATE TABLE IF NOT EXISTS skills (...)` block.
5. Add a new migration line at the end of the statements array:
   ```
   "DROP TABLE IF EXISTS skills",
   ```
6. Delete the `pub mod skills;` declaration at the top of `db/mod.rs`.
7. Delete the file: `git rm plexus-server/src/db/skills.rs`.

- [ ] **Step 6: Run tests**

```bash
cargo build --package plexus-server && cargo test --package plexus-server
```

Some existing tests may reference `save_memory` or `db::skills`. Delete those tests (the functionality is gone; any replacements come in later plans).

- [ ] **Step 7: Drop the `legacy_skills_dir_for_user` shim**

The shim added in Task A-1 is now unused. Remove it from `config.rs` and any remaining call sites (grep `legacy_skills_dir_for_user`).

- [ ] **Step 8: Commit**

```bash
git add -u
git rm plexus-server/src/server_tools/memory.rs plexus-server/src/server_tools/skills.rs plexus-server/src/db/skills.rs
git commit -m "refactor: remove memory/skills tools and DB surfaces

save_memory, edit_memory, read_skill, install_skill all removed.
Agents now edit MEMORY.md / SOUL.md directly via edit_file.
Skills authored via write_file on skills/{name}/SKILL.md.
Skills DB table and users.memory_text + users.soul columns all
dropped; disk is the source of truth for memory, soul, and
skills. Old admin endpoints return 410 Gone; Plan B ships the
/api/workspace/file endpoint as the replacement. The
legacy_skills_dir_for_user shim from Task A-1 is also removed."
```

---

### Task A-18: Channel adapters write uploads to workspace

**Files:**
- Modify: `plexus-server/src/file_store.rs`
- Modify: `plexus-server/src/channels/discord/mod.rs`
- Modify: `plexus-server/src/channels/telegram.rs`
- Modify: `plexus-server/src/main.rs`

- [ ] **Step 1: Refactor `file_store::save_upload` to target the workspace**

```rust
// plexus-server/src/file_store.rs
use crate::state::AppState;
use std::path::PathBuf;
use std::sync::Arc;

pub async fn save_upload(
    state: &Arc<AppState>,
    user_id: &str,
    filename: &str,
    bytes: &[u8],
) -> Result<String, String> {
    // Quota check first — rejects over-cap uploads at the channel edge.
    state.quota.check_and_reserve_upload(user_id, bytes.len() as u64)
        .map_err(|e| format!("{e}"))?;

    let safe_name = sanitize_filename(filename);
    let date = chrono::Utc::now().format("%Y-%m-%d");
    let hash = blake3::hash(bytes).to_hex();
    let short_hash = &hash.as_str()[..8];
    let rel = format!("uploads/{date}-{short_hash}-{safe_name}");

    let ws_root = std::path::Path::new(&state.config.workspace_root);
    let abs = crate::workspace::resolve_user_path_for_create(ws_root, user_id, &rel).await
        .map_err(|e| format!("Resolve: {e}"))?;
    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| format!("Mkdir: {e}"))?;
    }
    tokio::fs::write(&abs, bytes).await.map_err(|e| format!("Write: {e}"))?;
    Ok(rel)  // return relative path, not absolute — the agent references this in messages
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
        .collect()
}
```

Remove any old `/tmp/plexus-uploads/{user_id}/...` path construction. Remove the cleanup task function entirely.

- [ ] **Step 2: Update channel adapters**

In `channels/discord/mod.rs` and `channels/telegram.rs`, where an attachment is downloaded and written:

```rust
// OLD:
// let url = file_store::save_upload_to_tmp(...).await?;

// NEW:
let rel_path = crate::file_store::save_upload(&state, &user_id, &attachment.filename, &bytes).await?;
// The inbound message text now references this relative path.
// For images that flow inline as base64 ImageUrl content blocks, the text block says:
// "User uploaded uploads/{date}-{hash}-{filename} via Discord."
```

Existing base64 image inlining (from the inbound-media spec) stays — the path in the text block is just updated.

- [ ] **Step 3: Remove `spawn_cleanup_task` call**

In `plexus-server/src/main.rs`:

```rust
// Remove:
// file_store::spawn_cleanup_task(state.clone());
```

Also delete the `spawn_cleanup_task` function from `file_store.rs`.

- [ ] **Step 4: Prime the quota cache at boot**

In `main.rs`, after `AppState` construction but before spawning background tasks:

```rust
state.quota.initialize_from_disk(std::path::Path::new(&state.config.workspace_root))
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to prime quota cache");
    });
```

- [ ] **Step 5: Run, fix, commit**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
git add -u
git commit -m "feat: channel adapters write uploads to user workspace

Drops the 24h cleanup task. Files persist as long as the user's
quota allows. Inbound-message text now references the stable
workspace-relative path (e.g., uploads/2026-04-17-abc-foo.pdf)
so subsequent read_file / file_transfer calls by the agent work
naturally."
```

---

### Task A-19: Gateway upload endpoint uses `gateway_upload_max_bytes` + quota

**Files:**
- Modify: `plexus-gateway/src/proxy.rs` (or wherever the body-limit layer lives)
- Modify: `plexus-gateway/src/main.rs`
- Modify: `plexus-gateway/src/state.rs` or config
- Modify: `plexus-common/src/consts.rs`

- [ ] **Step 1: Remove `FILE_UPLOAD_MAX_BYTES` from common**

```bash
grep -rn "FILE_UPLOAD_MAX_BYTES" plexus-common/ plexus-gateway/ plexus-server/
```

Delete the constant from `plexus-common/src/consts.rs`. Every call site in the gateway needs to read the value from server config or `system_config`.

- [ ] **Step 2: Fetch `gateway_upload_max_bytes` at gateway boot**

The gateway already has a REST API call to the server on startup (JWT verification path, etc.). Add a small helper that reads `system_config.gateway_upload_max_bytes` via an existing admin endpoint — or simpler: expose it via a dedicated `GET /api/config/public` endpoint on the server.

For Plan A, simplest is: gateway reads an env var `PLEXUS_GATEWAY_UPLOAD_MAX_BYTES` (default 1 GB). Admin sets both that env var and the `system_config` row in tandem.

```rust
// plexus-gateway/src/config.rs
pub struct GatewayConfig {
    // ... existing fields ...
    pub upload_max_bytes: usize,
}

impl GatewayConfig {
    pub fn from_env() -> Self {
        Self {
            // ...
            upload_max_bytes: std::env::var("PLEXUS_GATEWAY_UPLOAD_MAX_BYTES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1 * 1024 * 1024 * 1024),  // 1 GB
        }
    }
}
```

- [ ] **Step 3: Wire `RequestBodyLimitLayer` to use the new value**

Find `RequestBodyLimitLayer::new(FILE_UPLOAD_MAX_BYTES)` in `plexus-gateway/src/main.rs` and `proxy.rs`. Replace with `RequestBodyLimitLayer::new(config.upload_max_bytes)`.

- [ ] **Step 4: Server-side quota still applies**

When the upload reaches the server via the gateway proxy, the server's `file_store::save_upload` already checks per-user quota. No additional changes needed server-side.

- [ ] **Step 5: Update `.env.example` on gateway**

Add `PLEXUS_GATEWAY_UPLOAD_MAX_BYTES=1073741824`.

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "refactor: replace FILE_UPLOAD_MAX_BYTES with per-channel caps

Gateway uses PLEXUS_GATEWAY_UPLOAD_MAX_BYTES env var (default 1 GB)
for its body-limit layer. Discord and Telegram enforce their
native 25 MB / 20 MB via their own SDKs. Per-user quota on the
server is the final authority."
```

---

### Task A-20: Boot-time system_config seeding

**Files:**
- Modify: `plexus-server/src/main.rs`
- Modify: `plexus-server/src/db/system_config.rs`

- [ ] **Step 1: Add a boot-seed helper**

```rust
// plexus-server/src/db/system_config.rs
pub async fn seed_defaults_if_missing(pool: &PgPool) -> sqlx::Result<()> {
    for (key, default) in [
        ("default_soul", include_str!("../../templates/workspace/SOUL.md")),
        ("default_memory", include_str!("../../templates/workspace/MEMORY.md")),
        ("default_heartbeat", include_str!("../../templates/workspace/HEARTBEAT.md")),
    ] {
        if get(pool, key).await.is_none() {
            upsert(pool, key, default).await?;
        }
    }
    // Integer/boolean keys
    for (key, default) in [
        ("workspace_quota_bytes", "5368709120"),       // 5 GB
        ("heartbeat_interval_seconds", "1800"),        // 30 min (used by Plan E)
        ("dream_enabled", "true"),                     // Plan D
    ] {
        if get(pool, key).await.is_none() {
            upsert(pool, key, default).await?;
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Call at startup**

In `plexus-server/src/main.rs`, after `db::init_db(&pool).await?`:

```rust
db::system_config::seed_defaults_if_missing(&pool).await?;
```

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "feat: seed default system_config keys at boot"
```

---

### Task A-21: Update account-deletion wipe to target workspace

**Files:**
- Modify: `plexus-server/src/account.rs`

- [ ] **Step 1: Update `wipe_file_store`**

```rust
// plexus-server/src/account.rs
async fn wipe_file_store(state: &Arc<AppState>, user_id: &str) {
    let path = std::path::Path::new(&state.config.workspace_root).join(user_id);
    match tokio::fs::remove_dir_all(&path).await {
        Ok(()) => tracing::info!(user_id = %user_id, "workspace wiped"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => { /* never uploaded */ }
        Err(e) => tracing::warn!(user_id = %user_id, error = %e, "failed to wipe workspace"),
    }
}
```

Also remove the user's entry from the quota cache:

```rust
state.quota.forget_user(user_id);  // add this helper to QuotaCache
```

Add to `QuotaCache`:
```rust
pub fn forget_user(&self, user_id: &str) {
    self.usage.remove(user_id);
}
```

- [ ] **Step 2: Commit**

```bash
git add -u
git commit -m "refactor: account deletion wipes user workspace tree

Updates wipe_file_store to remove {workspace_root}/{user_id}
recursively (instead of the old /tmp/plexus-uploads path).
Also forgets the user's quota cache entry."
```

---

## 6. Self-Review Checklist (run before declaring Plan A done)

1. **Every section of spec §3–§6 maps to a task.** Workspace layout, path validation, quota, 11-tool toolset (11 − 4 removed), skills-as-disk, templates, registration, account deletion impact — all present above.
2. **No placeholders.** Scan for "TODO", "TBD", "etc." — only one allowed reference is the "see `file_transfer.rs` current code" guidance in Task A-14, since the exact code depends on the current state of that file.
3. **Types and method signatures consistent.** `QuotaCache::check_and_reserve_upload`, `resolve_user_path`, `SkillsCache::invalidate` names match everywhere they're used.
4. **Plan B's dependencies visible.** Plan A disables `PUT /api/user/memory`/`/soul` endpoints with 410 Gone; Plan B ships `/api/workspace/file` to replace them. That handoff is noted.
5. **Plan C/D/E's dependencies visible.** `cron_jobs.kind` column added here (Plan D needs it); `users.timezone` added here (Plan E needs it); `seed_defaults_if_missing` includes the heartbeat/dream keys Plans D/E will consume.

## 7. Execution Hints

- Tasks A-1 through A-16 are independent enough to batch in small groups during subagent review. A-17 is the big cutover — do this one on its own with an in-depth diff review.
- A-17 will surface dead-reference errors all across the codebase (anything that called `db::skills::*` or `save_memory`). Expect to touch `api.rs`, `ws.rs`, and admin endpoints. All changes are mechanical but high-volume.
- Tests for each file tool exercise the sandbox boundary — make sure you actually run them against a fresh `TempDir` per test, not a shared one.
