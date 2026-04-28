# Cleanup-Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Execute the cleanup-pass design — unify file storage, unify tool contract, drop 410 endpoints + specialty tools, make device config first-class, collapse DB migrations to canonical CREATE TABLE, sweep mechanical drift.

**Architecture:** Workspace is canonical file store. Agent tools share one schema per op, routed by `device_name`. Errors, MIME, matcher, network policy all centralized in `plexus-common`. Device config (workspace_path, shell_timeout_max, ssrf_whitelist, fs_policy, mcp_servers) fully editable via Settings. Server-only tools down to 4 (message, web_fetch, cron, file_transfer). See full design at `docs/superpowers/specs/2026-04-19-cleanup-pass-design.md`.

**Tech Stack:** Rust 1.85 (edition 2024), axum 0.7, tokio, sqlx (PostgreSQL), tokio-util (ReaderStream), ipnetwork, serde, tracing, React 19 + TypeScript, Zustand, Tailwind 4.

**Spec reference:** `docs/superpowers/specs/2026-04-19-cleanup-pass-design.md` (referred to as "spec §N" below).

**Branch:** `M3-gateway-frontend`. No users, no backward compat, DB resets allowed.

---

## Phase 1 — Foundations in plexus-common

Shared modules that everything else depends on. Land these first.

### Task P1.1: Restructure `plexus-common` errors from flat `error.rs` to `errors/` tree

**Files:**
- Read: `plexus-common/src/error.rs` (current)
- Create: `plexus-common/src/errors/mod.rs`
- Create: `plexus-common/src/errors/workspace.rs`
- Create: `plexus-common/src/errors/tool.rs`
- Create: `plexus-common/src/errors/auth.rs`
- Create: `plexus-common/src/errors/protocol.rs`
- Create: `plexus-common/src/errors/mcp.rs`
- Delete: `plexus-common/src/error.rs`
- Modify: `plexus-common/src/lib.rs` — change `pub mod error;` to `pub mod errors;`

- [ ] **Step 1:** Read current `error.rs` to inventory existing types (`ErrorCode` enum, any domain errors). Keep `ErrorCode` — it's the wire discriminant. Move into `errors/mod.rs` unchanged.

- [ ] **Step 2:** Create `errors/workspace.rs` with `WorkspaceError` per spec §11. Fold any `QuotaError` variants into it as `UploadTooLarge { limit: u64, actual: u64 }` and `SoftLocked`. Implement `fn code(&self) -> ErrorCode` on `WorkspaceError`.

- [ ] **Step 3:** Create `errors/tool.rs` with `ToolError` (execution failures, timeouts, device-unreachable, retriable variants). Implement `code()`.

- [ ] **Step 4:** Create `errors/auth.rs` with `AuthError`. Same pattern.

- [ ] **Step 5:** Create `errors/protocol.rs` with `ProtocolError` (WS frame malformed, version mismatch). Same pattern.

- [ ] **Step 6:** Create `errors/mcp.rs` with `McpError` (server unreachable, `SchemaCollision { mcp_server: String, tool: String }`). Same pattern.

- [ ] **Step 7:** Update `plexus-common/src/lib.rs`: replace `pub mod error;` with `pub mod errors;`. Re-export the top-level types used by other crates via `pub use errors::{...};` to minimize downstream churn.

- [ ] **Step 8:** Run `cargo build -p plexus-common` from `Plexus/`. Expected: clean build. Fix any import paths in the error files that refer to the old structure.

- [ ] **Step 9:** Run `cargo check` for the whole workspace. Fix any downstream imports that used `plexus_common::error::*` — change to `plexus_common::errors::*` or rely on the top-level re-exports.

- [ ] **Step 10:** Commit:
```bash
git add -A
git commit -m "cleanup(common): restructure errors into errors/ tree + fold QuotaError"
```

### Task P1.2: Add `plexus-common/src/fuzzy_match.rs` (nanobot-derived multi-level matcher)

**Files:**
- Create: `plexus-common/src/fuzzy_match.rs`
- Modify: `plexus-common/src/lib.rs` — add `pub mod fuzzy_match;`

**Reference source:** `/home/yucheng/Documents/GitHub/nanobot/nanobot/agent/tools/filesystem.py` — functions `_find_match`, `_find_matches`, `_best_fuzzy_window`, `_diagnose_near_match`. Port the logic to Rust.

- [ ] **Step 1:** Write failing tests in `plexus-common/src/fuzzy_match.rs` (TDD). Cover the 4 matching levels:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_single() {
        let r = find_match("foo\nbar\nbaz", "bar").unwrap();
        assert_eq!(r.matched_text, "bar");
        assert_eq!(r.count, 1);
    }

    #[test]
    fn line_trimmed_handles_indentation_drift() {
        let content = "    if x {\n        y();\n    }";
        let old     = "if x {\n    y();\n}";        // different leading indent
        let r = find_match(content, old).unwrap();
        assert_eq!(r.count, 1);
    }

    #[test]
    fn smart_quote_normalization() {
        let content = "say \u{201C}hi\u{201D}";
        let old     = "say \"hi\"";
        let r = find_match(content, old).unwrap();
        assert_eq!(r.count, 1);
    }

    #[test]
    fn multi_match_reports_count() {
        let r = find_match("a b\na b\na b", "a b").unwrap();
        assert_eq!(r.count, 3);
    }

    #[test]
    fn no_match_returns_error_with_diagnosis() {
        let r = find_match("foo\nbar", "xyz");
        assert!(r.is_err());
    }
}
```

- [ ] **Step 2:** Run tests — expected FAIL (types undefined).
```bash
cd Plexus && cargo test -p plexus-common fuzzy_match
```

- [ ] **Step 3:** Implement the matcher. Types:
```rust
pub struct MatchResult {
    pub matched_text: String,   // what was found (may differ from old_text via whitespace/quote normalization)
    pub count: usize,
}

pub struct MatchFailure {
    pub best_ratio: f64,
    pub hints: Vec<String>,
}

pub fn find_match(content: &str, old_text: &str) -> Result<MatchResult, MatchFailure>;
```

Implementation steps (port from nanobot):
- Normalize CRLF → LF in both inputs.
- Level 1: exact substring. `content.match_indices(old_text).collect()`.
- Level 2: if no exact match, line-trim-sliding-window. Split content + old into lines; for each window-of-N-lines in content (where N = old_text.lines().count()), strip leading+trailing whitespace per line on both sides, compare.
- Level 3: normalize curly-to-straight quotes (" → ", ' → ') on both sides, re-run levels 1-2.
- On failure: compute best-similarity window (SequenceMatcher-style ratio) and produce hints.

- [ ] **Step 4:** Run tests — expected PASS.

- [ ] **Step 5:** Add module to lib.rs + commit:
```bash
git add plexus-common/src/fuzzy_match.rs plexus-common/src/lib.rs
git commit -m "common: add fuzzy_match (nanobot-derived multi-level matcher)"
```

### Task P1.3: Add `plexus-common/src/network.rs` (SSRF CIDR block + validate_url)

**Files:**
- Create: `plexus-common/src/network.rs`
- Modify: `plexus-common/src/lib.rs` — add `pub mod network;`
- Modify: `plexus-common/Cargo.toml` — add `ipnet = "2"` if not present

**Reference source:** `/home/yucheng/Documents/GitHub/nanobot/nanobot/security/network.py` lines 10-60.

- [ ] **Step 1:** Write failing tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use ipnet::IpNet;

    #[test]
    fn blocks_rfc1918() {
        assert!(validate_url("http://10.0.0.1/foo", &[]).is_err());
        assert!(validate_url("http://192.168.1.1/", &[]).is_err());
        assert!(validate_url("http://172.16.0.1/", &[]).is_err());
    }

    #[test]
    fn blocks_metadata_endpoint() {
        assert!(validate_url("http://169.254.169.254/meta", &[]).is_err());
    }

    #[test]
    fn allows_public() {
        assert!(validate_url("https://example.com/foo", &[]).is_ok());
    }

    #[test]
    fn whitelist_punches_hole() {
        let wl = vec![IpNet::from_str("10.180.0.0/16").unwrap()];
        assert!(validate_url("http://10.180.1.1/", &wl).is_ok());
        assert!(validate_url("http://10.0.0.1/", &wl).is_err());  // not in whitelist
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(validate_url("file:///etc/passwd", &[]).is_err());
        assert!(validate_url("ftp://example.com/", &[]).is_err());
    }
}
```

- [ ] **Step 2:** Run tests — FAIL.

