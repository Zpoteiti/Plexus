# Workspace Frontend + REST API Implementation Plan (Plan B of 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Spec reference:** The full design lives at `/home/yucheng/Documents/GitHub/Plexus/docs/superpowers/specs/2026-04-17-workspace-and-autonomy-design.md` §7. Read it if this plan's context seems incomplete.
>
> **This is Plan B of 5.** Prior plans: **A** (workspace foundation, `e6f1da4..2fe90a0`), **C** (shared evaluator + cron integration, `2464692..6643b0c`), **D** (dream subsystem, `5be59f9..8f60648`), **E** (heartbeat subsystem, `0f4d681..994b5a3`). This is the LAST plan in the autonomy-and-workspace rewrite set. Remaining after this: the M2 closeout backlog (account deletion, admin user-management, graceful-shutdown extension, unread badge).

**Goal:** Ship a `/settings/workspace` React page that lets each user browse, view, edit, upload, and delete files in their own `{WORKSPACE_ROOT}/{user_id}/` tree, plus the 7 REST endpoints the page consumes. Remove the two Settings sections (Soul, Memory) that have been broken since Plan A-17 returned 410 Gone.

**Architecture:** Backend first — add 7 workspace endpoints under `plexus-server/src/api.rs`, each using Plan A's `resolve_user_path` + `QuotaCache` so the server-side sandbox invariant is enforced once for both the agent's file tools and the user's frontend access. Frontend is a single page with a left tree pane, a right content pane with type-specific renderers (markdown view/edit, plain-text edit, image inline, binary download), a top quota bar with amber/red thresholds, drag-and-drop upload, and a quick-access sidebar for SOUL.md / MEMORY.md / HEARTBEAT.md. Settings.tsx is pared down to its still-working tabs; the Skills tab rewires from the dropped `/api/skills` shape to the new `/api/workspace/skills` endpoint.

**Tech Stack:** Rust 1.85 (edition 2024), axum 0.7, tokio, sqlx (no new queries — reuses Plan A helpers), walkdir; React 19 + TypeScript (strict), Tailwind 4 via `@tailwindcss/vite`, Zustand 5, React Router 7.5, Lucide icons, `react-markdown` (NEW — add to deps), pnpm.