- [ ] **Step 3:** Implement:
```rust
use ipnet::IpNet;
use std::net::IpAddr;
use std::str::FromStr;
use url::Url;
use crate::errors::protocol::NetworkError;  // or define NetworkError in errors/protocol.rs

pub fn blocked_networks() -> Vec<IpNet> {
    [
        "0.0.0.0/8", "10.0.0.0/8", "100.64.0.0/10", "127.0.0.0/8",
        "169.254.0.0/16", "172.16.0.0/12", "192.168.0.0/16",
        "::1/128", "fc00::/7", "fe80::/10",
    ].iter().map(|s| IpNet::from_str(s).unwrap()).collect()
}

pub fn validate_url(url: &str, whitelist: &[IpNet]) -> Result<(), NetworkError> {
    let parsed = Url::parse(url).map_err(|_| NetworkError::InvalidUrl)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(NetworkError::InvalidScheme);
    }
    let host = parsed.host_str().ok_or(NetworkError::MissingHost)?;
    // Resolve host to IPs (sync for simplicity here; async variant in consumer)
    let ips: Vec<IpAddr> = (host, 0).to_socket_addrs()
        .map_err(|_| NetworkError::ResolutionFailed)?
        .map(|sa| sa.ip())
        .collect();
    for ip in ips {
        if whitelist.iter().any(|n| n.contains(&ip)) { continue; }
        if blocked_networks().iter().any(|n| n.contains(&ip)) {
            return Err(NetworkError::BlockedNetwork(ip));
        }
    }
    Ok(())
}
```

Add `NetworkError` variants to `plexus-common/src/errors/protocol.rs`: `InvalidUrl`, `InvalidScheme`, `MissingHost`, `ResolutionFailed`, `BlockedNetwork(IpAddr)`.

- [ ] **Step 4:** Run tests — PASS. Some tests may need network access for DNS (127.0.0.1, 10.x are literal IPs so no DNS needed — prefer literal IP URLs in tests).

- [ ] **Step 5:** Commit:
```bash
git add plexus-common/
git commit -m "common: add network module (SSRF CIDR block + validate_url)"
```

### Task P1.4: Expand `plexus-common/src/mime.rs` — union of coverage from three sources

**Files:**
- Modify: `plexus-common/src/mime.rs`
- Read: `plexus-server/src/api.rs` around `mime_from_path`
- Read: `plexus-server/src/context.rs` around `mime_from_filename`

- [ ] **Step 1:** Inventory current mappings in all three sites. Take the union — `context.rs` has widest coverage (18 types including heic/heif/audio/video). `api.rs` has some code-file mappings (rs, ts, py). Unique union includes text, source, image, audio, video, pdf, archive, office docs.

- [ ] **Step 2:** Write tests covering the union:
```rust
#[test]
fn comprehensive_coverage() {
    assert_eq!(detect_mime_from_extension("foo.rs"), "text/x-rust");
    assert_eq!(detect_mime_from_extension("foo.png"), "image/png");
    assert_eq!(detect_mime_from_extension("foo.heic"), "image/heic");
    assert_eq!(detect_mime_from_extension("foo.mp4"), "video/mp4");
    assert_eq!(detect_mime_from_extension("foo.md"), "text/markdown");
    assert_eq!(detect_mime_from_extension("foo.pdf"), "application/pdf");
    assert_eq!(detect_mime_from_extension("no_ext"), "application/octet-stream");
}
```

- [ ] **Step 3:** Expand `detect_mime_from_extension` to cover the union. Keep `&'static str` return type (zero-alloc).

- [ ] **Step 4:** Delete the other two sites (will be cleaned up in Phase 3 where `api.rs` and `context.rs` get refactored). Leave a note comment in each file pointing at `plexus_common::mime::detect_mime_from_extension` so the later tasks know to swap.

- [ ] **Step 5:** Run tests:
```bash
cargo test -p plexus-common mime
```

- [ ] **Step 6:** Commit:
```bash
git add plexus-common/src/mime.rs
git commit -m "common(mime): union coverage from server api.rs + context.rs"
```

### Task P1.5: Add `ConfigUpdate` + streaming frames to `plexus-common::protocol`

**Files:**
- Modify: `plexus-common/src/protocol.rs`

- [ ] **Step 1:** Add variants to `ServerToClient` enum (additive):
```rust
ConfigUpdate {
    workspace_path: String,
    shell_timeout_max: u32,
    ssrf_whitelist: Vec<String>,       // CIDR strings
    fs_policy: String,                  // "sandbox" | "unrestricted"
    mcp_servers: serde_json::Value,     // JSONB passthrough
},
ReadStream {
    request_id: String,
    path: String,
},
```

Add to `ClientToServer` enum:
```rust
StreamChunk {
    request_id: String,
    data: Vec<u8>,      // base64 in transit via serde_bytes or similar
    offset: u64,
},
StreamEnd {
    request_id: String,
    total_size: u64,
},
StreamError {
    request_id: String,
    error: String,
},
```

- [ ] **Step 2:** Write serde roundtrip tests for the new variants:
```rust
#[test]
fn config_update_roundtrip() {
    let v = ServerToClient::ConfigUpdate { /* fill */ };
    let s = serde_json::to_string(&v).unwrap();
    let back: ServerToClient = serde_json::from_str(&s).unwrap();
    // assert match
}
```

- [ ] **Step 3:** Run tests — PASS.

- [ ] **Step 4:** Commit:
```bash
git add plexus-common/src/protocol.rs
git commit -m "common(protocol): add ConfigUpdate + streaming frames"
```

---

## Phase 2 — DB Schema Canonical

Collapse the migration soup into one CREATE TABLE statement per table. See spec §8 for full schema.

### Task P2.1: Create `plexus-server/src/db/schema.sql`

**Files:**
- Create: `plexus-server/src/db/schema.sql` (full canonical schema from spec §8.2, verbatim)

- [ ] **Step 1:** Create the file with the complete SQL from spec §8.2 — 8 tables (users, devices, device_tokens, sessions, messages, cron_jobs, discord_configs, telegram_configs, system_config) + indexes. Every FK has `ON DELETE CASCADE` inline. `devices` has `workspace_path`, `shell_timeout_max`, `ssrf_whitelist`, `fs_policy`, `mcp_servers`. `users` has no `soul`, `memory_text`, `ssrf_whitelist`.

- [ ] **Step 2:** Lint the SQL — run `psql --set ON_ERROR_STOP=on -f plexus-server/src/db/schema.sql` against a scratch DB to verify syntax:
```bash
createdb -U postgres plexus_schema_check
psql -U postgres plexus_schema_check -f plexus-server/src/db/schema.sql
dropdb -U postgres plexus_schema_check
```

- [ ] **Step 3:** Commit:
```bash
git add plexus-server/src/db/schema.sql
git commit -m "db: canonical schema.sql (collapses migration soup)"
```

### Task P2.2: Simplify `db/mod.rs::initialize` to load schema.sql

**Files:**
- Modify: `plexus-server/src/db/mod.rs`

- [ ] **Step 1:** Read current `db/mod.rs` to inventory the migration statements. Note the cascade_migrations loop, the 12 ALTER TABLE, 2 DROP COLUMN, 1 DROP TABLE.

- [ ] **Step 2:** Replace the entire body of `initialize()` with:
```rust
pub async fn initialize(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(include_str!("schema.sql")).execute(pool).await?;
    seed_system_config(pool).await?;
    Ok(())
}

async fn seed_system_config(pool: &PgPool) -> Result<(), sqlx::Error> {
    // Insert default keys if not present. Existing key set documented in SCHEMA.md.
    // Keep idempotent via ON CONFLICT DO NOTHING.
    for (key, default) in DEFAULT_SYSTEM_CONFIG {
        sqlx::query("INSERT INTO system_config (key, value) VALUES ($1, $2) ON CONFLICT (key) DO NOTHING")
            .bind(key)
            .bind(default)
            .execute(pool).await?;
    }
    Ok(())
}

const DEFAULT_SYSTEM_CONFIG: &[(&str, &str)] = &[
    ("llm_config", r#"{"model": "claude-opus-4-7"}"#),
    // ... other keys per spec §8.2 footer + existing system_config contents
];
```

- [ ] **Step 3:** Delete the cascade_migrations loop, ALTER TABLE, DROP COLUMN, DROP TABLE statements. Delete the constraint mutation logic at lines 193-194.

- [ ] **Step 4:** Reset a scratch dev DB and run the server startup to verify `initialize()` produces a working schema:
```bash
dropdb --if-exists plexus_dev_cleanup_test
createdb plexus_dev_cleanup_test
DATABASE_URL=postgres://.../plexus_dev_cleanup_test cargo run -p plexus-server 2>&1 | head -40
# Then inspect: psql plexus_dev_cleanup_test -c '\d devices'
```