**Parent branch:** current `M3-gateway-frontend`, based on commit `994b5a3` (Plan E's final polish).

---

## 1. Overview

Plan B is the frontend counterpart to Plan A's workspace-foundation overhaul. The agent now reads and writes a per-user workspace tree through server tools; Plan B gives humans the same capability through the browser. Everything at `{WORKSPACE_ROOT}/{user_id}/` — `SOUL.md`, `MEMORY.md`, `HEARTBEAT.md`, `skills/{name}/SKILL.md`, `uploads/*`, plus anything the agent wrote via `write_file` — becomes viewable, editable, and deletable with the same path-validation sandbox the agent uses.

Four bundled pieces:

1. **Seven REST endpoints** on plexus-server, all under `/api/workspace/*`. JWT-gated, path-validated, quota-aware. The gateway proxies them transparently (no gateway-side changes needed — its generic handler already forwards `/api/*` after JWT validation).

2. **`pages/Workspace.tsx`** — a new full-page React component. Left pane: collapsible tree + quick-access sidebar. Right pane: content renderer dispatched on MIME type. Top: quota bar with amber/red thresholds. Drag-and-drop upload anywhere on the page.

3. **Settings cleanup.** The Soul + Memory sections in `Settings.tsx::ProfileTab` (which have been hitting `410 Gone` since Plan A-17) are removed. The Skills tab rewires from `/api/skills` (GET → 410, POST install → 410) to `/api/workspace/skills` (GET parsed frontmatter).

4. **One new shared component.** `ConfirmModal` — used by the workspace page for delete confirmation. Small enough to be a primitive, not a library.

After Plan B lands, the autonomy-and-workspace rewrite (A → C → D → E → B) is fully realized: backend state, agent toolset, autonomous subsystems, and the human-facing UI are all consistent with the workspace-as-one-tree design.

## 2. Goals & Non-Goals

**Goals**

- A user can browse, view, edit, upload, and delete any file in their own `{WORKSPACE_ROOT}/{user_id}/` tree via a single web page.
- Quota usage is visible at all times. Soft-lock state (usage > 100%) surfaces an explicit message; amber warning at 80%, red at 95%.
- Markdown files render as rich text by default; an "Edit" toggle flips to a plain textarea editor. Save writes via `PUT /api/workspace/file`.
- Images (png/jpg/gif/webp) display inline. Other binaries show metadata + a download button; no preview attempt.
- Drag-and-drop upload anywhere on the page. Multi-file supported. Per-file progress.
- SOUL.md / MEMORY.md / HEARTBEAT.md are one click away from the top of the tree (quick-access sidebar).
- The Settings page stops 410-ing on Soul / Memory / skills-install flows — users can successfully edit their soul/memory via the Workspace page's quick-access.
- Delete is confirmation-gated (modal). Recursive delete (for directories) is allowed only when the user has explicitly accepted the recursive-delete warning.
- All paths are validated server-side via `resolve_user_path` (Plan A) — the agent and the frontend use the same sandbox enforcement function.
- No frontend trusts the server for auth: every HTTP call attaches the JWT from `useAuthStore`.

**Non-Goals**

- **Rename.** The spec lists rename as a UI action (§7.2), but §7.3's endpoint list has no dedicated rename route. Client-side rename via "upload-to-new + delete-old" is fine for small text files but is silly for large binaries. Plan B defers rename entirely — users can delete + re-upload. Tracked as a deferred item in ISSUE.md.
- **File-system search.** The workspace tree is flat enough per user (< a few thousand files in typical use) that a search box is not a priority. Use `grep` through the agent if needed.
- **Multi-file select / bulk delete.** Individual file operations only.
- **Frontend test harness.** `plexus-frontend` has no Vitest / Jest / RTL / Playwright today. Plan B does not introduce one. Manual smoke + visual verification is the test plan; full automation is a separate post-M2 effort noted in ISSUE.md.
- **Admin user-management UI.** Part of the M2 closeout backlog, not Plan B.
- **Inline tree-level operations** (right-click context menus, drag-to-move, duplicate). The tree is click-to-select + tree-of-file-entries. Operations happen in the content pane or on dedicated buttons.
- **Live collaborative editing.** Single-user single-session; if two tabs open the same file, last-save-wins. This is an acceptable trade for M2.
- **End-to-end file-type coverage.** Only the 4 image formats listed (png, jpg, gif, webp) render inline. SVG, HEIC, PDF, video are metadata-only (download).

## 3. Design

### 3.1 REST endpoints (plexus-server)

All endpoints live in `plexus-server/src/api.rs`, all require a valid JWT (via the existing `Claims` extractor), and all resolve paths through `crate::workspace::resolve_user_path` / `resolve_user_path_for_create`. Quota enforcement via `state.quota.check_and_reserve_upload` / `release_after_delete`.

```
GET    /api/workspace/quota              -> { used_bytes: u64, total_bytes: u64 }
GET    /api/workspace/tree               -> [{ path, is_dir, size_bytes, modified_at }]
GET    /api/workspace/file?path=...      -> raw bytes (Content-Type sniffed from extension)
PUT    /api/workspace/file?path=...      -> 204 (body = raw content, quota-checked)
DELETE /api/workspace/file?path=...&recursive=bool -> 204
POST   /api/workspace/upload             -> [{ path, size_bytes }]  (multipart, multi-file)
GET    /api/workspace/skills             -> [{ name, description, always_on }]
```

**Path convention:** the `path` query param is relative to `{WORKSPACE_ROOT}/{user_id}/`. Leading slashes are rejected. An empty or `.` path means "the user root".

**Error shape:** all endpoints return JSON `{ "error": "message" }` on failure. Quota errors return `413 Payload Too Large`. Traversal attempts return `403 Forbidden` with a generic message (no path echo). Not-found returns `404`. Soft-lock writes return `413` with a specific message mentioning deletion as the resolution.

**`/api/workspace/tree` payload:**

```json
[
  { "path": "SOUL.md",                    "is_dir": false, "size_bytes": 420,  "modified_at": "2026-04-18T10:00:00Z" },
  { "path": "skills",                      "is_dir": true,  "size_bytes": 0,    "modified_at": "2026-04-18T10:00:00Z" },
  { "path": "skills/create_skill",         "is_dir": true,  "size_bytes": 0,    "modified_at": "2026-04-18T10:00:00Z" },
  { "path": "skills/create_skill/SKILL.md","is_dir": false, "size_bytes": 1234, "modified_at": "2026-04-18T10:00:00Z" },
  { "path": "uploads",                     "is_dir": true,  "size_bytes": 0,    "modified_at": "2026-04-18T10:00:00Z" }
]
```

Flat list (not nested) — the frontend builds the tree. Entries sorted alphabetically with directories before files. Bounded implicitly by per-user quota (several thousand entries max on a 5 GB quota filled with small files).

**`/api/workspace/upload` (multipart):** accepts multiple files. Each file is stored at `uploads/{YYYY-MM-DD}-{hash}-{filename}` (same pattern channel adapters already use). Returns an array of `{ path, size_bytes }` for the files that succeeded. On per-file quota failure, that file is skipped and returned with `{ path, size_bytes: 0, error: "..." }`.

**`/api/workspace/file` GET content-type sniffing:** uses the existing `mime_from_filename` helper in `context.rs` (pulled into a shared location). Images → `image/*`; text files → `text/plain`; binaries → `application/octet-stream`.

### 3.2 Frontend route + component structure

One new page, one new component:

| File | Purpose |
|---|---|
| `src/pages/Workspace.tsx` | Full workspace UI. Self-contained — no subroute. |
| `src/components/ConfirmModal.tsx` | Reusable confirm dialog used by delete. |

Route registration in `src/App.tsx`: one new `<Route>` line under the existing `/settings` route.

New types in `src/lib/types.ts`:

```ts
export type WorkspaceFile = {
  path: string;
  is_dir: boolean;
  size_bytes: number;
  modified_at: string;
};

export type WorkspaceQuota = {
  used_bytes: number;
  total_bytes: number;
};

export type WorkspaceSkill = {
  name: string;
  description: string;
  always_on: boolean;
};
```

API calls: no new methods on `src/lib/api.ts` — the existing `.get<T>()`, `.put()`, `.delete()` cover the JSON endpoints. Multipart upload uses the same pattern as `src/lib/upload.ts` (XMLHttpRequest + FormData) for progress reporting.

### 3.3 Page layout

```
┌─────────────────────────────────────────────────────────────────┐
│ ← back            Workspace                [quota: 2.1 / 5.0 GB]│  ← top bar (sticky)
├───────────────────────┬─────────────────────────────────────────┤
│ 📄 Soul               │ # MEMORY.md                              │
│ 📄 Memory             │                                          │
│ 📄 Heartbeat Tasks    │ ## User Facts                            │
│ ─────────────         │ - ...                                    │
│ 📁 skills             │                                          │
│   📁 create_skill     │                                          │
│     📄 SKILL.md       │                                          │  ← content pane
│ 📁 uploads            │                                          │
│   📄 photo.jpg        │                                          │
│ 📄 SOUL.md            │                                          │
│ 📄 MEMORY.md          │                                          │
│ 📄 HEARTBEAT.md       │                                          │
│                       │    [Edit] [Download] [Delete]            │  ← action buttons
├───────────────────────┴─────────────────────────────────────────┤
│ Drop files here to upload, or click + to pick                   │  ← drop zone (always visible)
└─────────────────────────────────────────────────────────────────┘
```

- **Top bar:** `<` back button (router.back), "Workspace" heading, quota chip on the right. Amber background at 80%, red at 95%.
- **Left pane (~25% width):** Quick-access section (Soul / Memory / Heartbeat) at top, separator, then the recursive tree. Folders open/close on click (keep expanded state in local component state).
- **Right pane (~75% width):** Renders the selected file. Empty state: "Select a file to view its contents."
- **Action bar (bottom of right pane):** `[Edit]` (markdown/text), `[Download]`, `[Delete]`.
- **Drop zone:** banner at the bottom; visible always. Drag-over state highlights the whole page (dashed border overlay).

### 3.4 Content renderers

Dispatched on the selected file's extension:

| Extension | Renderer |
|---|---|
| `.md` | Rendered with `react-markdown` by default. Edit button flips to `<textarea>`. Save writes via `PUT`. |
| `.txt`, `.json`, `.yaml`, `.yml`, `.toml`, `.rs`, `.ts`, `.tsx`, `.js`, `.py`, `.html`, `.css` | Direct `<textarea>` editor. Save writes via `PUT`. |
| `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp` | `<img>` with blob URL from the GET response. Download button. |
| Other | Metadata display (filename, size, modified date) + Download button. No inline preview. |
| Directory selected | Metadata (file count, total size) + "Delete directory" button (recursive-delete confirm). |

### 3.5 Quota bar

Progress bar rendering logic:

```
usage_pct = (used_bytes / total_bytes) * 100
color = usage_pct < 80 ? 'default' : usage_pct < 95 ? 'amber' : 'red'
label = `${formatBytes(used_bytes)} / ${formatBytes(total_bytes)}`
if (usage_pct >= 100) { banner = "Workspace full. Delete files to resume writes." }
```

Re-fetched after every successful write or delete.

### 3.6 Upload UX

Multiple files allowed. Per-file progress via XHR `onprogress`. On completion:
- Each successful file appears in the tree (refetch `tree`).
- Failed files surface a toast: `"upload {name}: {error}"`.

Drop zone accepts drag-over events on the whole document. The implementation uses `useEffect` to register/deregister listeners at component mount/unmount.

### 3.7 Settings cleanup scope

`plexus-frontend/src/pages/Settings.tsx::ProfileTab` currently has:
- `soul` + `memory` textareas + Save buttons that hit `PATCH /api/user/soul` / `PATCH /api/user/memory` (both return 410 Gone since Plan A-17).

Remove:
- `soul`, `memory` state variables.
- `saveSoul`, `saveMemory` functions.
- The two textarea sections in the JSX.
- The mount-time `GET /api/user/soul` / `GET /api/user/memory` fetches.

Keep: display-name + email + timezone UI (all still working).

Skills tab:
- Currently calls `GET /api/skills` (returns 410).
- Rewire to `GET /api/workspace/skills`.
- The "Install skill" button (`POST /api/skills/install` — 410) is removed; the comment notes users should use the agent's `write_file` tool or the Workspace page.
- The list becomes read-only display + "Edit in Workspace" link that jumps to `/settings/workspace` with `?path=skills/{name}/SKILL.md` pre-selected.

### 3.8 Quick-access sidebar → URL state

Selecting a file is reflected in the URL via query param `?path=...`. This makes links like `/settings/workspace?path=SOUL.md` directly shareable and lets the Skills tab deep-link.

### 3.9 react-markdown dependency

New runtime dep. Add via pnpm:

```
pnpm add react-markdown
```

Version pinned by pnpm's lockfile — no explicit version constraint in the task (use whatever latest stable at install time). The library is ~50 KB gzipped and has no peer dep conflicts with React 19.

### 3.10 Server-side: `file_store` interaction

Plan A-15 moved inbound-media uploads to the workspace. `POST /api/workspace/upload` reuses the same `file_store::save_upload` semantics but targets `uploads/` under the user's workspace, not `/tmp/plexus-uploads/`. Since Plan A already consolidated to the workspace, `save_upload` already writes to the correct location — this task is mostly about hooking the existing logic to a new HTTP endpoint, not re-implementing upload.

## 4. File Structure

### New files

| File | Responsibility |
|---|---|
| `plexus-frontend/src/pages/Workspace.tsx` | The full workspace page. |
| `plexus-frontend/src/components/ConfirmModal.tsx` | Shared confirm dialog. |

### Modified files (backend)

| File | Change |
|---|---|
| `plexus-server/src/api.rs` | +7 handlers + 7 route registrations. |
| `plexus-server/src/workspace/mod.rs` | May add a `tree::walk_user_tree` helper if the logic doesn't fit inline. |

### Modified files (frontend)

| File | Change |
|---|---|
| `plexus-frontend/src/App.tsx` | +1 `<Route path="/settings/workspace" element={<Workspace />} />` line. |
| `plexus-frontend/src/pages/Settings.tsx` | Remove Soul + Memory sections from ProfileTab; rewire Skills tab to `/api/workspace/skills`. |
| `plexus-frontend/src/lib/types.ts` | +3 types: `WorkspaceFile`, `WorkspaceQuota`, `WorkspaceSkill`. |
| `plexus-frontend/package.json` | +`react-markdown` dependency. |

### Docs

| File | Change |
|---|---|
| `plexus-server/docs/API.md` | +7 endpoint specs. |
| `plexus-server/docs/DECISIONS.md` | +ADR-37 (workspace REST API + frontend file manager). |
| `plexus-server/docs/ISSUE.md` | +deferred items (rename, bulk ops, frontend test harness, file-type coverage). |
| `docs/superpowers/plans/2026-04-18-workspace-frontend.md` | +Post-Plan Adjustments footer at completion. |
| `Plexus/README.md` or `plexus-frontend/README.md` | +paragraph describing the Workspace page. |

## 5. Testing Strategy

- **Backend unit + integration tests** gated on `DATABASE_URL` for the endpoints that need DB fixtures (mostly the upload + tree walk tests that need a real user + workspace directory). Pure-FS endpoints tested with `tempfile::TempDir`.
- **Backend TDD discipline:** every endpoint task writes the failing test first. Reuses Plan A's pattern of `#[tokio::test] #[ignore]` for DB-backed tests.
- **Frontend:** no automated tests ship with Plan B. Each task concludes with a manual smoke: run `pnpm dev` + click through the feature. The plan's self-check at the end requires a live-browser walkthrough of every feature.
- **Regression fence for existing tests:** 141 passing tests + 10 `#[ignore]`-gated on the server, 0 on the frontend. The backend tasks must keep all 141 green. The frontend cleanup (Settings) is risk-bounded to the Soul/Memory/skills-install flows.

## 6. Tasks

16 tasks total. Backend tasks B-1 through B-7 land the REST surface. Frontend tasks B-8 through B-15 build the UI on top. B-16 wraps docs + ADR.

Execution ordering rationale:
- Backend first (B-1 → B-7): frontend tasks can assume endpoints exist. Endpoint failures surface as Rust compile / test errors immediately, before UI work begins.
- Within backend: quota (smallest) → tree (foundational) → file GET/PUT/DELETE → upload (heaviest) → skills. Each depends only on earlier ones.
- Frontend: types + scaffold → tree → content renderers → upload → delete → sidebar/cleanup → docs.

---

### Task B-1: `/api/workspace/quota` endpoint

**Files:**
- Modify: `plexus-server/src/api.rs`

- [ ] **Step 1: Write the failing test**

Append to `#[cfg(test)] mod tests` at the bottom of `plexus-server/src/api.rs` (or create one if missing — check before writing):

```rust
    #[tokio::test]
    async fn test_workspace_quota_shape() {
        // Pure-logic test: the handler's response body matches the spec.
        // No HTTP harness needed — we construct the state and call the handler.
        let tmp = tempfile::TempDir::new().unwrap();
        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 5 * 1024 * 1024);
        state.quota.reserve_for_test("alice", 1024);

        let result = workspace_quota_handler(&state, "alice").await;
        assert_eq!(result.used_bytes, 1024);
        assert_eq!(result.total_bytes, 5 * 1024 * 1024);
    }
```

NOTE: `QuotaCache::reserve_for_test` may not exist; if it doesn't, add it as a `#[cfg(test)]` helper in `workspace/quota.rs`:

```rust
#[cfg(test)]
impl QuotaCache {
    pub fn reserve_for_test(&self, user_id: &str, bytes: u64) {
        let entry = self.usage.entry(user_id.to_string())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)));
        entry.fetch_add(bytes, Ordering::Relaxed);
    }
}
```

- [ ] **Step 2: Run test — verify it fails**

```bash
cargo test --package plexus-server api::tests::test_workspace_quota_shape
```

Expected: FAIL — `workspace_quota_handler` does not exist.

- [ ] **Step 3: Implement the handler + route**

Add to `plexus-server/src/api.rs`:

```rust
// At top of file, near the existing handlers:
use serde::Serialize;

#[derive(Serialize)]
pub struct WorkspaceQuotaResponse {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

pub async fn workspace_quota_handler(
    state: &AppState,
    user_id: &str,
) -> WorkspaceQuotaResponse {
    let used = state.quota.current_usage(user_id);
    let total = state.quota.total_bytes();
    WorkspaceQuotaResponse { used_bytes: used, total_bytes: total }
}

// The axum-level HTTP handler wrapping the pure function:
async fn workspace_quota(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> Result<Json<WorkspaceQuotaResponse>, (StatusCode, Json<ErrorBody>)> {
    Ok(Json(workspace_quota_handler(&state, &claims.sub).await))
}
```

`QuotaCache::current_usage(user_id: &str) -> u64` + `QuotaCache::total_bytes() -> u64` may not exist. Add them to `workspace/quota.rs` if missing:

```rust
impl QuotaCache {
    pub fn current_usage(&self, user_id: &str) -> u64 {
        self.usage.get(user_id)
            .map(|v| v.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    pub fn total_bytes(&self) -> u64 {
        self.quota_bytes
    }
}
```

- [ ] **Step 4: Register the route**

In `api_routes()` (around line 218 of `api.rs`):

```rust
.route("/api/workspace/quota", get(workspace_quota))
```

- [ ] **Step 5: Run test — verify it passes**

```bash
cargo test --package plexus-server api::tests::test_workspace_quota_shape
```

Expected: PASS.

- [ ] **Step 6: Build + full test suite**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
```

Expected: 142 tests pass (141 + 1 new).

- [ ] **Step 7: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(workspace-api): GET /api/workspace/quota

First of seven /api/workspace/* endpoints that Plan B's frontend
consumes. Returns { used_bytes, total_bytes } from the in-memory
QuotaCache — no DB round-trip. Reuses Plan A's quota machinery
unchanged.

Added two small QuotaCache helpers (current_usage, total_bytes)
for read-only access plus a #[cfg(test)] reserve_for_test helper
so the handler shape can be pinned without a real upload flow.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-2: `/api/workspace/tree` endpoint

**Files:**
- Modify: `plexus-server/src/api.rs`
- Create (or modify existing): `plexus-server/src/workspace/tree.rs` (tree-walk helper)
- Modify: `plexus-server/src/workspace/mod.rs` (re-export if new module)

- [ ] **Step 1: Add walkdir to Cargo.toml**

Check `plexus-server/Cargo.toml`. If `walkdir` is not present, add to `[dependencies]`:

```toml
walkdir = "2.5"
```

Run `cargo check --package plexus-server` to verify the dep resolves.

- [ ] **Step 2: Write the failing test**

In `plexus-server/src/workspace/tree.rs` (new file):

```rust
//! User workspace tree enumeration for the Workspace page (Plan B).

use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct WorkspaceEntry {
    pub path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub modified_at: chrono::DateTime<chrono::Utc>,
}

/// Walk the user's workspace tree depth-first. Returns a flat sorted list
/// of entries, paths relative to `{user_root}`. Symlinks are followed but
/// their targets must still live under `{user_root}` (canonicalized prefix
/// check — same invariant as `resolve_user_path`).
pub async fn walk_user_tree(
    workspace_root: &Path,
    user_id: &str,
) -> std::io::Result<Vec<WorkspaceEntry>> {
    let user_root = workspace_root.join(user_id);
    let user_root_canon = tokio::fs::canonicalize(&user_root).await?;

    let user_root_for_task = user_root_canon.clone();
    tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        for entry in walkdir::WalkDir::new(&user_root_for_task)
            .follow_links(true)
            .min_depth(1)
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue, // broken symlinks, permission errors — skip
            };
            let full = entry.path();
            // Symlink escape check: canonicalize and ensure still under user_root.
            let canon = match full.canonicalize() {
                Ok(c) => c,
                Err(_) => continue,
            };
            if !canon.starts_with(&user_root_for_task) {
                continue; // symlink escape — drop silently
            }
            let rel = match full.strip_prefix(&user_root_for_task) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(chrono::DateTime::<chrono::Utc>::from)
                .unwrap_or_else(chrono::Utc::now);
            entries.push(WorkspaceEntry {
                path: rel.to_string_lossy().into_owned(),
                is_dir: entry.file_type().is_dir(),
                size_bytes: if entry.file_type().is_dir() { 0 } else { size },
                modified_at: modified,
            });
        }
        // Directories first, then alphabetical.
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.path.cmp(&b.path),
        });
        Ok(entries)
    })
    .await
    .unwrap_or_else(|e| Err(std::io::Error::new(std::io::ErrorKind::Other, e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn walk_returns_sorted_entries() {
        let tmp = TempDir::new().unwrap();
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(user_root.join("skills/foo")).await.unwrap();
        tokio::fs::write(user_root.join("SOUL.md"), b"hello").await.unwrap();
        tokio::fs::write(user_root.join("skills/foo/SKILL.md"), b"---\nname: foo\n---").await.unwrap();

        let entries = walk_user_tree(tmp.path(), "alice").await.unwrap();

        // Directories first.
        assert!(entries[0].is_dir, "expected first entry to be dir; got {:?}", entries);
        // All paths relative.
        for e in &entries {
            assert!(!e.path.starts_with('/'), "paths must be relative; got {}", e.path);
        }
        // SOUL.md present with its bytes.
        let soul = entries.iter().find(|e| e.path == "SOUL.md").expect("SOUL.md missing");
        assert_eq!(soul.size_bytes, 5);
        assert!(!soul.is_dir);
    }

    #[tokio::test]
    async fn walk_rejects_symlink_escape() {
        let tmp = TempDir::new().unwrap();
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_root).await.unwrap();
        let outside = tmp.path().join("outside.txt");
        tokio::fs::write(&outside, b"secret").await.unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, user_root.join("escape.txt")).unwrap();

        let entries = walk_user_tree(tmp.path(), "alice").await.unwrap();

        // escape.txt must NOT be in the results (its canonicalized target escapes user_root).
        assert!(!entries.iter().any(|e| e.path == "escape.txt"),
                "symlink escape leaked into walk output: {:?}", entries);
    }
}
```

Register the module in `plexus-server/src/workspace/mod.rs`:

```rust
pub mod tree;
pub use tree::{walk_user_tree, WorkspaceEntry};
```

- [ ] **Step 3: Run test — verify it fails then passes after module add**

```bash
cargo test --package plexus-server workspace::tree
```

After writing the module, tests should pass. If they fail, read the output and fix.

- [ ] **Step 4: Wire the HTTP handler + route**

In `plexus-server/src/api.rs`:

```rust
async fn workspace_tree(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> Result<Json<Vec<crate::workspace::WorkspaceEntry>>, (StatusCode, Json<ErrorBody>)> {
    let root = std::path::Path::new(&state.config.workspace_root);
    match crate::workspace::walk_user_tree(root, &claims.sub).await {
        Ok(entries) => Ok(Json(entries)),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody { error: format!("tree walk failed: {e}") }),
        )),
    }
}
```

Register in `api_routes()`:

```rust
.route("/api/workspace/tree", get(workspace_tree))
```

- [ ] **Step 5: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server workspace::tree
cargo test --package plexus-server
```

Expected: new tests pass, 144 total (142 + 2 new).

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(workspace-api): GET /api/workspace/tree

Flat-list tree-walk of {workspace_root}/{user_id}/, paths relative
to user root, directories before files alphabetically. Symlinks are
followed but canonicalized targets must still live under user_root
— same invariant as resolve_user_path, so a symlink escape can't
leak out of the walk.

tokio::task::spawn_blocking isolates the sync walkdir iteration so
the async runtime doesn't stall on large trees.

Two unit tests: (1) sort/shape/relative-paths pin, (2) symlink
escape rejection.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-3: `/api/workspace/file` GET endpoint

**Files:**
- Modify: `plexus-server/src/api.rs`

- [ ] **Step 1: Write the failing test**

Append to `api.rs` tests:

```rust
    #[tokio::test]
    async fn test_workspace_file_get_inside_user_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_root).await.unwrap();
        tokio::fs::write(user_root.join("greeting.txt"), b"hi there").await.unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let bytes = workspace_file_get_bytes(&state, "alice", "greeting.txt").await.unwrap();
        assert_eq!(&bytes[..], b"hi there");
    }

    #[tokio::test]
    async fn test_workspace_file_get_rejects_traversal() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir_all(tmp.path().join("alice")).await.unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let err = workspace_file_get_bytes(&state, "alice", "../../etc/passwd")
            .await
            .unwrap_err();
        // WorkspaceError::Traversal or similar — just check it's an error, message isn't critical.
        assert!(format!("{err:?}").to_lowercase().contains("traversal")
             || format!("{err:?}").to_lowercase().contains("not found")
             || format!("{err:?}").to_lowercase().contains("outside"));
    }
```

- [ ] **Step 2: Implement the handler**

```rust
#[derive(Deserialize)]
pub struct WorkspaceFileQuery {
    pub path: String,
}

/// Testable core: given user_id + rel path, return bytes or an error.
/// HTTP wrapper below converts to StatusCode + headers.
pub async fn workspace_file_get_bytes(
    state: &AppState,
    user_id: &str,
    rel_path: &str,
) -> Result<Vec<u8>, crate::workspace::WorkspaceError> {
    let root = std::path::Path::new(&state.config.workspace_root);
    let resolved = crate::workspace::resolve_user_path(root, user_id, rel_path).await?;
    let bytes = tokio::fs::read(&resolved).await
        .map_err(|e| crate::workspace::WorkspaceError::Io(e.to_string()))?;
    Ok(bytes)
}

async fn workspace_file_get(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<WorkspaceFileQuery>,
) -> Result<Response, (StatusCode, Json<ErrorBody>)> {
    match workspace_file_get_bytes(&state, &claims.sub, &q.path).await {
        Ok(bytes) => {
            let mime = mime_from_path(&q.path);
            Ok(Response::builder()
                .header("Content-Type", mime)
                .body(axum::body::Body::from(bytes))
                .unwrap())
        }
        Err(e) => {
            let (code, msg) = match &e {
                crate::workspace::WorkspaceError::Traversal => (StatusCode::FORBIDDEN, "forbidden".to_string()),
                crate::workspace::WorkspaceError::Io(s) if s.contains("No such file") => (StatusCode::NOT_FOUND, "not found".to_string()),
                _ => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:?}")),
            };
            Err((code, Json(ErrorBody { error: msg })))
        }
    }
}

fn mime_from_path(p: &str) -> &'static str {
    let ext = p.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "md" | "txt" | "log" | "toml" | "rs" | "ts" | "tsx" | "js" | "py" | "yaml" | "yml" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}
```

- [ ] **Step 3: Register route**

```rust
.route("/api/workspace/file", get(workspace_file_get))
```

- [ ] **Step 4: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
```

Expected: 146 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(workspace-api): GET /api/workspace/file

Streams bytes from {workspace_root}/{user_id}/{rel_path}. Path
validated via workspace::resolve_user_path (same sandbox the agent's
file tools use). Content-Type inferred from filename extension
(text/json/images/fallback).

Testable core workspace_file_get_bytes is separated from the axum
HTTP wrapper so unit tests can pin the traversal + happy-path
behavior without a real HTTP harness.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-4: `/api/workspace/file` PUT endpoint

**Files:**
- Modify: `plexus-server/src/api.rs`

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn test_workspace_file_put_writes_bytes_and_updates_quota() {
        let tmp = tempfile::TempDir::new().unwrap();
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_root).await.unwrap();

        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 1024 * 1024);

        workspace_file_put_bytes(&state, "alice", "notes.md", b"hello".to_vec())
            .await
            .unwrap();

        let written = tokio::fs::read(user_root.join("notes.md")).await.unwrap();
        assert_eq!(written, b"hello");
        assert_eq!(state.quota.current_usage("alice"), 5);
    }

    #[tokio::test]
    async fn test_workspace_file_put_rejects_quota_overage() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir_all(tmp.path().join("alice")).await.unwrap();

        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 10);

        let err = workspace_file_put_bytes(&state, "alice", "big.bin", vec![0; 100])
            .await
            .unwrap_err();
        assert!(format!("{err:?}").to_lowercase().contains("quota"));
    }
```

- [ ] **Step 2: Implement**

```rust
pub async fn workspace_file_put_bytes(
    state: &AppState,
    user_id: &str,
    rel_path: &str,
    bytes: Vec<u8>,
) -> Result<(), crate::workspace::WorkspaceError> {
    let root = std::path::Path::new(&state.config.workspace_root);
    let resolved = crate::workspace::resolve_user_path_for_create(root, user_id, rel_path).await?;

    // Compute delta against existing file (if any), reserve against quota.
    let existing = tokio::fs::metadata(&resolved).await.map(|m| m.len()).unwrap_or(0);
    let new_size = bytes.len() as u64;
    let delta = new_size.saturating_sub(existing);

    if delta > 0 {
        state.quota.check_and_reserve_upload(user_id, delta)?;
    }

    // Create parent dirs.
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent).await
            .map_err(|e| crate::workspace::WorkspaceError::Io(e.to_string()))?;
    }

    match tokio::fs::write(&resolved, &bytes).await {
        Ok(()) => {
            // If we shrunk the file, release the reclaimed bytes.
            if new_size < existing {
                state.quota.release(user_id, existing - new_size);
            }
            Ok(())
        }
        Err(e) => {
            // Rollback the reservation.
            if delta > 0 {
                state.quota.release(user_id, delta);
            }
            Err(crate::workspace::WorkspaceError::Io(e.to_string()))
        }
    }
}

async fn workspace_file_put(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<WorkspaceFileQuery>,
    body: axum::body::Bytes,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    match workspace_file_put_bytes(&state, &claims.sub, &q.path, body.to_vec()).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => {
            let code = match &e {
                crate::workspace::WorkspaceError::Traversal => StatusCode::FORBIDDEN,
                crate::workspace::WorkspaceError::Quota(_) => StatusCode::PAYLOAD_TOO_LARGE,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err((code, Json(ErrorBody { error: format!("{e:?}") })))
        }
    }
}
```

If `QuotaCache::release(user_id, bytes)` doesn't exist (check `workspace/quota.rs`), add it:

```rust
pub fn release(&self, user_id: &str, bytes: u64) {
    if let Some(entry) = self.usage.get(user_id) {
        entry.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
            Some(v.saturating_sub(bytes))
        }).ok();
    }
}
```