- [ ] **Step 5:** Run existing ignore-gated integration tests against a fresh DB:
```bash
DATABASE_URL=... cargo test -p plexus-server -- --ignored
```
Expected: passing (schema is compatible with current test fixtures except for columns we've already dropped in design).

- [ ] **Step 6:** Commit:
```bash
git add plexus-server/src/db/mod.rs
git commit -m "db: replace migration soup with include_str!(\"schema.sql\") load"
```

### Task P2.3: Create `plexus-server/scripts/reset-db.sh`

**Files:**
- Create: `plexus-server/scripts/reset-db.sh` (make executable)

- [ ] **Step 1:** Create the script:
```bash
#!/usr/bin/env bash
set -euo pipefail

DB_NAME="${1:-plexus}"

echo "Resetting database: $DB_NAME"
dropdb --if-exists "$DB_NAME"
createdb "$DB_NAME"
psql "$DB_NAME" -c "CREATE EXTENSION IF NOT EXISTS pgcrypto;"
echo "Done. Start the server to load schema.sql."
```

- [ ] **Step 2:** `chmod +x plexus-server/scripts/reset-db.sh`

- [ ] **Step 3:** Test:
```bash
./plexus-server/scripts/reset-db.sh plexus_dev_cleanup_test
```
Expected: no errors.

- [ ] **Step 4:** Commit:
```bash
git add plexus-server/scripts/reset-db.sh
git commit -m "scripts: add reset-db.sh for dev DB rebuilds"
```

### Task P2.4: Sweep code references to dropped columns

**Files:**
- Search: `plexus-server/src/`
- Modify: any file referencing `users.soul`, `users.memory_text`, `users.ssrf_whitelist`, `devices.shell_timeout` (non-max)

- [ ] **Step 1:** Grep for each dropped column:
```bash
cd Plexus/plexus-server/src
grep -rn "\.soul" --include="*.rs"
grep -rn "memory_text" --include="*.rs"
grep -rn "ssrf_whitelist" --include="*.rs" | grep -v device  # users.ssrf_whitelist only
grep -rn "shell_timeout\b" --include="*.rs"  # rename to shell_timeout_max
```

- [ ] **Step 2:** For each hit:
  - If querying/inserting into `users.soul` / `users.memory_text` — delete the query (these endpoints are 410 until Phase 8 deletes them; just unblock compile).
  - If `users.ssrf_whitelist` — delete the struct field + query.
  - If `devices.shell_timeout` — rename to `shell_timeout_max` throughout.

- [ ] **Step 3:** `cargo build -p plexus-server`. Expected: clean.

- [ ] **Step 4:** Commit:
```bash
git add -A
git commit -m "db: sweep code refs to dropped columns (soul/memory_text/ssrf_whitelist) + rename shell_timeout → shell_timeout_max"
```

---

## Phase 3 — `workspace_fs` Service Module

Unify the 3 duplicate write sites into one service. See spec §3.

### Task P3.1: Create `plexus-server/src/workspace/fs.rs` skeleton with public API

**Files:**
- Create: `plexus-server/src/workspace/fs.rs`
- Modify: `plexus-server/src/workspace/mod.rs` — add `pub mod fs;`

- [ ] **Step 1:** Scaffold the struct + function signatures per spec §3.2. Body of every function: `unimplemented!()`. This task is just shape.
```rust
use std::path::PathBuf;
use std::sync::Arc;
use plexus_common::errors::workspace::WorkspaceError;
// imports...

pub struct WorkspaceFs {
    root: PathBuf,
    quota: Arc<crate::workspace::quota::QuotaCache>,
    skills_cache: Arc<crate::skills_cache::SkillsCache>,   // or whatever the existing type is
}

impl WorkspaceFs {
    pub fn new(root: PathBuf, quota: Arc<_>, skills_cache: Arc<_>) -> Self { /* ... */ }

    pub async fn read(&self, user_id: &str, path: &str) -> Result<Vec<u8>, WorkspaceError> { unimplemented!() }
    pub async fn read_stream(&self, user_id: &str, path: &str)
        -> Result<tokio_util::io::ReaderStream<tokio::fs::File>, WorkspaceError> { unimplemented!() }
    pub async fn stat(&self, user_id: &str, path: &str) -> Result<FileStat, WorkspaceError> { unimplemented!() }
    pub async fn write(&self, user_id: &str, path: &str, bytes: &[u8]) -> Result<(), WorkspaceError> { unimplemented!() }
    pub async fn write_stream<R: tokio::io::AsyncRead + Unpin>(
        &self, user_id: &str, path: &str, reader: R, expected_size: u64,
    ) -> Result<(), WorkspaceError> { unimplemented!() }
    pub async fn delete(&self, user_id: &str, path: &str) -> Result<(), WorkspaceError> { unimplemented!() }
    pub async fn delete_prefix(&self, user_id: &str, prefix: &str) -> Result<u64, WorkspaceError> { unimplemented!() }
    pub async fn list(&self, user_id: &str, path: &str) -> Result<Vec<DirEntry>, WorkspaceError> { unimplemented!() }
    pub async fn glob(&self, user_id: &str, pattern: &str, root: &str) -> Result<Vec<String>, WorkspaceError> { unimplemented!() }
    pub async fn grep(&self, user_id: &str, pattern: &str, root: &str, opts: GrepOpts) -> Result<Vec<GrepHit>, WorkspaceError> { unimplemented!() }
    pub fn quota(&self, user_id: &str) -> QuotaSnapshot { unimplemented!() }
    pub async fn wipe_user(&self, user_id: &str) -> Result<(), WorkspaceError> { unimplemented!() }
}

pub struct FileStat { /* path, size, mime, mtime */ }
pub struct DirEntry { /* name, kind (File/Dir), size */ }
pub struct GrepOpts { /* case_insensitive, context_lines, file_type, file_glob */ }
pub struct GrepHit { /* path, line_number, line_content */ }
pub struct QuotaSnapshot { /* used_bytes, limit_bytes */ }
```

- [ ] **Step 2:** `cargo build -p plexus-server`. Expected: clean (unimplemented! is fine at compile time).

- [ ] **Step 3:** Commit:
```bash
git add plexus-server/src/workspace/fs.rs plexus-server/src/workspace/mod.rs
git commit -m "workspace(fs): scaffold service module (unimplemented bodies)"
```

### Task P3.2: Implement `read` / `read_stream` / `stat` with path resolution + escape check

**Files:**
- Modify: `plexus-server/src/workspace/fs.rs`

- [ ] **Step 1:** Write tests first (use `tempfile::tempdir`):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn read_inside_user_root_succeeds() {
        let dir = tempdir().unwrap();
        let fs = WorkspaceFs::new_for_test(dir.path().to_path_buf());
        std::fs::create_dir_all(dir.path().join("alice")).unwrap();
        std::fs::write(dir.path().join("alice/hello.txt"), b"hi").unwrap();

        let bytes = fs.read("alice", "hello.txt").await.unwrap();
        assert_eq!(bytes, b"hi");
    }

    #[tokio::test]
    async fn read_outside_user_root_rejected() {
        let dir = tempdir().unwrap();
        let fs = WorkspaceFs::new_for_test(dir.path().to_path_buf());
        std::fs::create_dir_all(dir.path().join("alice")).unwrap();

        let r = fs.read("alice", "../bob/secret").await;
        assert!(matches!(r, Err(WorkspaceError::PathEscape { .. })));
    }

    #[tokio::test]
    async fn symlink_escape_rejected_and_logged() {
        let dir = tempdir().unwrap();
        let fs = WorkspaceFs::new_for_test(dir.path().to_path_buf());
        std::fs::create_dir_all(dir.path().join("alice")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", dir.path().join("alice/pw")).unwrap();

        let r = fs.read("alice", "pw").await;
        assert!(matches!(r, Err(WorkspaceError::PathEscape { .. })));
    }
}
```

- [ ] **Step 2:** Run tests — FAIL.

- [ ] **Step 3:** Implement `read`, `read_stream`, `stat` + internal `resolve_path(user_id, path) -> Result<PathBuf>` helper. `resolve_path` accepts absolute and relative (spec §3.4), canonicalizes, verifies prefix. Log at `warn!` on escape.

Use `plexus_common::mime::detect_mime_from_extension` for the `FileStat.mime` field.

- [ ] **Step 4:** Run tests — PASS.

- [ ] **Step 5:** Commit:
```bash
git add plexus-server/src/workspace/fs.rs
git commit -m "workspace(fs): implement read/read_stream/stat + path escape check"
```

### Task P3.3: Implement `write` / `write_stream` with quota reserve + rollback + skills invalidation

**Files:**
- Modify: `plexus-server/src/workspace/fs.rs`

- [ ] **Step 1:** Write tests:
```rust
#[tokio::test]
async fn write_within_quota_succeeds_and_reserves() {
    // setup fs with quota cache, write, verify bytes on disk + quota reflects size
}

#[tokio::test]
async fn write_exceeding_quota_rejected_and_rolled_back() {
    // setup with tight quota, attempt oversized write, expect WorkspaceError::Quota(UploadTooLarge),
    // verify file NOT on disk, quota unchanged
}

#[tokio::test]
async fn write_to_skills_subdir_invalidates_cache() {
    // setup with skills_cache stub that records invalidations
    // write to "skills/foo/SKILL.md"
    // assert skills_cache.was_invalidated_for("user_id")
}

#[tokio::test]
async fn attachments_count_against_quota() {
    // write to ".attachments/msg-1/img.png", verify quota usage increases
}
```

- [ ] **Step 2:** Run tests — FAIL.

- [ ] **Step 3:** Implement `write` and `write_stream`. Flow:
  1. Resolve + escape-check path.
  2. `self.quota.check_and_reserve_upload(user_id, expected_size)` — returns error if exceeded.
  3. `tokio::fs::write` (or stream variant).
  4. On I/O error → `self.quota.forget(user_id, expected_size)` + return error.
  5. On success → if resolved path is under `<user_root>/skills/` → `self.skills_cache.invalidate(user_id)`.

- [ ] **Step 4:** Run tests — PASS.

- [ ] **Step 5:** Commit:
```bash
git add plexus-server/src/workspace/fs.rs
git commit -m "workspace(fs): implement write/write_stream with quota + skills cache"
```

### Task P3.4: Implement `delete` / `delete_prefix` / `list` / `wipe_user`

**Files:**
- Modify: `plexus-server/src/workspace/fs.rs`

- [ ] **Step 1:** Write tests covering:
  - `delete` single file → quota decreases, skills_cache invalidates if under `skills/`.
  - `delete_prefix(user_id, ".attachments/")` → deletes all files older than cutoff age (accept age param). Returns bytes reclaimed.
  - `list(user_id, ".")` → returns top-level entries.
  - `wipe_user(alice)` → deletes alice's entire tree, resets quota.

- [ ] **Step 2:** Run tests — FAIL.

- [ ] **Step 3:** Implement. `delete_prefix` signature: `delete_prefix(&self, user_id: &str, prefix: &str, older_than: Option<Duration>) -> Result<u64, WorkspaceError>`. For TTL sweep: `older_than: Some(Duration::from_days(30))` matches spec §2.2.

- [ ] **Step 4:** Run tests — PASS.

- [ ] **Step 5:** Commit:
```bash
git add plexus-server/src/workspace/fs.rs
git commit -m "workspace(fs): implement delete/delete_prefix/list/wipe_user"
```

### Task P3.5: Implement `glob` / `grep`

**Files:**
- Modify: `plexus-server/src/workspace/fs.rs`
- Add dep if missing: `globset` and `grep` (or use `ignore` + `regex`)

- [ ] **Step 1:** Write tests:
```rust
#[tokio::test]
async fn glob_stars_work() {
    // create files: a.rs, b.rs, c.py ; glob("**/*.rs") returns [a.rs, b.rs]
}

#[tokio::test]
async fn grep_finds_pattern_with_context() {
    // create files with pattern; grep returns hits with line + context
}
```

- [ ] **Step 2:** Run tests — FAIL.

- [ ] **Step 3:** Implement using existing crate choices. Port behavior from `server_tools/file_ops.rs` existing glob/grep — that code stays but becomes a thin wrapper in a later task.

- [ ] **Step 4:** Run tests — PASS.

- [ ] **Step 5:** Commit:
```bash
git add plexus-server/src/workspace/fs.rs plexus-server/Cargo.toml
git commit -m "workspace(fs): implement glob + grep"
```

### Task P3.6: Wire `WorkspaceFs` into `AppState`

**Files:**
- Modify: `plexus-server/src/state.rs`
- Modify: `plexus-server/src/main.rs` (construction site)

- [ ] **Step 1:** Add `workspace_fs: Arc<WorkspaceFs>` to `AppState`. Add to every test state constructor.
- [ ] **Step 2:** Construct in `main.rs` from `PLEXUS_WORKSPACE_ROOT` + existing `quota` + `skills_cache`.
- [ ] **Step 3:** `cargo build -p plexus-server`. Fix test constructors that fail — `workspace_fs` must be present in `test_minimal`, etc.
- [ ] **Step 4:** `cargo test -p plexus-server` — passing.
- [ ] **Step 5:** Commit:
```bash
git add plexus-server/src/state.rs plexus-server/src/main.rs
git commit -m "workspace(fs): wire WorkspaceFs into AppState"
```

### Task P3.7: Rewire REST workspace handlers to thin wrappers

**Files:**
- Modify: `plexus-server/src/api.rs` (workspace_quota, workspace_tree, workspace_file_get, workspace_file_put, workspace_file_delete, workspace_upload, workspace_skills)

- [ ] **Step 1:** For each handler, replace its body with a 5-15 line call into `state.workspace_fs.*`. Map `WorkspaceError` variants to HTTP status (existing `ApiError::from(WorkspaceError)` pattern).

- [ ] **Step 2:** Rename route `/api/workspace/file` (singular) → `/api/workspace/files/{path:.*}` (plural + catch-all) per spec §7. Update frontend calls in a later Phase 9 task.

- [ ] **Step 3:** `GET /api/workspace/files/{path}` body uses `workspace_fs::read_stream` + returns `Body::from_stream(reader_stream)` with `Content-Type` from `FileStat.mime`.

- [ ] **Step 4:** Run integration tests (ignore-gated), smoke-test via curl:
```bash
curl -H "Authorization: Bearer $JWT" http://localhost:8080/api/workspace/files/test.txt
```

- [ ] **Step 5:** Commit:
```bash
git add plexus-server/src/api.rs
git commit -m "api: rewire workspace handlers as thin wrappers over workspace_fs"
```

### Task P3.8: Refactor `WorkspaceUploadResult` to typed `outcome`

**Files:**
- Modify: `plexus-server/src/api.rs` (workspace_upload handler + struct def)

- [ ] **Step 1:** Redefine struct per spec §3.5:
```rust
#[derive(Serialize)]
pub struct WorkspaceUploadResult {
    pub filename: String,
    pub outcome: Result<Uploaded, UploadError>,
}
#[derive(Serialize)] pub struct Uploaded { pub path: String, pub size_bytes: u64 }
#[derive(Serialize)] #[serde(tag = "kind")]
pub enum UploadError {
    Quota { remaining: u64 },
    TooLarge,
    Io(String),
}
```

Note: `Result` doesn't serialize cleanly in serde by default — use `#[serde(tag = ...)]` enum:
```rust
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UploadOutcome {
    Success(Uploaded),
    Error(UploadError),
}
```
(Alternative to `Result<_, _>` field; same semantic.)

- [ ] **Step 2:** Update handler to build `UploadOutcome` cases. Remove `format!("ERROR:{filename}")` sentinel.

- [ ] **Step 3:** Commit:
```bash
git add plexus-server/src/api.rs
git commit -m "api(upload): typed outcome enum replaces ERROR:{} sentinel"
```

---

## Phase 4 — File Storage Unification (kill `/api/files`)

See spec §2 and §5.5.

### Task P4.1: Delete `file_store.rs` + POST/GET `/api/files` routes

**Files:**
- Delete: `plexus-server/src/file_store.rs`
- Modify: `plexus-server/src/api.rs` — remove `upload_file`, `download_file` handlers + `POST /api/files`, `GET /api/files/{file_id}` route registrations
- Modify: `plexus-server/src/lib.rs` (or `main.rs`) — remove `mod file_store;`

- [ ] **Step 1:** Confirm no remaining direct callers before deleting:
```bash
grep -rn "file_store::" plexus-server/src/ plexus-client/src/
grep -rn "/api/files" plexus-server/src/ plexus-frontend/src/
```
Expected: some remaining references — those are addressed in P4.2–P4.7 (message.rs, context.rs, channels, frontend). Delete will cascade compile errors; each task below fixes one.

- [ ] **Step 2:** Delete `file_store.rs`, remove its `mod` declaration, delete the two handlers + route lines.

- [ ] **Step 3:** `cargo build -p plexus-server` — EXPECTED TO FAIL. Note the failing files. Pass the list into the next tasks.

- [ ] **Step 4:** Commit (build will be red; this is intentional — next tasks fix it):
```bash
git add -A
git commit -m "api: delete file_store + /api/files routes (build breaks, fixed in P4.2–P4.7)"
```

*Note: the commit-while-red pattern is explicit here because the fix is cross-cutting. If you prefer an atomic green commit, instead combine P4.1–P4.7 into one task — at cost of reviewability.*

### Task P4.2: Refactor server `message` tool to stream from workspace or device

**Files:**
- Modify: `plexus-server/src/server_tools/message.rs`

- [ ] **Step 1:** Read spec §5.1 for the target contract + pseudocode. Attachment schema is `[{ device_name, path }]`. Delivery: `match device_name { "server" => workspace_fs::read_stream, other => open_device_stream }`.

- [ ] **Step 2:** Write retry wrapper per spec pseudocode. 3 attempts, exponential backoff 500ms * 2^n.

- [ ] **Step 3:** Tool JSON schema in the tool definition: update to match spec §5.1.

- [ ] **Step 4:** Remove imports of `file_store` / `/api/files`.

- [ ] **Step 5:** `cargo build -p plexus-server`. Still red on channels — next tasks.

- [ ] **Step 6:** Commit:
```bash
git add plexus-server/src/server_tools/message.rs
git commit -m "server_tools(message): stream attachments with retry; no staging"
```

### Task P4.3: Update channel adapters — Discord, Telegram, Gateway

**Files:**
- Modify: `plexus-server/src/channels/discord.rs`
- Modify: `plexus-server/src/channels/telegram.rs`
- Modify: `plexus-server/src/channels/gateway.rs`

- [ ] **Step 1:** **Discord** — accept `Vec<AttachmentStream>` where `AttachmentStream = { filename, stream: impl Stream<Item = Bytes> }`. Feed into `serenity` `AttachmentType::Bytes` (read full buffer) or `AttachmentType::Reader` (stream). Replace any `/api/files/{id}` URL posting with direct multipart.

- [ ] **Step 2:** **Telegram** — `teloxide::payloads::SendDocument` + `InputFile::memory(Vec<u8>)`. Buffer the stream, then send. For larger files, prefer `SendVideo`/`SendPhoto` based on mime. Use `plexus_common::mime` to classify.

- [ ] **Step 3:** **Gateway** — the outbound frame `OutboundFrame::Message` carries a `MessageBlock[]` where image/file blocks reference `/api/workspace/files/<path>` (server-origin) or `/api/device-stream/<device>/<path>` (device-origin). Browser fetches at render time.

- [ ] **Step 4:** `cargo build -p plexus-server` — should build green now.

- [ ] **Step 5:** Commit:
```bash
git add plexus-server/src/channels/
git commit -m "channels: stream attachments direct; drop /api/files URL references"
```

### Task P4.4: Update `context.rs` media loading via workspace_fs

**Files:**
- Modify: `plexus-server/src/context.rs`

- [ ] **Step 1:** Find the media-loading code around context.rs:89 (currently calls `file_store::load_file()` after stripping `/api/files/` prefix per verification report).

- [ ] **Step 2:** Replace: messages' image content blocks carry `workspace_path` (plus base64 per spec §2.1). For context build, just pass the base64 through unchanged — the model consumes content blocks directly.

- [ ] **Step 3:** For older messages without embedded base64 (if any exist in dev DBs) — read via `workspace_fs::read(user_id, path)` if `workspace_path` is present; else fall back to "[attachment not available]" placeholder.

- [ ] **Step 4:** Remove `mime_from_filename` helper (use `plexus_common::mime::detect_mime_from_extension`).

- [ ] **Step 5:** `cargo build -p plexus-server` — green.

- [ ] **Step 6:** Commit:
```bash
git add plexus-server/src/context.rs
git commit -m "context: load media via workspace_fs; drop file_store dependency"
```

### Task P4.5: Add `GET /api/device-stream/{device_name}/{path:.*}` endpoint

**Files:**
- Modify: `plexus-server/src/api.rs` (add handler + route)

- [ ] **Step 1:** Write handler per spec §5.5:
```rust
async fn device_stream(
    State(state): State<Arc<AppState>>,
    AuthUser(user_id): AuthUser,
    Path((device_name, path)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    // validate device belongs to user
    let device = state.devices.get_by_name(&user_id, &device_name).await?;
    // open WS ReadStream to device
    let stream = state.device_ws.open_read_stream(&device.id, &path).await?;
    // return HTTP streamed response with MIME from path extension
    let mime = plexus_common::mime::detect_mime_from_extension(&path);
    Ok(Response::builder()
        .header("content-type", mime)
        .body(Body::from_stream(stream))?)
}
```

- [ ] **Step 2:** Register route in `api.rs::api_routes()`.

- [ ] **Step 3:** Integration test (ignore-gated — requires real client connection):
```rust
#[tokio::test]
#[ignore]
async fn device_stream_end_to_end() { /* pair a mock client, write a file, fetch via endpoint */ }
```

- [ ] **Step 4:** Commit:
```bash
git add plexus-server/src/api.rs
git commit -m "api: add GET /api/device-stream/{device_name}/{path} (WS relay)"
```

### Task P4.6: Frontend — image-drop uses workspace PUT + embeds base64 in message

**Files:**
- Modify: `plexus-frontend/src/pages/Chat.tsx` (or wherever image-drop lives)
- Modify: `plexus-frontend/src/store/chat.ts` (message construction)

- [ ] **Step 1:** Find the current image-drop handler. Replace POST `/api/files` call with:
```typescript
async function uploadChatImage(file: File, msgId: string): Promise<string> {
    const path = `.attachments/${msgId}/${file.name}`;
    const res = await fetch(`/api/workspace/files/${encodeURIComponent(path)}`, {
        method: 'PUT',
        headers: {
            'Content-Type': file.type,
            'Authorization': `Bearer ${useAuthStore.getState().token}`,
        },
        body: file,
    });
    if (!res.ok) throw new Error(`Upload failed: ${res.status}`);
    return path;
}
```

- [ ] **Step 2:** Message construction: build content blocks with both `workspace_path` and base64:
```typescript
async function fileToBase64(file: File): Promise<string> {
    return new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve((reader.result as string).split(',')[1]);
        reader.onerror = reject;
        reader.readAsDataURL(file);
    });
}

const blocks = [
    { type: 'text', text: userText },
    {
        type: 'image',
        source: { type: 'base64', media_type: file.type, data: await fileToBase64(file) },
        workspace_path: path,
    },
];
```

- [ ] **Step 3:** Message re-render: prefer `workspace_path` URL; fall back to base64 if 404 (spec §2.1). In the image display component:
```tsx
function MessageImage({ block }: { block: ImageBlock }) {
    const [src, setSrc] = useState(
        block.workspace_path ? `/api/workspace/files/${encodeURIComponent(block.workspace_path)}` : null
    );
    const onError = () => setSrc(`data:${block.source.media_type};base64,${block.source.data}`);
    return <img src={src ?? /* fallback directly */} onError={onError} />;
}
```

- [ ] **Step 4:** Start frontend dev server and manually test an image drop end-to-end. Verify:
  - Image lands at `workspace/.attachments/<msg_id>/<filename>` (check Workspace page).
  - Message JSON in network tab has base64 content block.
  - After 30d-TTL simulation (delete file manually), re-rendering shows the base64 fallback.

- [ ] **Step 5:** Commit:
```bash
git add plexus-frontend/src/pages/Chat.tsx plexus-frontend/src/store/chat.ts
git commit -m "frontend(chat): image-drop to workspace PUT + base64 in message"
```

---

## Phase 5 — Tool Unification

See spec §4.

### Task P5.1: Create `plexus-server/src/server_tools/dispatch.rs`

**Files:**
- Create: `plexus-server/src/server_tools/dispatch.rs`
- Modify: `plexus-server/src/server_tools/mod.rs` — `pub mod dispatch;`

- [ ] **Step 1:** Define the dispatcher per spec §4.4. Use an enum for typed file tools:
```rust
pub enum FileTool {
    ReadFile  { device_name: String, path: String, offset: Option<usize>, limit: Option<usize> },
    WriteFile { device_name: String, path: String, content: String },
    EditFile  { device_name: String, path: String, old_text: String, new_text: String, replace_all: bool },
    ListDir   { device_name: String, path: String },
    Glob      { device_name: String, pattern: String, path: Option<String> },
    Grep      { device_name: String, pattern: String, path: Option<String>, /* ... */ },
    Shell     { device_name: String, command: String, working_dir: Option<String>, timeout: Option<u32> },
}

pub async fn dispatch(state: &AppState, user_id: &str, tool: FileTool) -> Result<ToolResult, ToolError> {
    match tool.device_name() {
        "server" => {
            if matches!(tool, FileTool::Shell { .. }) { return Err(ToolError::ServerHasNoShell); }
            run_on_server_workspace(state, user_id, tool).await
        }
        other => forward_to_client(state, user_id, other, tool).await,
    }
}
```

- [ ] **Step 2:** Implement `run_on_server_workspace` using `workspace_fs` for each tool variant. For `edit_file`, use `plexus_common::fuzzy_match::find_match`.

- [ ] **Step 3:** Implement `forward_to_client` sending `ServerToClient::ToolCall` over WS; awaits `ClientToServer::ToolResult`. Timeout = device's `shell_timeout_max` for Shell, 60s for file ops.

- [ ] **Step 4:** Tests (use stub WS channel):
```rust
#[tokio::test]
async fn dispatch_server_read_goes_to_workspace_fs() { /* ... */ }

#[tokio::test]
async fn dispatch_device_forwards_to_ws() { /* ... */ }

#[tokio::test]
async fn dispatch_shell_to_server_rejected() { /* ... */ }
```

- [ ] **Step 5:** Commit:
```bash
git add plexus-server/src/server_tools/dispatch.rs plexus-server/src/server_tools/mod.rs
git commit -m "server_tools: add dispatch module (file tools + device_name routing)"
```

### Task P5.2: Update server file-tool schemas to include `device_name`

**Files:**
- Modify: `plexus-server/src/server_tools/file_ops.rs` (or wherever the server's tool schemas are built)

- [ ] **Step 1:** For each of `edit_file`, `read_file`, `write_file`, `list_dir`, `glob`, `grep`, `shell` — update the JSON Schema emitted to the LLM to include `device_name` (spec §4.3). The enum is populated at schema-build time from the user's `devices` list + "server".

- [ ] **Step 2:** Update the tool handler in `file_ops.rs` to call `dispatch::dispatch(state, user_id, FileTool::Variant{...})` instead of directly executing. Each handler becomes a 5-line shim that constructs the typed variant.

- [ ] **Step 3:** Remove the direct filesystem logic (path resolve, quota, write-to-disk) from `file_ops.rs` — it all moved to `workspace_fs` via dispatch.

- [ ] **Step 4:** `cargo build -p plexus-server` — green.

- [ ] **Step 5:** Commit:
```bash
git add plexus-server/src/server_tools/file_ops.rs
git commit -m "server_tools(file_ops): schemas gain device_name; handlers delegate to dispatch"
```

### Task P5.3: Delete specialty memory/skill tools

**Files:**
- Modify: `plexus-server/src/server_tools/mod.rs` (remove save_memory, edit_memory, read_skill, install_skill definitions)
- Modify: agent loop's tool registry (wherever the 8 server tools were listed)

- [ ] **Step 1:** Grep to confirm no residual references:
```bash
grep -rn "save_memory\|edit_memory\|read_skill\|install_skill" plexus-server/src/
```

- [ ] **Step 2:** Delete each tool's JSON Schema + handler function + tool-registry entry. Post-cleanup, server-only tools are exactly: `message`, `web_fetch`, `cron`, `file_transfer` (spec §5).

- [ ] **Step 3:** `cargo build` — expect some cleanup (unused imports).

- [ ] **Step 4:** Commit:
```bash
git add -A
git commit -m "server_tools: delete save_memory/edit_memory/read_skill/install_skill (workspace fs replaces them)"
```

### Task P5.4: Update client tool schemas — drop input_schema, adopt shared matcher

**Files:**
- Modify: `plexus-client/src/tools/edit_file.rs`
- Modify: `plexus-client/src/tools/read_file.rs`
- Modify: `plexus-client/src/tools/write_file.rs`
- Modify: `plexus-client/src/tools/list_dir.rs`
- Modify: `plexus-client/src/tools/glob.rs`
- Modify: `plexus-client/src/tools/grep.rs`
- Modify: `plexus-client/src/tools/shell.rs`
- Modify: `plexus-client/src/tools/mod.rs` (registry)

- [ ] **Step 1:** For each tool, remove `input_schema()` and `description()` methods. Client only registers capability NAMES (not schemas) — spec §4.5. Update the tool trait to not require schema methods.

- [ ] **Step 2:** In `edit_file::execute`, replace the existing fuzzy logic with `plexus_common::fuzzy_match::find_match`. Normalize arg extraction: accept `path` (not `file_path`), `old_text` (not `old_string`), `new_text` (not `new_string`), `replace_all`.

- [ ] **Step 3:** For read/write/list/glob/grep, confirm arg names are `path` (already verified). No change expected; audit only.

- [ ] **Step 4:** `cargo build -p plexus-client` — green.

- [ ] **Step 5:** Commit:
```bash
git add plexus-client/src/tools/
git commit -m "client(tools): drop input_schema; adopt shared fuzzy_match; normalize args"
```

### Task P5.5: Create `plexus-server/src/mcp/wrap.rs` + schema collision check

**Files:**
- Create: `plexus-server/src/mcp/wrap.rs`
- Modify: `plexus-server/src/mcp/mod.rs` (create if not exists; register module)

- [ ] **Step 1:** Implement per spec §4.6–4.7:
```rust
pub fn wrap_mcp_tool(
    mcp_server_name: &str,
    raw_schema: Value,
    install_sites: &[String],   // ["server", "linux-devbox", ...]
) -> Value {
    let mut schema = raw_schema.clone();
    // Prefix name
    let tool_name = schema["name"].as_str().unwrap();
    schema["name"] = json!(format!("mcp_{mcp_server_name}_{tool_name}"));
    // Inject device_name into properties + required
    let props = schema["parameters"]["properties"].as_object_mut().unwrap();
    props.insert("device_name".into(), json!({
        "type": "string",
        "enum": install_sites,
        "description": "Where this MCP runs."
    }));
    let req = schema["parameters"]["required"].as_array_mut().unwrap();
    req.push(json!("device_name"));
    schema
}

pub fn check_mcp_schema_collision(
    existing: &[McpInstall],
    incoming: &McpInstall,
) -> Result<(), McpError> {
    for e in existing {
        if e.mcp_server_name == incoming.mcp_server_name {
            for (tool_name, incoming_schema) in &incoming.tools {
                if let Some(existing_schema) = e.tools.get(tool_name) {
                    if existing_schema != incoming_schema {
                        return Err(McpError::SchemaCollision { /* diff body per spec §4.6 */ });
                    }
                }
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 2:** Tests:
```rust
#[test]
fn wrap_injects_device_name_and_prefixes() { /* ... */ }

#[test]
fn collision_detected_on_schema_mismatch() { /* ... */ }

#[test]
fn no_collision_when_same_schema() { /* ... */ }
```

- [ ] **Step 3:** Commit:
```bash
git add plexus-server/src/mcp/
git commit -m "mcp: wrap module (device_name injection + collision check)"
```

### Task P5.6: Wire MCP collision check into `PUT /api/devices/{name}/mcp` + `PUT /api/server-mcp`

**Files:**
- Modify: `plexus-server/src/auth/device.rs` (put_mcp handler)
- Modify: `plexus-server/src/auth/admin.rs` (put_server_mcp handler)

- [ ] **Step 1:** In both handlers, before writing to DB:
  1. For each incoming MCP config, fetch its tool list (or read from submitted payload if client sends tools).
  2. Gather all existing installs of the same `mcp_server_name` (across server and other devices of same user).
  3. Run `check_mcp_schema_collision` — return 409 Conflict with structured body on collision.
  4. Proceed with DB write.

- [ ] **Step 2:** Integration test (ignore-gated):
```rust
#[tokio::test] #[ignore]
async fn put_mcp_conflicts_with_server_install() { /* ... */ }
```

- [ ] **Step 3:** Commit:
```bash
git add plexus-server/src/auth/
git commit -m "mcp: enforce schema-collision check on PUT /api/devices/.../mcp + /api/server-mcp"
```

---

## Phase 6 — Server-Only Tools (details)

### Task P6.1: Update `file_transfer` to 4-case matrix

**Files:**
- Modify: `plexus-server/src/server_tools/file_transfer.rs`

- [ ] **Step 1:** Rewrite tool per spec §5.2. Args: `from_device, from_path, to_device, to_path`. Dispatch matrix:
  - server↔server: `workspace_fs::copy` (add helper in P3 if not present; simple read+write).
  - server→device: `workspace_fs::read_stream` + WS `write_stream`.
  - device→server: WS `read_stream` + `workspace_fs::write_stream`.
  - device↔device: WS `read_stream` (source) → relay → WS `write_stream` (dest).

- [ ] **Step 2:** Retry policy — up to 3 retries on retriable errors. Non-retriable (quota, permission) fail immediately.

- [ ] **Step 3:** Tests with stub WS.

- [ ] **Step 4:** Commit:
```bash
git add plexus-server/src/server_tools/file_transfer.rs
git commit -m "server_tools(file_transfer): 4-case matrix + retry"
```

### Task P6.2: Harden `web_fetch` — drop per-user whitelist, unconditional block

**Files:**
- Modify: `plexus-server/src/server_tools/web_fetch.rs`

- [ ] **Step 1:** Import `plexus_common::network::validate_url`. Call with `whitelist = &[]` (empty). If the current code reads `users.ssrf_whitelist` — delete that.

- [ ] **Step 2:** Update tool description (spec §5.3).

- [ ] **Step 3:** Test: ensure 10.x, 192.168.x, 169.254.169.254 all return error.

- [ ] **Step 4:** Commit:
```bash
git add plexus-server/src/server_tools/web_fetch.rs
git commit -m "server_tools(web_fetch): hardcoded SSRF block; drop per-user whitelist"
```

---

## Phase 7 — Device Config First-Class

### Task P7.1: Rename `/api/devices/{name}/policy` → `/api/devices/{name}/config`

**Files:**
- Modify: `plexus-server/src/auth/device.rs`
- Modify: `plexus-frontend/src/**` (Settings.tsx, device store)

- [ ] **Step 1:** Rename handler function: `get_policy` → `get_config`, `patch_policy` → `patch_config`. Update route path.

- [ ] **Step 2:** Update frontend fetches.

- [ ] **Step 3:** Commit:
```bash
git add -A
git commit -m "api: rename /api/devices/{name}/policy → /config"
```

### Task P7.2: Extend `PATCH /api/devices/{name}/config` to accept all editable fields

**Files:**
- Modify: `plexus-server/src/auth/device.rs` (patch_config)

- [ ] **Step 1:** Accept partial payload:
```rust
#[derive(Deserialize, Default)]
pub struct PatchDeviceConfig {
    pub workspace_path:     Option<String>,
    pub shell_timeout_max:  Option<u32>,
    pub ssrf_whitelist:     Option<Vec<String>>,
    pub fs_policy:          Option<String>,
}
```

- [ ] **Step 2:** Validate:
  - `workspace_path` — must be absolute, non-empty.
  - `shell_timeout_max` — 10 ≤ x ≤ 1800.
  - `ssrf_whitelist` — each entry parses as `IpNet`.
  - `fs_policy` — must be `"sandbox"` or `"unrestricted"`.
  - Any failure → 422 with field-keyed error body.

- [ ] **Step 3:** On success, emit `ServerToClient::ConfigUpdate` to the device's current WS session (if online).

- [ ] **Step 4:** Integration test.

- [ ] **Step 5:** Commit:
```bash
git add plexus-server/src/auth/device.rs
git commit -m "api: PATCH /api/devices/{name}/config accepts all editable fields + pushes ConfigUpdate"
```

### Task P7.3: Client applies `ConfigUpdate` (reconnects on workspace_path change)

**Files:**
- Modify: `plexus-client/src/**` (the WS frame handler + client state)

- [ ] **Step 1:** On `ConfigUpdate` receipt: replace in-memory config fields. If `workspace_path` differs from previous, log and trigger reconnect.

- [ ] **Step 2:** Re-apply bwrap jail root on next tool invocation (if that's where it's established) — implementation-specific to existing bwrap plumbing.

- [ ] **Step 3:** Commit:
```bash
git add plexus-client/src/
git commit -m "client: apply ConfigUpdate; reconnect on workspace_path change"
```

### Task P7.4: Structured device-status block in system prompt

**Files:**
- Modify: `plexus-server/src/context.rs` (render_device_status or equivalent)

- [ ] **Step 1:** Replace current ad-hoc workspace_path echo with structured block per spec §6.4:
```rust
fn render_device_status(devices: &[Device]) -> String {
    let mut out = String::from("## Your targets\n\n### server\nworkspace_root: ...\n\n");
    for d in devices {
        out.push_str(&format!(
            "### {} ({})\nworkspace_root: {}\nshell_timeout_max: {}s\nssrf_whitelist: {}\nfs_policy: {}\nmcp_servers: {}\n\n",
            d.name, last_seen_str(d), d.workspace_path, d.shell_timeout_max,
            if d.ssrf_whitelist.is_empty() { "(none; default RFC-1918 block applies)".into() }
            else { d.ssrf_whitelist.join(", ") },
            d.fs_policy,
            d.mcp_names().join(", ").or_else("(none)")
        ));
    }
    out
}
```

- [ ] **Step 2:** Snapshot-test against an expected output format.

- [ ] **Step 3:** Commit:
```bash
git add plexus-server/src/context.rs
git commit -m "context: structured device-status block in system prompt"
```

### Task P7.5: Settings.tsx — expand device config editor

**Files:**
- Modify: `plexus-frontend/src/pages/Settings.tsx` (devices tab)

- [ ] **Step 1:** For each device row, when expanded: editable inputs for `workspace_path`, `shell_timeout_max`, `ssrf_whitelist` (multi-input, CIDR-validated live), `fs_policy` (toggle).

- [ ] **Step 2:** `fs_policy` flip to `unrestricted`: typed-confirmation modal — user types device name to confirm. Matches account-deletion pattern (already in codebase).

- [ ] **Step 3:** Save button calls `PATCH /api/devices/{name}/config` with only changed fields.

- [ ] **Step 4:** Visual QA in browser.

- [ ] **Step 5:** Commit:
```bash
git add plexus-frontend/src/pages/Settings.tsx
git commit -m "frontend(settings): expanded device config editor + unrestricted-fs-policy modal"
```

---

## Phase 8 — API Surface Deletes (remaining)

### Task P8.1: Delete soul/memory 410 handlers

**Files:**
- Modify: `plexus-server/src/api.rs` (delete get_soul, patch_soul, get_memory, patch_memory + their route lines)

- [ ] **Step 1:** Delete handler functions and route `.route("/api/user/soul", ...)` and `.route("/api/user/memory", ...)` lines.

- [ ] **Step 2:** Grep to confirm no remaining frontend callers:
```bash
grep -rn "/api/user/soul\|/api/user/memory" plexus-frontend/src/
```
Expected: zero hits (already cleaned in B-15).

- [ ] **Step 3:** Commit:
```bash
git add plexus-server/src/api.rs
git commit -m "api: delete /api/user/soul + /api/user/memory (410 handlers gone)"
```

### Task P8.2: Delete `plexus-server/src/auth/skills_api.rs`

**Files:**
- Delete: `plexus-server/src/auth/skills_api.rs`
- Modify: `plexus-server/src/auth/mod.rs` — remove `pub mod skills_api;`
- Modify: `plexus-server/src/main.rs` — remove `skills_api_routes()` merge

- [ ] **Step 1:** Delete the file, remove mod declaration, remove route merge.

- [ ] **Step 2:** `cargo build` — green.

- [ ] **Step 3:** Commit:
```bash
git add -A
git commit -m "api: delete skills_api.rs + its route merge"
```

### Task P8.3: Delete `/api/admin/default-soul`

**Files:**
- Modify: `plexus-server/src/auth/admin.rs`

- [ ] **Step 1:** Delete `get_default_soul`, `put_default_soul` handlers + route line.

- [ ] **Step 2:** If `system_config` has a `default_soul` key seeded — remove it from `DEFAULT_SYSTEM_CONFIG` in `db/mod.rs`.

- [ ] **Step 3:** Commit:
```bash
git add -A
git commit -m "admin: delete /api/admin/default-soul (soul retired)"
```

---

## Phase 9 — Frontend Additional Cleanup

### Task P9.1: Workspace.tsx — .attachments/ collapse + MEMORY.md inline render

**Files:**
- Modify: `plexus-frontend/src/pages/Workspace.tsx`

- [ ] **Step 1:** Tree-view filter: hide `.attachments/` from the default listing; render as a collapsed "Attachments (N)" entry that expands on click.

- [ ] **Step 2:** When user clicks `MEMORY.md`: fetch via `GET /api/workspace/files/MEMORY.md`, render as full markdown (use existing `react-markdown`). "Edit" button switches to textarea + PUT on save.

- [ ] **Step 3:** Same for `skills/<name>/SKILL.md`.

- [ ] **Step 4:** Visual QA.

- [ ] **Step 5:** Commit:
```bash
git add plexus-frontend/src/pages/Workspace.tsx
git commit -m "frontend(workspace): .attachments/ collapse + inline MEMORY.md + SKILL.md render"
```

### Task P9.2: Admin.tsx — add Server MCPs tab

**Files:**
- Modify: `plexus-frontend/src/pages/Admin.tsx`

- [ ] **Step 1:** New tab between Users and the rest. Fetches `GET /api/server-mcp`. List each MCP with name, transport, url. "Add MCP" button → modal (name, transport, url, env key/value pairs). "Remove" button per MCP.

- [ ] **Step 2:** Save via `PUT /api/server-mcp` (replace-all). Surface 409 collision errors inline.

- [ ] **Step 3:** Commit:
```bash
git add plexus-frontend/src/pages/Admin.tsx
git commit -m "frontend(admin): Server MCPs tab"
```

---

## Phase 10 — Mechanical Sweep

### Task P10.1: state.rs prompt fields `Arc<RwLock<String>>` → `Arc<str>`

**Files:**
- Modify: `plexus-server/src/state.rs`
- Modify: callsites: `plexus-server/src/dream.rs`, `plexus-server/src/heartbeat.rs`, `plexus-server/src/context.rs`

- [ ] **Step 1:** Change field types. Init via `Arc::from(s)`.
- [ ] **Step 2:** Grep for `.read().await` on these three fields; replace with direct `.as_ref()` (since `Arc<str>` derefs to `&str`).
- [ ] **Step 3:** `cargo build` + `cargo test` — green.
- [ ] **Step 4:** Commit:
```bash
git add -A
git commit -m "state: prompt fields Arc<RwLock<String>> → Arc<str> (load-once-at-boot)"
```

### Task P10.2: Delete unused `update_timezone`

**Files:**
- Modify: `plexus-server/src/db/users.rs`

- [ ] **Step 1:** Confirm zero callers:
```bash
grep -rn "update_timezone" plexus-server/src/ plexus-client/src/
```

- [ ] **Step 2:** Delete the function.

- [ ] **Step 3:** Commit:
```bash
git add plexus-server/src/db/users.rs
git commit -m "db(users): delete unused update_timezone"
```

### Task P10.3: Remove `PLEXUS_SKILLS_DIR` from env + docs

**Files:**
- Modify: `.env` (if present in repo)
- Modify: `plexus-server/README.md`
- Modify: `plexus-server/docs/DEPLOYMENT.md`

- [ ] **Step 1:** Grep for `PLEXUS_SKILLS_DIR`, remove every line referencing it. Replace with `PLEXUS_WORKSPACE_ROOT` where contextually relevant.

- [ ] **Step 2:** Commit:
```bash
git add -A
git commit -m "docs: remove PLEXUS_SKILLS_DIR from env + README + DEPLOYMENT"
```

### Task P10.4: ErrorCode + consts audit

**Files:**
- Modify: `plexus-common/src/errors/mod.rs` (ErrorCode enum)
- Modify: `plexus-common/src/consts.rs`

- [ ] **Step 1:** For each ErrorCode variant, grep usage:
```bash
for v in $(awk '/enum ErrorCode/,/^}/' plexus-common/src/errors/mod.rs | grep -oE '^\s+[A-Z][a-zA-Z]+' | tr -d ' ,'); do
  count=$(grep -rn "ErrorCode::$v" --include="*.rs" Plexus/ | wc -l)
  echo "$v: $count"
done
```
Delete variants with 0 callers.

- [ ] **Step 2:** Same for `consts.rs` — grep each exported const, delete zero-use.

- [ ] **Step 3:** Commit:
```bash
git add plexus-common/src/
git commit -m "common: audit-delete zero-use ErrorCode variants + consts"
```

### Task P10.5: SECURITY.md audit

**Files:**
- Modify: `plexus-server/docs/SECURITY.md`

- [ ] **Step 1:** Read the doc. Flag references to: `soul`, memory-as-endpoint, three-tier FsPolicy, per-user ssrf_whitelist.

- [ ] **Step 2:** Update to current state: two-tier FsPolicy (sandbox/unrestricted), server `web_fetch` hardcoded block, per-device ssrf_whitelist.

- [ ] **Step 3:** Commit:
```bash
git add plexus-server/docs/SECURITY.md
git commit -m "docs(security): align with post-cleanup state (two-tier FsPolicy, hardcoded SSRF)"
```

### Task P10.6: README.md + DEPLOYMENT.md post-cleanup pass

**Files:**
- Modify: root `README.md`
- Modify: `plexus-server/README.md`
- Modify: `plexus-server/docs/DEPLOYMENT.md`

- [ ] **Step 1:** Grep for `/api/files`, `/api/user/soul`, `/api/user/memory`, `/api/skills/*`. Remove any mention.

- [ ] **Step 2:** Add a paragraph explaining the unified file model (workspace canonical, `.attachments/` TTL, no `/api/files`) and the `device_name` tool routing pattern.

- [ ] **Step 3:** Commit:
```bash
git add README.md plexus-server/README.md plexus-server/docs/DEPLOYMENT.md
git commit -m "docs: README + DEPLOYMENT post-cleanup refresh"
```

### Task P10.7: Dead-import sweep

**Files:** all

- [ ] **Step 1:**
```bash
cd Plexus && cargo build --all 2>&1 | grep "warning: unused import"
```

- [ ] **Step 2:** Fix every unused import flagged.

- [ ] **Step 3:** Run `cargo fmt --all`.

- [ ] **Step 4:** Run `cargo clippy --all -- -D warnings` — fix any clippy warnings (should be minor).

- [ ] **Step 5:** Commit:
```bash
git add -A
git commit -m "cleanup: dead-import sweep + cargo fmt + clippy"
```

---

## Final Verification

### Task F1: Full build + test matrix

- [ ] `cd Plexus && cargo build --all` — green
- [ ] `cargo test --all` — all non-ignored tests pass
- [ ] `DATABASE_URL=... cargo test --all -- --ignored` — all ignored DB tests pass
- [ ] `cd plexus-frontend && npm run typecheck` — clean
- [ ] `cd plexus-frontend && npm run build` — clean
- [ ] Manual smoke:
  - Start server, gateway, frontend, pair a device
  - Send a chat message with a drag-dropped image (verify workspace file + base64 in message)
  - Agent edit_file on server and on device (verify dispatcher routes correctly)
  - Device config edit (workspace_path, shell_timeout_max, ssrf_whitelist, fs_policy) — ConfigUpdate applied
  - Admin adds a server-side MCP; user adds same-named MCP with different schema to a device — 409 conflict
  - Account deletion end-to-end (WS kick + workspace wipe + DB cascade)

### Task F2: Post-cleanup ISSUE.md update

- [ ] Move "full cleanup pass" item from Open to Closed.
- [ ] Log any discovered items as new Open/Deferred.
- [ ] Commit:
```bash
git add plexus-server/docs/ISSUE.md
git commit -m "docs(issue): cleanup-pass complete"
```

---

## Rough Ordering + Parallelism Notes

- **P1** tasks are mostly independent — can dispatch in parallel (P1.1, P1.2, P1.3, P1.4, P1.5).
- **P2** depends on P1.1 (errors). P2.1 → P2.2 → P2.3 → P2.4 sequential.
- **P3** depends on P1 (errors + mime). P3.1–P3.8 mostly sequential within phase.
- **P4** depends on P3 (workspace_fs must exist). Compile-red pattern in P4.1 then fixed by P4.2–P4.7.
- **P5** depends on P3 (fuzzy_match) and P1 (schemas). P5.1 → P5.2 → P5.3 → P5.4 → P5.5 → P5.6 sequential.
- **P6** depends on P5 + P1.3 (network). Sequential inside phase.
- **P7** depends on P5 (dispatch) + P1.5 (ConfigUpdate frame). Sequential.
- **P8** depends on nothing new — can run in parallel with later P9/P10.
- **P9** depends on P3/P4 frontend changes landing first.
- **P10** last; sweep across everything.

Estimated total: ~40 tasks, each 15-40min. Calendar time: 2-3 focused days end-to-end via subagent-driven-development with reviews.

---

## Scope Sanity Check

Every spec section maps to a task:

| Spec § | Task(s) |
|---|---|
| §2 File storage | P4.1–P4.6 |
| §3 workspace_fs | P3.1–P3.8 |
| §4 Tool contract | P5.1–P5.6 |
| §5 Server-only tools | P4.2 (message), P6.1 (file_transfer), P6.2 (web_fetch), P4.5 (/api/device-stream) |
| §6 Device config | P7.1–P7.5 |
| §7 API surface | distributed across P4, P7, P8 |
| §8 DB schema | P2.1–P2.4 |
| §9 Mechanical sweep | P10.1–P10.7 |
| §10 Frontend | P4.6, P7.5, P9.1, P9.2 |
| §11 Errors in common | P1.1 |
| §12 Tests | covered in each task |
| §13 Delta | tracked via commit log |
| §14 Non-goals | not implemented (by design) |

Coverage confirmed.