- [ ] **Step 3: Register route**

```rust
.route("/api/workspace/file", get(workspace_file_get).put(workspace_file_put))
```

(Combine with the GET route — axum supports multiple methods per path via `.get().put()`.)

- [ ] **Step 4: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
```

Expected: 148 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(workspace-api): PUT /api/workspace/file

Writes raw body bytes to {workspace_root}/{user_id}/{rel_path}.
Creates parent dirs. Quota-checked via check_and_reserve_upload
(delta against existing file size). Rollback on write failure.

Testable core workspace_file_put_bytes is separated from the axum
HTTP wrapper — two unit tests pin the happy-path quota accounting
and the over-quota rejection.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-5: `/api/workspace/file` DELETE endpoint

**Files:**
- Modify: `plexus-server/src/api.rs`

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn test_workspace_file_delete_file_updates_quota() {
        let tmp = tempfile::TempDir::new().unwrap();
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(&user_root).await.unwrap();
        tokio::fs::write(user_root.join("doomed.txt"), b"goodbye").await.unwrap();

        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 1024 * 1024);
        state.quota.reserve_for_test("alice", 7);

        workspace_file_delete_path(&state, "alice", "doomed.txt", false)
            .await
            .unwrap();

        assert!(!user_root.join("doomed.txt").exists());
        assert_eq!(state.quota.current_usage("alice"), 0);
    }

    #[tokio::test]
    async fn test_workspace_file_delete_directory_requires_recursive_flag() {
        let tmp = tempfile::TempDir::new().unwrap();
        let user_root = tmp.path().join("alice");
        tokio::fs::create_dir_all(user_root.join("subdir")).await.unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let err = workspace_file_delete_path(&state, "alice", "subdir", false)
            .await
            .unwrap_err();
        assert!(format!("{err:?}").to_lowercase().contains("directory"));

        // With recursive: true, it succeeds.
        workspace_file_delete_path(&state, "alice", "subdir", true).await.unwrap();
        assert!(!user_root.join("subdir").exists());
    }
```

- [ ] **Step 2: Implement**

```rust
#[derive(Deserialize)]
pub struct WorkspaceDeleteQuery {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

pub async fn workspace_file_delete_path(
    state: &AppState,
    user_id: &str,
    rel_path: &str,
    recursive: bool,
) -> Result<(), crate::workspace::WorkspaceError> {
    let root = std::path::Path::new(&state.config.workspace_root);
    let resolved = crate::workspace::resolve_user_path(root, user_id, rel_path).await?;

    let meta = tokio::fs::metadata(&resolved).await
        .map_err(|e| crate::workspace::WorkspaceError::Io(e.to_string()))?;

    if meta.is_dir() {
        if !recursive {
            return Err(crate::workspace::WorkspaceError::Io(
                "directory delete requires recursive=true".into()
            ));
        }
        // Sum sizes before deletion to release from quota.
        let freed = dir_size(&resolved).await.unwrap_or(0);
        tokio::fs::remove_dir_all(&resolved).await
            .map_err(|e| crate::workspace::WorkspaceError::Io(e.to_string()))?;
        state.quota.release(user_id, freed);
    } else {
        let size = meta.len();
        tokio::fs::remove_file(&resolved).await
            .map_err(|e| crate::workspace::WorkspaceError::Io(e.to_string()))?;
        state.quota.release(user_id, size);
    }
    Ok(())
}

async fn dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut total = 0u64;
        for entry in walkdir::WalkDir::new(&path).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                if let Ok(m) = entry.metadata() {
                    total = total.saturating_add(m.len());
                }
            }
        }
        Ok(total)
    })
    .await
    .unwrap_or_else(|e| Err(std::io::Error::new(std::io::ErrorKind::Other, e)))
}

async fn workspace_file_delete(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Query(q): Query<WorkspaceDeleteQuery>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    workspace_file_delete_path(&state, &claims.sub, &q.path, q.recursive)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|e| {
            let code = match &e {
                crate::workspace::WorkspaceError::Traversal => StatusCode::FORBIDDEN,
                _ => StatusCode::BAD_REQUEST,
            };
            (code, Json(ErrorBody { error: format!("{e:?}") }))
        })
}
```

- [ ] **Step 3: Register route**

Update the `/api/workspace/file` route to include DELETE:

```rust
.route("/api/workspace/file",
    get(workspace_file_get).put(workspace_file_put).delete(workspace_file_delete))
```

- [ ] **Step 4: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
```

Expected: 150 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(workspace-api): DELETE /api/workspace/file

Deletes a file or (with recursive=true) a directory. Recursive-flag
gate prevents accidental directory wipes. Released bytes are
subtracted from the in-memory QuotaCache.

Two unit tests: (1) single-file delete + quota release, (2) the
recursive flag gate — attempting to delete a directory without it
returns an error; with it, the directory and its subtree vanish.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-6: `/api/workspace/upload` endpoint (multipart)

**Files:**
- Modify: `plexus-server/src/api.rs`

- [ ] **Step 1: Ensure multipart deps are in place**

`axum` already has `axum::extract::Multipart` (behind the `multipart` feature). Check `plexus-server/Cargo.toml`; the existing `POST /api/files` handler (line 175-196 of `api.rs`) already uses `Multipart`, so the feature is already enabled. No dep change.

- [ ] **Step 2: Write the failing test — skip if pure multipart testing is awkward**

Skip a formal Rust-level multipart test. Multipart parsing is best exercised via a manual curl smoke or a frontend task-level test. Instead, write a test that exercises the *file-landing* logic directly:

```rust
    #[tokio::test]
    async fn test_workspace_upload_saves_to_uploads_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir_all(tmp.path().join("alice/uploads")).await.unwrap();

        let state = crate::state::AppState::test_minimal_with_quota(tmp.path(), 1024 * 1024);
        let saved = workspace_upload_save_one(
            &state, "alice", "photo.jpg", b"fakedata".to_vec(),
        ).await.unwrap();

        assert!(saved.path.starts_with("uploads/"), "expected uploads/ prefix; got {}", saved.path);
        assert!(saved.path.ends_with("photo.jpg"));
        assert_eq!(saved.size_bytes, 8);

        // File actually exists on disk.
        let full = tmp.path().join("alice").join(&saved.path);
        assert!(full.exists());
    }
```

- [ ] **Step 3: Implement the save-one helper + multipart handler**

```rust
#[derive(Serialize)]
pub struct WorkspaceUploadResult {
    pub path: String,
    pub size_bytes: u64,
}

/// Save a single uploaded file under {user_root}/uploads/, using the same
/// dated-hashed naming convention as the channel adapters' inbound-media path:
///   uploads/{YYYY-MM-DD}-{8-char-hash}-{filename}
///
/// Quota enforcement happens inside `workspace_file_put_bytes` (reserves the
/// delta against the existing file, rolls back on write failure). Do NOT
/// pre-reserve here — that would double-count against the quota.
pub async fn workspace_upload_save_one(
    state: &AppState,
    user_id: &str,
    original_filename: &str,
    bytes: Vec<u8>,
) -> Result<WorkspaceUploadResult, crate::workspace::WorkspaceError> {
    let size = bytes.len() as u64;

    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        format!("{:08x}", hasher.finish() as u32)
    };
    let safe_name = original_filename
        .replace(['/', '\\'], "_")
        .replace("..", "_");
    let rel = format!("uploads/{date}-{hash}-{safe_name}");

    workspace_file_put_bytes(state, user_id, &rel, bytes).await?;
    Ok(WorkspaceUploadResult { path: rel, size_bytes: size })
}

async fn workspace_upload(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    mut multipart: axum::extract::Multipart,
) -> Result<Json<Vec<WorkspaceUploadResult>>, (StatusCode, Json<ErrorBody>)> {
    let mut results = Vec::new();
    while let Some(field) = multipart.next_field().await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ErrorBody { error: format!("multipart: {e}") })))?
    {
        let filename = field.file_name().unwrap_or("unnamed").to_string();
        let bytes = field.bytes().await
            .map_err(|e| (StatusCode::BAD_REQUEST, Json(ErrorBody { error: format!("multipart read: {e}") })))?;
        match workspace_upload_save_one(&state, &claims.sub, &filename, bytes.to_vec()).await {
            Ok(r) => results.push(r),
            Err(e) => results.push(WorkspaceUploadResult {
                path: format!("ERROR:{filename}"),
                size_bytes: 0,
                // NOTE: simpler shape without an inline error field keeps the client
                // contract consistent; the "ERROR:" prefix is a sentinel.
            }),
        }
    }
    Ok(Json(results))
}
```

The hash is computed via `std::collections::hash_map::DefaultHasher` (dep-free, 8 hex chars). The hash doesn't need to be cryptographic — it just needs to disambiguate same-day same-filename uploads, and collision-resistance at 32 bits is fine for that.

- [ ] **Step 4: Register route**

```rust
.route("/api/workspace/upload", post(workspace_upload))
```

- [ ] **Step 5: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
```

Expected: 151 tests pass.

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(workspace-api): POST /api/workspace/upload (multipart)

Accepts one or more files and lands them under
{user_root}/uploads/{YYYY-MM-DD}-{hash}-{filename}. Matches the
existing channel-adapter inbound-media naming convention so the
agent can find uploads the same way regardless of origin.

Per-file quota enforcement via the shared workspace_file_put_bytes
helper. Failed files surface with path="ERROR:{filename}" + size=0
sentinel; successful files return their relative path + size.

Dep-free hash (DefaultHasher → 8 hex chars) avoids pulling in
sha2/hex for this one use.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-7: `/api/workspace/skills` endpoint

**Files:**
- Modify: `plexus-server/src/api.rs`

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn test_workspace_skills_returns_parsed_frontmatter() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skills_root = tmp.path().join("alice/skills/demo");
        tokio::fs::create_dir_all(&skills_root).await.unwrap();
        tokio::fs::write(
            skills_root.join("SKILL.md"),
            b"---\nname: demo\ndescription: A demo skill\nalways_on: true\n---\n\n# Demo",
        ).await.unwrap();

        let state = crate::state::AppState::test_minimal(tmp.path());
        let skills = workspace_skills_list(&state, "alice").await;
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "demo");
        assert_eq!(skills[0].description, "A demo skill");
        assert!(skills[0].always_on);
    }
```

- [ ] **Step 2: Implement**

```rust
#[derive(Serialize)]
pub struct WorkspaceSkillSummary {
    pub name: String,
    pub description: String,
    pub always_on: bool,
}

pub async fn workspace_skills_list(
    state: &AppState,
    user_id: &str,
) -> Vec<WorkspaceSkillSummary> {
    let root = std::path::Path::new(&state.config.workspace_root);
    let bundle = state.skills_cache.get_or_load(user_id, root).await;
    bundle.iter().map(|s| WorkspaceSkillSummary {
        name: s.name.clone(),
        description: s.description.clone(),
        always_on: s.always_on,
    }).collect()
}

async fn workspace_skills(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> Json<Vec<WorkspaceSkillSummary>> {
    Json(workspace_skills_list(&state, &claims.sub).await)
}
```

- [ ] **Step 3: Register route**

```rust
.route("/api/workspace/skills", get(workspace_skills))
```

- [ ] **Step 4: Build + test**

```bash
cargo build --package plexus-server
cargo test --package plexus-server
```

Expected: 152 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(workspace-api): GET /api/workspace/skills

Returns the user's skills as parsed frontmatter:
[{name, description, always_on}]. Reuses Plan A's SkillsCache
get_or_load, so disk changes (e.g., dream creating a new skill)
reflect on the next call.

Replaces the Plan A-17 dropped /api/skills endpoint for the
frontend. The Settings Skills tab rewires to this in B-15.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-8: Frontend types + `/settings/workspace` route scaffold

**Files:**
- Modify: `plexus-frontend/src/lib/types.ts`
- Create: `plexus-frontend/src/pages/Workspace.tsx`
- Modify: `plexus-frontend/src/App.tsx`

- [ ] **Step 1: Add the types**

In `plexus-frontend/src/lib/types.ts`, append:

```ts
export type WorkspaceFile = {
  path: string;
  is_dir: boolean;
  size_bytes: number;
  modified_at: string;
};

export type WorkspaceQuota = {
  used_bytes: number;
  total_bytes: number;
};

export type WorkspaceSkill = {
  name: string;
  description: string;
  always_on: boolean;
};
```

- [ ] **Step 2: Create the Workspace page scaffold**

Create `plexus-frontend/src/pages/Workspace.tsx`:

```tsx
import { useEffect, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { ArrowLeft } from 'lucide-react';
import { api } from '../lib/api';
import type { WorkspaceFile, WorkspaceQuota } from '../lib/types';

export default function Workspace() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const selectedPath = params.get('path') ?? '';

  const [tree, setTree] = useState<WorkspaceFile[] | null>(null);
  const [quota, setQuota] = useState<WorkspaceQuota | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void refresh();
  }, []);

  async function refresh() {
    try {
      const [t, q] = await Promise.all([
        api.get<WorkspaceFile[]>('/api/workspace/tree'),
        api.get<WorkspaceQuota>('/api/workspace/quota'),
      ]);
      setTree(t);
      setQuota(q);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'failed to load workspace');
    }
  }

  return (
    <div className="flex flex-col h-screen" style={{ background: 'var(--bg)', color: 'var(--text)' }}>
      <header
        className="flex items-center gap-4 px-4 py-3 border-b"
        style={{ borderColor: 'var(--border)' }}
      >
        <button onClick={() => navigate('/settings')} className="hover:opacity-70">
          <ArrowLeft size={18} />
        </button>
        <h1 className="text-lg font-semibold">Workspace</h1>
        <div className="ml-auto">
          {quota && (
            <span className="text-sm" style={{ color: 'var(--muted)' }}>
              {formatBytes(quota.used_bytes)} / {formatBytes(quota.total_bytes)}
            </span>
          )}
        </div>
      </header>

      {error && (
        <div className="p-2 text-sm" style={{ color: '#ff6b6b' }}>
          {error}
        </div>
      )}

      <main className="flex-1 flex overflow-hidden">
        <aside
          className="w-1/4 overflow-y-auto border-r p-2"
          style={{ borderColor: 'var(--border)', background: 'var(--sidebar)' }}
        >
          {/* Tree will land in B-9 */}
          <div style={{ color: 'var(--muted)' }}>Tree pane (B-9)</div>
        </aside>

        <section className="flex-1 overflow-y-auto p-4">
          {/* Content pane will land in B-10 */}
          <div style={{ color: 'var(--muted)' }}>
            Content pane for <code>{selectedPath || '(nothing selected)'}</code> (B-10)
          </div>
        </section>
      </main>
    </div>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
```

- [ ] **Step 3: Register the route**

In `plexus-frontend/src/App.tsx`, add a new `<Route>` for Workspace. Import at the top:

```tsx
import Workspace from './pages/Workspace';
```

Then inside the existing authenticated route group (wherever `<Route path="/settings" ...>` lives), add:

```tsx
<Route path="/settings/workspace" element={<Workspace />} />
```

- [ ] **Step 4: Smoke test**

```bash
cd plexus-frontend
pnpm typecheck
pnpm dev
```

In a browser, navigate to `http://localhost:5173/settings/workspace`. Expected: page loads, top bar shows "Workspace", quota chip shows "X / 5.0 GB", left pane shows "Tree pane (B-9)", right pane shows "Content pane for (nothing selected) (B-10)".

If `api.get` fails with 401, log in first via `/login`.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(frontend): Workspace page scaffold at /settings/workspace

Empty shell that: (1) fetches /api/workspace/tree + /api/workspace/quota
on mount, (2) renders the top bar with a quota chip, (3) leaves
the tree + content panes as B-9/B-10 placeholders.

Also adds WorkspaceFile/WorkspaceQuota/WorkspaceSkill types to
lib/types.ts for the rest of the page's consumers.

Route registered in App.tsx via React Router 7.5.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-9: Quota bar + tree view

**Files:**
- Modify: `plexus-frontend/src/pages/Workspace.tsx`

- [ ] **Step 1: Add the quota-bar visual + tree component inline**

Replace the `header` section of `Workspace.tsx` with a richer quota bar:

```tsx
      <header
        className="flex items-center gap-4 px-4 py-3 border-b"
        style={{ borderColor: 'var(--border)' }}
      >
        <button onClick={() => navigate('/settings')} className="hover:opacity-70">
          <ArrowLeft size={18} />
        </button>
        <h1 className="text-lg font-semibold">Workspace</h1>
        <div className="flex-1 max-w-md">
          {quota && <QuotaBar q={quota} />}
        </div>
      </header>
```

Add the `QuotaBar` component at the bottom of the file:

```tsx
function QuotaBar({ q }: { q: WorkspaceQuota }) {
  const pct = q.total_bytes > 0 ? (q.used_bytes / q.total_bytes) * 100 : 0;
  const clamped = Math.min(100, pct);
  const color = pct >= 95 ? '#ef4444' : pct >= 80 ? '#f59e0b' : 'var(--accent)';
  return (
    <div className="flex flex-col gap-1">
      <div className="flex justify-between text-xs" style={{ color: 'var(--muted)' }}>
        <span>{formatBytes(q.used_bytes)} / {formatBytes(q.total_bytes)}</span>
        <span>{pct.toFixed(1)}%</span>
      </div>
      <div className="h-2 rounded" style={{ background: 'var(--border)' }}>
        <div
          className="h-2 rounded"
          style={{ width: `${clamped}%`, background: color, transition: 'width 200ms ease' }}
        />
      </div>
      {pct >= 100 && (
        <div className="text-xs" style={{ color: '#ef4444' }}>
          Workspace full. Delete files to resume writes.
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Build the recursive tree view**

Add a new component `TreeView` that takes the flat `WorkspaceFile[]` and renders a nested collapsible tree. Replace the left-pane placeholder:

```tsx
        <aside
          className="w-1/4 overflow-y-auto border-r p-2"
          style={{ borderColor: 'var(--border)', background: 'var(--sidebar)' }}
        >
          {tree && (
            <TreeView
              entries={tree}
              selected={selectedPath}
              onSelect={(path) => setParams({ path })}
            />
          )}
          {!tree && (
            <div style={{ color: 'var(--muted)' }}>Loading…</div>
          )}
        </aside>
```

Add `TreeView` component at the bottom of the file. The tree builder groups entries by their path prefixes:

```tsx
import { ChevronRight, ChevronDown, File, Folder } from 'lucide-react';

type TreeNode = {
  name: string;
  path: string;
  is_dir: boolean;
  size_bytes: number;
  children: TreeNode[];
};

function buildTree(entries: WorkspaceFile[]): TreeNode[] {
  const root: TreeNode = { name: '', path: '', is_dir: true, size_bytes: 0, children: [] };
  // Sort so parents are inserted before children.
  const sorted = [...entries].sort((a, b) => a.path.localeCompare(b.path));
  for (const e of sorted) {
    const parts = e.path.split('/');
    let cur = root;
    for (let i = 0; i < parts.length; i++) {
      const partPath = parts.slice(0, i + 1).join('/');
      let child = cur.children.find((c) => c.path === partPath);
      if (!child) {
        child = {
          name: parts[i],
          path: partPath,
          is_dir: i < parts.length - 1 ? true : e.is_dir,
          size_bytes: i === parts.length - 1 ? e.size_bytes : 0,
          children: [],
        };
        cur.children.push(child);
      }
      cur = child;
    }
  }
  // Sort each level: dirs first, then alphabetical.
  const sortChildren = (n: TreeNode) => {
    n.children.sort((a, b) => {
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    for (const c of n.children) sortChildren(c);
  };
  sortChildren(root);
  return root.children;
}

function TreeView({
  entries,
  selected,
  onSelect,
}: {
  entries: WorkspaceFile[];
  selected: string;
  onSelect: (path: string) => void;
}) {
  const tree = buildTree(entries);
  return <TreeNodeList nodes={tree} depth={0} selected={selected} onSelect={onSelect} />;
}

function TreeNodeList({
  nodes,
  depth,
  selected,
  onSelect,
}: {
  nodes: TreeNode[];
  depth: number;
  selected: string;
  onSelect: (path: string) => void;
}) {
  return (
    <ul className="list-none p-0 m-0">
      {nodes.map((n) => (
        <TreeItem key={n.path} node={n} depth={depth} selected={selected} onSelect={onSelect} />
      ))}
    </ul>
  );
}

function TreeItem({
  node,
  depth,
  selected,
  onSelect,
}: {
  node: TreeNode;
  depth: number;
  selected: string;
  onSelect: (path: string) => void;
}) {
  const [open, setOpen] = useState(depth === 0);
  const isSelected = selected === node.path;
  return (
    <li>
      <div
        className="flex items-center gap-1 px-1 py-0.5 rounded cursor-pointer"
        style={{
          paddingLeft: `${depth * 12 + 4}px`,
          background: isSelected ? 'var(--accent)' : 'transparent',
          color: isSelected ? 'var(--bg)' : 'var(--text)',
        }}
        onClick={() => {
          if (node.is_dir) setOpen(!open);
          onSelect(node.path);
        }}
      >
        {node.is_dir ? (
          open ? <ChevronDown size={14} /> : <ChevronRight size={14} />
        ) : (
          <span style={{ width: 14 }} />
        )}
        {node.is_dir ? <Folder size={14} /> : <File size={14} />}
        <span className="text-sm">{node.name}</span>
        {!node.is_dir && (
          <span className="ml-auto text-xs" style={{ color: 'var(--muted)' }}>
            {formatBytes(node.size_bytes)}
          </span>
        )}
      </div>
      {node.is_dir && open && node.children.length > 0 && (
        <TreeNodeList nodes={node.children} depth={depth + 1} selected={selected} onSelect={onSelect} />
      )}
    </li>
  );
}
```

- [ ] **Step 3: Smoke test**

```bash
cd plexus-frontend
pnpm typecheck
pnpm dev
```

Navigate to `/settings/workspace`. Expect to see:
- Quota bar across the top with a filled-in amount, percentage, and appropriate color.
- Left pane populated with the user's workspace tree. Folders collapsible; clicking a file sets the `?path=...` URL param and highlights the entry.

Manually verify:
- A 5-byte file shows "5 B".
- Directories sort before files at each level.
- URL query param updates when you click.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(frontend): workspace quota bar + recursive tree view

Quota bar: green/amber/red thresholds at 80% / 95% with a
"workspace full" banner at ≥100%. Updates via re-fetch after
any mutation.

Tree view: flat server response → nested client-side tree via
path-prefix grouping, directories-first ordering at each level,
chevron toggle for folders, file-size display, URL-synced selection
via ?path=… (deep-linkable, shareable, back-button-safe).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-10: Content pane — markdown render + text viewer

**Files:**
- Modify: `plexus-frontend/src/pages/Workspace.tsx`
- Modify: `plexus-frontend/package.json` (add react-markdown)

- [ ] **Step 1: Install react-markdown**

```bash
cd plexus-frontend
pnpm add react-markdown
```

This updates `package.json` + `pnpm-lock.yaml`.

- [ ] **Step 2: Add a ContentPane component**

In `Workspace.tsx`, replace the right-pane placeholder with a real content pane. Add:

```tsx
import ReactMarkdown from 'react-markdown';

function ContentPane({ path, onDeleted }: { path: string; onDeleted: () => void }) {
  const [bytes, setBytes] = useState<ArrayBuffer | null>(null);
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!path) {
      setBytes(null);
      setText(null);
      return;
    }
    void load();
  }, [path]);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const res = await fetch(`/api/workspace/file?path=${encodeURIComponent(path)}`, {
        headers: { Authorization: `Bearer ${localStorage.getItem('token') ?? ''}` },
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${await res.text()}`);
      const buf = await res.arrayBuffer();
      setBytes(buf);
      const mime = res.headers.get('Content-Type') ?? '';
      if (mime.startsWith('text/') || mime === 'application/json') {
        setText(new TextDecoder().decode(buf));
      } else {
        setText(null);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : 'load failed');
    } finally {
      setLoading(false);
    }
  }

  if (!path) return <div style={{ color: 'var(--muted)' }}>Select a file to view its contents.</div>;
  if (loading) return <div style={{ color: 'var(--muted)' }}>Loading…</div>;
  if (error) return <div style={{ color: '#ef4444' }}>{error}</div>;

  const isMarkdown = path.toLowerCase().endsWith('.md');

  return (
    <div className="flex flex-col gap-4 h-full">
      <div
        className="text-xs px-2 py-1 rounded"
        style={{ background: 'var(--sidebar)', color: 'var(--muted)' }}
      >
        {path}
      </div>
      <div className="flex-1 overflow-y-auto">
        {isMarkdown && text !== null ? (
          <div className="prose prose-invert max-w-none">
            <ReactMarkdown>{text}</ReactMarkdown>
          </div>
        ) : text !== null ? (
          <pre
            className="text-sm whitespace-pre-wrap"
            style={{ background: 'var(--card)', padding: '1rem', borderRadius: '4px' }}
          >
            {text}
          </pre>
        ) : (
          <div style={{ color: 'var(--muted)' }}>Non-text file (renderer B-12 will handle images/binaries).</div>
        )}
      </div>
    </div>
  );
}
```

Wire it into the page:

```tsx
        <section className="flex-1 overflow-y-auto p-4">
          <ContentPane path={selectedPath} onDeleted={() => void refresh()} />
        </section>
```

- [ ] **Step 3: Smoke test**

Navigate to `/settings/workspace`, click `SOUL.md` in the tree. Expect the markdown content to render as rich text in the right pane.

Click `HEARTBEAT.md` — different markdown content renders.

Click `skills/create_skill/SKILL.md` — frontmatter is visible at the top (react-markdown treats YAML-style fences as code blocks by default), then the body renders.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(frontend): content pane — markdown render + text viewer

Selected file fetches via /api/workspace/file, dispatches on
Content-Type:
  - .md → ReactMarkdown rich rendering (prose styling).
  - text/* or application/json → <pre> verbatim view.
  - other → placeholder for B-12 (images/binaries).

Uses the raw fetch API with the Authorization header (not api.get)
because ArrayBuffer responses need explicit content-type dispatch.

pnpm added react-markdown.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-11: Content pane — edit mode + save

**Files:**
- Modify: `plexus-frontend/src/pages/Workspace.tsx`

- [ ] **Step 1: Add edit mode to ContentPane**

Extend `ContentPane` state + UI:

```tsx
function ContentPane({ path, onChanged }: { path: string; onChanged: () => void }) {
  const [bytes, setBytes] = useState<ArrayBuffer | null>(null);
  const [text, setText] = useState<string | null>(null);
  const [editBuf, setEditBuf] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // … existing load() unchanged …

  async function save() {
    if (editBuf === null) return;
    setSaving(true);
    setError(null);
    try {
      const res = await fetch(`/api/workspace/file?path=${encodeURIComponent(path)}`, {
        method: 'PUT',
        headers: { Authorization: `Bearer ${localStorage.getItem('token') ?? ''}` },
        body: editBuf,
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${await res.text()}`);
      setText(editBuf);
      setEditBuf(null);
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'save failed');
    } finally {
      setSaving(false);
    }
  }

  const isEditable = text !== null; // text/*, application/json, .md — all editable
  const inEditMode = editBuf !== null;

  // … existing short-circuits for !path / loading / error …

  return (
    <div className="flex flex-col gap-4 h-full">
      <div className="flex items-center gap-2">
        <div
          className="text-xs px-2 py-1 rounded flex-1"
          style={{ background: 'var(--sidebar)', color: 'var(--muted)' }}
        >
          {path}
        </div>
        {isEditable && !inEditMode && (
          <button
            onClick={() => setEditBuf(text)}
            className="text-xs px-2 py-1 rounded"
            style={{ border: '1px solid var(--border)' }}
          >
            Edit
          </button>
        )}
        {inEditMode && (
          <>
            <button
              onClick={save}
              disabled={saving}
              className="text-xs px-2 py-1 rounded"
              style={{ background: 'var(--accent)', color: 'var(--bg)' }}
            >
              {saving ? 'Saving…' : 'Save'}
            </button>
            <button
              onClick={() => setEditBuf(null)}
              className="text-xs px-2 py-1 rounded"
              style={{ border: '1px solid var(--border)' }}
            >
              Cancel
            </button>
          </>
        )}
      </div>
      <div className="flex-1 overflow-y-auto">
        {inEditMode ? (
          <textarea
            value={editBuf ?? ''}
            onChange={(e) => setEditBuf(e.target.value)}
            className="w-full h-full text-sm font-mono p-4"
            style={{ background: 'var(--card)', color: 'var(--text)', border: '1px solid var(--border)' }}
          />
        ) : path.toLowerCase().endsWith('.md') && text !== null ? (
          <div className="prose prose-invert max-w-none">
            <ReactMarkdown>{text}</ReactMarkdown>
          </div>
        ) : text !== null ? (
          <pre
            className="text-sm whitespace-pre-wrap"
            style={{ background: 'var(--card)', padding: '1rem', borderRadius: '4px' }}
          >
            {text}
          </pre>
        ) : (
          <div style={{ color: 'var(--muted)' }}>Non-text file (renderer B-12 will handle images/binaries).</div>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Smoke test**

Navigate to `/settings/workspace?path=SOUL.md`. Click "Edit" — textarea appears with the file content. Modify some text. Click "Save". Expect status "Saving…" briefly, then back to rendered view with the updated content.

Refresh the page — changes persisted.

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(frontend): content pane edit mode + PUT save

Text/JSON/markdown files gain an "Edit" button that flips the
renderer to a monospace textarea. Save writes via PUT to
/api/workspace/file; on success, reverts to rendered view with
the new content and calls onChanged so the tree+quota re-fetch.

Cancel discards the edit buffer without touching disk.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-12: Content pane — image + binary rendering

**Files:**
- Modify: `plexus-frontend/src/pages/Workspace.tsx`

- [ ] **Step 1: Extend ContentPane for non-text files**

Update `load()` to retain the blob URL for image MIMEs:

```tsx
  const [blobUrl, setBlobUrl] = useState<string | null>(null);
  const [mime, setMime] = useState<string>('');

  // … in load() after setBytes(buf):
      const mimeType = res.headers.get('Content-Type') ?? '';
      setMime(mimeType);
      if (mimeType.startsWith('image/')) {
        // Revoke any prior URL.
        if (blobUrl) URL.revokeObjectURL(blobUrl);
        const url = URL.createObjectURL(new Blob([buf], { type: mimeType }));
        setBlobUrl(url);
      } else {
        if (blobUrl) {
          URL.revokeObjectURL(blobUrl);
          setBlobUrl(null);
        }
      }
```

Add a cleanup effect:

```tsx
  useEffect(() => () => {
    if (blobUrl) URL.revokeObjectURL(blobUrl);
  }, []);
```

Replace the non-text branch:

```tsx
        ) : mime.startsWith('image/') && blobUrl ? (
          <div className="flex flex-col gap-2">
            <img src={blobUrl} alt={path} className="max-w-full max-h-full object-contain" />
            <a
              href={blobUrl}
              download={path.split('/').pop()}
              className="text-xs self-start px-2 py-1 rounded"
              style={{ border: '1px solid var(--border)' }}
            >
              Download
            </a>
          </div>
        ) : (
          <div className="flex flex-col gap-2 items-start">
            <div style={{ color: 'var(--muted)' }}>
              Binary file — {bytes ? formatBytes(bytes.byteLength) : '?'} · {mime || 'application/octet-stream'}
            </div>
            <a
              href={`/api/workspace/file?path=${encodeURIComponent(path)}`}
              download={path.split('/').pop()}
              className="text-xs px-2 py-1 rounded"
              style={{ border: '1px solid var(--border)' }}
            >
              Download
            </a>
          </div>
        )}
```

- [ ] **Step 2: Smoke test**

Upload a test image via a later task (or pre-seed the workspace manually: `cp plexus-frontend/public/favicon.ico /var/lib/plexus/workspace/{your-user-id}/test.png` — adjust MIME as needed). Navigate to the file — it should render inline.

Click a non-image, non-text file (e.g., a PDF if available) — it should show metadata + download button.

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(frontend): content pane image preview + binary download

Images (png/jpg/gif/webp) render inline via createObjectURL. Other
binaries show a metadata line (size + MIME) plus a download link
that bypasses the ArrayBuffer path entirely — letting the browser
stream the file directly.

Blob URLs are cleaned up on unmount to avoid memory leaks.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-13: Upload (click + drag-drop)

**Files:**
- Modify: `plexus-frontend/src/pages/Workspace.tsx`

- [ ] **Step 1: Add UploadDropZone component**

Append to `Workspace.tsx`:

```tsx
function UploadDropZone({ onUploaded }: { onUploaded: () => void }) {
  const [dragActive, setDragActive] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const enter = (e: DragEvent) => { e.preventDefault(); setDragActive(true); };
    const over = (e: DragEvent) => { e.preventDefault(); };
    const leave = (e: DragEvent) => {
      // Only reset when leaving the window entirely.
      if ((e.target as HTMLElement).nodeName === 'HTML') setDragActive(false);
    };
    const drop = (e: DragEvent) => {
      e.preventDefault();
      setDragActive(false);
      if (e.dataTransfer?.files) void uploadFiles(Array.from(e.dataTransfer.files));
    };
    window.addEventListener('dragenter', enter);
    window.addEventListener('dragover', over);
    window.addEventListener('dragleave', leave);
    window.addEventListener('drop', drop);
    return () => {
      window.removeEventListener('dragenter', enter);
      window.removeEventListener('dragover', over);
      window.removeEventListener('dragleave', leave);
      window.removeEventListener('drop', drop);
    };
  }, []);

  async function uploadFiles(files: File[]) {
    if (files.length === 0) return;
    setUploading(true);
    setError(null);
    try {
      const form = new FormData();
      for (const f of files) form.append('files', f, f.name);
      const res = await fetch('/api/workspace/upload', {
        method: 'POST',
        headers: { Authorization: `Bearer ${localStorage.getItem('token') ?? ''}` },
        body: form,
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${await res.text()}`);
      onUploaded();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'upload failed');
    } finally {
      setUploading(false);
    }
  }

  function onPick(e: React.ChangeEvent<HTMLInputElement>) {
    const files = Array.from(e.target.files ?? []);
    void uploadFiles(files);
    e.target.value = '';
  }

  return (
    <>
      <div
        className="border-t px-4 py-2 flex items-center gap-2"
        style={{ borderColor: 'var(--border)' }}
      >
        <label
          className="text-xs px-2 py-1 rounded cursor-pointer"
          style={{ border: '1px solid var(--border)' }}
        >
          {uploading ? 'Uploading…' : 'Upload'}
          <input type="file" multiple onChange={onPick} className="hidden" />
        </label>
        <span className="text-xs" style={{ color: 'var(--muted)' }}>
          or drop files anywhere on this page
        </span>
        {error && <span className="text-xs" style={{ color: '#ef4444' }}>{error}</span>}
      </div>
      {dragActive && (
        <div
          className="fixed inset-0 pointer-events-none z-50"
          style={{
            border: '3px dashed var(--accent)',
            background: 'rgba(57, 255, 20, 0.1)',
          }}
        />
      )}
    </>
  );
}
```

Wire into the page (below the main flex row):

```tsx
      </main>
      <UploadDropZone onUploaded={() => void refresh()} />
    </div>
```

- [ ] **Step 2: Smoke test**

Navigate to `/settings/workspace`. Drag a file from your desktop onto the browser. Expect a dashed green overlay. Drop — file uploads, tree refreshes, the new file appears under `uploads/`.

Click the "Upload" button and pick one file. Same behavior.

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(frontend): workspace multi-file upload (click + drag-drop)

Window-level drag listeners highlight the whole page with a dashed
green overlay during drag-over. Dropping the files kicks off a
multipart POST /api/workspace/upload; the tree + quota re-fetch
on success.

Click-to-upload via a hidden <input type="file" multiple> works
alongside drag-drop for users who prefer the button.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-14: Delete with ConfirmModal

**Files:**
- Create: `plexus-frontend/src/components/ConfirmModal.tsx`
- Modify: `plexus-frontend/src/pages/Workspace.tsx`

- [ ] **Step 1: Create ConfirmModal**

Create `plexus-frontend/src/components/ConfirmModal.tsx`:

```tsx
type Props = {
  open: boolean;
  title: string;
  message: string;
  confirmLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
  destructive?: boolean;
};

export function ConfirmModal({
  open, title, message, confirmLabel = 'Confirm', onConfirm, onCancel, destructive,
}: Props) {
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: 'rgba(0,0,0,0.5)' }}
      onClick={onCancel}
    >
      <div
        className="rounded p-6 max-w-md w-full"
        style={{ background: 'var(--card)', border: '1px solid var(--border)' }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="text-lg font-semibold mb-2">{title}</h2>
        <p className="text-sm mb-4" style={{ color: 'var(--muted)' }}>{message}</p>
        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="text-sm px-3 py-1 rounded"
            style={{ border: '1px solid var(--border)' }}
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            className="text-sm px-3 py-1 rounded"
            style={{
              background: destructive ? '#ef4444' : 'var(--accent)',
              color: 'var(--bg)',
            }}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Wire delete into ContentPane**

Add a delete button to `ContentPane`'s header row. At the top of `ContentPane`, import:

```tsx
import { ConfirmModal } from '../components/ConfirmModal';
```

Add state + handler:

```tsx
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);

  async function doDelete() {
    setDeleting(true);
    try {
      const url = `/api/workspace/file?path=${encodeURIComponent(path)}&recursive=true`;
      const res = await fetch(url, {
        method: 'DELETE',
        headers: { Authorization: `Bearer ${localStorage.getItem('token') ?? ''}` },
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${await res.text()}`);
      setConfirmDelete(false);
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'delete failed');
    } finally {
      setDeleting(false);
    }
  }
```

Add the button next to Edit:

```tsx
        {!inEditMode && (
          <button
            onClick={() => setConfirmDelete(true)}
            className="text-xs px-2 py-1 rounded"
            style={{ border: '1px solid var(--border)', color: '#ef4444' }}
          >
            Delete
          </button>
        )}
```

And the modal at the bottom of the component:

```tsx
      <ConfirmModal
        open={confirmDelete}
        title={`Delete ${path}?`}
        message="This cannot be undone. If this is a directory, its contents will be removed recursively."
        confirmLabel={deleting ? 'Deleting…' : 'Delete'}
        destructive
        onConfirm={doDelete}
        onCancel={() => setConfirmDelete(false)}
      />
```

Also: when a file is deleted, clear the selection. In the `onChanged` callback passed from `Workspace`, pass an updater that clears `selectedPath`:

```tsx
          <ContentPane
            path={selectedPath}
            onChanged={() => {
              void refresh();
              // Clear selection so the content pane doesn't try to reload a missing file.
              setParams({});
            }}
          />
```

- [ ] **Step 3: Smoke test**

Upload a test file (from B-13). Click it in the tree. Click "Delete". Expect a modal. Click "Delete" in the modal. Expect the file to disappear from the tree, quota to decrease, and the content pane to revert to the empty "Select a file…" state.

Test directory deletion: click a directory, click "Delete", confirm. Recursive delete should succeed.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(frontend): ConfirmModal + delete workflow for workspace files

Shared ConfirmModal primitive (in src/components/) used for
destructive actions. Workspace's ContentPane gains a red Delete
button that pops the modal; confirming sends DELETE with
recursive=true (covers the directory case without a separate
confirm tier).

On success the selection clears and the tree + quota refresh.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-15: Quick-access sidebar + Settings cleanup

**Files:**
- Modify: `plexus-frontend/src/pages/Workspace.tsx`
- Modify: `plexus-frontend/src/pages/Settings.tsx`

- [ ] **Step 1: Add quick-access to the tree pane**

In `Workspace.tsx`, update the left aside to include a quick-access list at the top:

```tsx
        <aside
          className="w-1/4 overflow-y-auto border-r p-2 flex flex-col gap-2"
          style={{ borderColor: 'var(--border)', background: 'var(--sidebar)' }}
        >
          <div className="flex flex-col gap-1">
            <div className="text-xs uppercase font-semibold" style={{ color: 'var(--muted)' }}>
              Quick access
            </div>
            {[
              { name: 'Soul', path: 'SOUL.md' },
              { name: 'Memory', path: 'MEMORY.md' },
              { name: 'Heartbeat Tasks', path: 'HEARTBEAT.md' },
            ].map((q) => (
              <button
                key={q.path}
                onClick={() => setParams({ path: q.path })}
                className="text-left text-sm px-1 py-0.5 rounded hover:opacity-70"
                style={{
                  background: selectedPath === q.path ? 'var(--accent)' : 'transparent',
                  color: selectedPath === q.path ? 'var(--bg)' : 'var(--text)',
                }}
              >
                📄 {q.name}
              </button>
            ))}
          </div>
          <hr style={{ borderColor: 'var(--border)' }} />
          {tree ? (
            <TreeView entries={tree} selected={selectedPath} onSelect={(path) => setParams({ path })} />
          ) : (
            <div style={{ color: 'var(--muted)' }}>Loading…</div>
          )}
        </aside>
```

- [ ] **Step 2: Remove Soul + Memory from Settings.tsx**

In `plexus-frontend/src/pages/Settings.tsx::ProfileTab`:

- Delete the `soul` and `memory` `useState` declarations.
- Delete the mount-time fetches for `/api/user/soul` and `/api/user/memory`.
- Delete the `saveSoul` and `saveMemory` functions.
- Delete the two textarea sections from the JSX.
- If the Tab type includes `'soul'` or `'memory'` variants, remove them.

Add one replacement paragraph in the ProfileTab JSX above the preserved display-name/email section:

```tsx
      <Section title="Soul & Memory">
        <p className="text-sm" style={{ color: 'var(--muted)' }}>
          Your soul (personality) and memory now live as editable Markdown files in your workspace.
          Edit them from the{' '}
          <a
            href="/settings/workspace?path=SOUL.md"
            className="underline"
            style={{ color: 'var(--accent)' }}
          >
            Workspace
          </a>{' '}
          page.
        </p>
      </Section>
```

- [ ] **Step 3: Rewire Skills tab to /api/workspace/skills**

In `Settings.tsx::SkillsTab`:

- Replace the `GET /api/skills` fetch with `GET /api/workspace/skills`.
- Change the local skill type to `WorkspaceSkill`.
- Remove the "Install from URL" button + handler (POST /api/skills/install is gone).
- Remove the "Create skill" form + handler (POST /api/skills is gone).
- Remove the DELETE handler (DELETE /api/skills/{name} is gone). Instead, add a note: "To delete a skill, visit the Workspace page and remove `skills/{name}/`."
- Replace the per-skill delete button with an "Edit" link that deep-jumps: `/settings/workspace?path=skills/{name}/SKILL.md`.

Simplified SkillsTab:

```tsx
function SkillsTab() {
  const [skills, setSkills] = useState<WorkspaceSkill[]>([]);
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => { void load(); }, []);

  async function load() {
    try {
      const data = await api.get<WorkspaceSkill[]>('/api/workspace/skills');
      setSkills(data);
    } catch (e) {
      setMsg(e instanceof Error ? e.message : 'load failed');
    }
  }

  return (
    <Section title="Skills">
      <p className="text-sm mb-4" style={{ color: 'var(--muted)' }}>
        Skills are Markdown files at <code>skills/{'{name}'}/SKILL.md</code> in your workspace.
        Create, edit, or delete them from the{' '}
        <a href="/settings/workspace?path=skills" className="underline" style={{ color: 'var(--accent)' }}>
          Workspace
        </a>{' '}
        page.
      </p>
      {msg && <div className="text-sm" style={{ color: '#ef4444' }}>{msg}</div>}
      <ul className="list-none p-0">
        {skills.map((s) => (
          <li
            key={s.name}
            className="flex items-center gap-2 py-1 border-b"
            style={{ borderColor: 'var(--border)' }}
          >
            <strong>{s.name}</strong>
            {s.always_on && <span className="text-xs" style={{ color: 'var(--accent)' }}>always-on</span>}
            <span className="text-sm flex-1" style={{ color: 'var(--muted)' }}>{s.description}</span>
            <a
              href={`/settings/workspace?path=skills/${s.name}/SKILL.md`}
              className="text-xs underline"
              style={{ color: 'var(--accent)' }}
            >
              Edit
            </a>
          </li>
        ))}
      </ul>
    </Section>
  );
}
```

Remember to import `WorkspaceSkill` from `../lib/types`.

- [ ] **Step 4: Typecheck + smoke**

```bash
cd plexus-frontend
pnpm typecheck
pnpm dev
```

Navigate to `/settings`:
- Profile tab: no more Soul/Memory textareas; instead, a small paragraph pointing to the Workspace page.
- Skills tab: clean list; each skill has an "Edit" link that opens the Workspace page with the SKILL.md selected.

Navigate to `/settings/workspace`:
- Left pane has "Quick access" at top with Soul/Memory/Heartbeat buttons.
- Clicking one selects the file in the content pane.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
feat(frontend): quick-access sidebar + Settings cleanup

Workspace page grows a top-of-tree "Quick access" section with
one-click jumps to SOUL.md / MEMORY.md / HEARTBEAT.md.

Settings.tsx is pared down:
- ProfileTab: Soul + Memory textareas removed (they hit 410 Gone
  since Plan A-17). A small paragraph replaces them, linking to
  the Workspace page.
- SkillsTab: rewired from the dropped /api/skills to
  /api/workspace/skills (frontmatter-parsed list). Install/Create/
  Delete UI removed — the Workspace page is the authoritative
  editor; SkillsTab only shows a read-only list with per-skill
  "Edit" deep-links.

Settings no longer has any 410-returning endpoints.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task B-16: ADR-37 + API.md + ISSUE sweep + Post-Plan footer

**Files:**
- Modify: `plexus-server/docs/DECISIONS.md`
- Modify: `plexus-server/docs/API.md`
- Modify: `plexus-server/docs/ISSUE.md`
- Modify: `docs/superpowers/plans/2026-04-18-workspace-frontend.md` (append Post-Plan Adjustments footer)

- [ ] **Step 1: ADR-37**

Append to `plexus-server/docs/DECISIONS.md`. Check the current latest ADR first (`grep -n "^## ADR-" plexus-server/docs/DECISIONS.md | tail -3` — latest should be ADR-36 from Plan E). Number this one ADR-37.

```markdown
## ADR-37: Workspace REST API + frontend file manager

**Date:** 2026-04-18
**Status:** Accepted
**Plan:** B (workspace frontend)

### Context

Plan A moved per-user state (soul, memory, skills, uploads) into a single `{WORKSPACE_ROOT}/{user_id}/` tree. The agent's server tools read and write this tree through `resolve_user_path` + `QuotaCache`. Plan A's cutover left the Settings page broken: its Soul + Memory textareas hit `PATCH /api/user/soul` / `PATCH /api/user/memory` which return 410 Gone, and the Skills tab's `/api/skills` endpoints also return 410. Users had no way to see or edit their workspace until Plan B landed.

### Decision

Add 7 REST endpoints under `/api/workspace/*` and one frontend page at `/settings/workspace`:

- `GET /api/workspace/quota` — `{used_bytes, total_bytes}`.
- `GET /api/workspace/tree` — flat sorted list of `{path, is_dir, size_bytes, modified_at}` under the user's root. Symlink escape rejected via canonicalized prefix check.
- `GET /api/workspace/file?path=…` — stream bytes with Content-Type sniff.
- `PUT /api/workspace/file?path=…` — write raw body bytes with quota enforcement.
- `DELETE /api/workspace/file?path=…&recursive=bool` — delete with recursive-directory gate.
- `POST /api/workspace/upload` — multipart upload; files land under `uploads/{YYYY-MM-DD}-{hash}-{filename}` matching the channel-adapter convention.
- `GET /api/workspace/skills` — parsed SKILL.md frontmatter (reuses Plan A's SkillsCache).

All path validation flows through `workspace::resolve_user_path` / `resolve_user_path_for_create` — the same sandbox the agent uses. Quota checks flow through `state.quota.check_and_reserve_upload`. No bypass path; no special-case frontend trust.

The frontend page is a single-file React component (`pages/Workspace.tsx`) with a tree pane (client-side tree-built from the flat server response), a content pane that dispatches on MIME (markdown via `react-markdown`, text as `<pre>`, images inline, binaries as download link), a quota bar with amber/red thresholds, drag-and-drop multi-file upload, and a quick-access sidebar for SOUL/MEMORY/HEARTBEAT.

The existing Settings page loses its Soul + Memory textareas and rewires the Skills tab from the dropped `/api/skills` to the new `/api/workspace/skills`. All 410-returning endpoints on the user flow are retired.

### Consequences

- **Positive:**
  - Single sandbox enforcement: agent tools and frontend share `resolve_user_path` — no second path-validation code to get wrong.
  - Quota state is global: the frontend quota bar reflects live state from the same counter the agent's writes touch.
  - Deep-linkable via `?path=…` — the Skills tab can link directly to `skills/{name}/SKILL.md`.
  - `react-markdown` is a single ~50 KB dep with no peer conflicts; the editor is a plain textarea so no code-editor dep is pulled in.
- **Negative:**
  - No rename endpoint in v1 (spec §7.2 listed rename as a UI action, but §7.3's endpoint list didn't include it). Users must delete + re-upload. Tracked in ISSUE.md.
  - Frontend has no automated test harness — all verification is manual. Noted in ISSUE.md as a post-M2 effort.
  - The upload "ERROR:{filename}" sentinel for failed partial uploads is a string-in-path hack; a cleaner shape would be `{path: string, size_bytes: number, error?: string}` but the current shape keeps the response monomorphic.

### Alternatives considered

- **Client-side rename via `upload-to-new + delete-old`:** rejected for this plan — silly for large binaries, and the spec doesn't require it. Re-add as an endpoint if users ask.
- **Separate frontend route per file type** (e.g., `/settings/memory`, `/settings/soul`): rejected — the single Workspace page is simpler and reflects the underlying "it's all just files" model.
- **TanStack Query for server-state caching:** overkill for a page with ~5 fetches. Local `useState` + manual refresh is fine.
```

- [ ] **Step 2: API.md**

Append to `plexus-server/docs/API.md`. If there's an existing endpoint-listing structure, match it. Otherwise, add a new section:

```markdown
## Workspace API

All endpoints require a valid user JWT. Paths in the `path` query param are relative to `{WORKSPACE_ROOT}/{user_id}/`. Traversal attempts return 403 Forbidden. Quota overflows return 413 Payload Too Large.

### GET /api/workspace/quota

Returns the user's current workspace usage and total quota.

**Response:** `{ "used_bytes": number, "total_bytes": number }` (200 OK)

### GET /api/workspace/tree

Returns a flat list of the user's workspace entries. Directories first, then alphabetical.

**Response:** `[{ "path": string, "is_dir": bool, "size_bytes": number, "modified_at": "RFC3339" }]` (200 OK)

### GET /api/workspace/file?path={path}

Streams the file's bytes. Content-Type sniffed from extension.

**Response:** raw bytes (200 OK), or 403/404 with `{ "error": string }`.

### PUT /api/workspace/file?path={path}

Body is raw bytes. Parent dirs are created as needed. Quota-checked.

**Response:** 204 No Content on success; 403/413/500 on failure.

### DELETE /api/workspace/file?path={path}&recursive={bool}

Deletes the file (or directory if `recursive=true`).

**Response:** 204 No Content; 400 if directory without recursive; 403 on traversal.

### POST /api/workspace/upload (multipart/form-data)

Accepts one or more files under any form-field name. Each is saved at `uploads/{date}-{hash}-{filename}`.

**Response:** `[{ "path": string, "size_bytes": number }]` — failed files appear with `path="ERROR:{filename}"`.

### GET /api/workspace/skills

Returns the user's skills as parsed frontmatter. Reuses the server's SkillsCache.

**Response:** `[{ "name": string, "description": string, "always_on": bool }]`.
```

- [ ] **Step 3: ISSUE.md**

Append to the `## Deferred` section under a new subsection:

```markdown
### Workspace Frontend (Plan B)

- **File rename endpoint** — spec §7.2 lists rename as a UI action, but §7.3's endpoint list omitted it. v1 requires delete + re-upload. Re-add a `POST /api/workspace/rename` with `{from, to}` if users ask.
- **Frontend test harness** — `plexus-frontend` has no Vitest/Jest/RTL/Playwright setup. Plan B's verification is manual smoke + visual review. Wiring up Vitest + React Testing Library is a post-M2 effort.
- **Bulk file operations** — multi-file select, bulk delete, bulk move. Single-file ops only in v1.
- **File-type coverage for inline preview** — SVG, HEIC, PDF, and video files currently fall through to the binary-metadata branch. Extending inline preview requires type-specific renderers (`<object>` for PDF, `<video>` for video, etc.).
- **Upload "ERROR:" sentinel** — the partial-success response shape uses a string prefix to indicate per-file failure. A cleaner shape would be `{path, size_bytes, error?: string}`. Both server and frontend would need to move together.
- **Server-pushed tree invalidation** — the tree refetches on every mutation initiated by the user in this tab, but an agent write (e.g., dream creating a new skill) doesn't push an invalidation. A WebSocket event or SSE push would let the tree auto-refresh while the page is open.
```

- [ ] **Step 4: Post-Plan Adjustments footer**

Append to `docs/superpowers/plans/2026-04-18-workspace-frontend.md`:

```markdown
---

## Post-Plan Adjustments

Deviations between the plan as written and the code that landed during execution. (Populate at the end of each task as needed.)

| Task | Deviation | Commit | Why |
|---|---|---|---|
| _pending_ | _pending_ | _pending_ | _pending_ |

## Commits map (Plan B)

| Plan step | Commits |
|---|---|
| B-1 | _tbd_ |
| B-2 | _tbd_ |
| B-3 | _tbd_ |
| B-4 | _tbd_ |
| B-5 | _tbd_ |
| B-6 | _tbd_ |
| B-7 | _tbd_ |
| B-8 | _tbd_ |
| B-9 | _tbd_ |
| B-10 | _tbd_ |
| B-11 | _tbd_ |
| B-12 | _tbd_ |
| B-13 | _tbd_ |
| B-14 | _tbd_ |
| B-15 | _tbd_ |
| B-16 | _this commit_ |
```

(Leave the tables stubbed — they're populated during execution. Plan E's footer is the reference shape.)

- [ ] **Step 5: Build + test (no-op — docs-only)**

```bash
cd /home/yucheng/Documents/GitHub/Plexus
cargo build --package plexus-server
cargo test --package plexus-server
cd plexus-frontend && pnpm typecheck
```

All still green.

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
docs: ADR-37 workspace API + ISSUE deferred items + Plan B footer

- DECISIONS.md: ADR-37 records the 7-endpoint REST surface, the
  single-page frontend shape, and the decision to defer rename.
- API.md: all 7 /api/workspace/* endpoints documented with
  request/response shapes and status codes.
- ISSUE.md: six Plan B follow-ups under Deferred — rename endpoint,
  frontend test harness, bulk ops, extended file-type preview,
  upload error-shape cleanup, push-based tree invalidation.
- 2026-04-18-workspace-frontend.md: Post-Plan Adjustments footer
  stub + commits map ready for execution-time population.

Plan B implementation is complete. The workspace page lets users
browse, view, edit, upload, and delete their own files through the
same sandbox the agent uses. Settings.tsx no longer has any
410-returning endpoints.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## 7. Completion Checklist

After all 16 tasks land, verify:

- [ ] `cargo build --workspace` clean.
- [ ] `cargo test --workspace` — all prior tests pass + new B-1..B-7 tests pass.
- [ ] `cd plexus-frontend && pnpm typecheck` clean.
- [ ] `pnpm build` produces a production bundle with no TypeScript errors.
- [ ] Live-browser walkthrough covers:
  - [ ] Navigate to `/settings/workspace` — page loads with quota bar + tree.
  - [ ] Click `SOUL.md` in the quick-access sidebar — markdown renders.
  - [ ] Click "Edit" — textarea appears with content; modify + Save — persists.
  - [ ] Drag a file from desktop onto the page — uploads into `uploads/`.
  - [ ] Click an image — inline preview appears.
  - [ ] Click a binary file — metadata + download link.
  - [ ] Click "Delete" — modal confirms; delete succeeds; tree refreshes.
  - [ ] Fill quota past 80% → amber; past 95% → red; past 100% → "workspace full" banner, writes rejected.
  - [ ] `/settings` Profile tab no longer has Soul/Memory textareas (paragraph link instead).
  - [ ] `/settings` Skills tab lists skills from `/api/workspace/skills`; each has an Edit deep-link.
- [ ] `ADR-37` landed in `plexus-server/docs/DECISIONS.md`.
- [ ] `API.md` has all 7 endpoints documented.
- [ ] `ISSUE.md` has the six Plan B follow-ups.
- [ ] This plan file's Post-Plan Adjustments footer is populated with any execution-time deviations.

At that point, Plan B is done and the A → C → D → E → B arc of the workspace/autonomy rewrite is complete. What remains is the M2 closeout backlog: account deletion, admin user-management UI, graceful-shutdown extension, session-list unread badge.
