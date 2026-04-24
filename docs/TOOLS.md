# Plexus — Tool Catalog

Authoritative spec for every tool surface available to the agent. Pairs with [DECISIONS.md](DECISIONS.md) (ADRs 038–048, 071, 075–088). When the implementation drifts from this doc, fix one or the other.

This is a *design* document. Use it during implementation as the source of truth for tool args, result shapes, and behaviors.

---

## Conventions

- **Source schemas are nanobot-shape.** Two patterns for how device-awareness shows up in source:
  - **Routing-only device** — for shared tools (`read_file`, `write_file`, etc.), `shell`, and MCP-wrapped tools, the source schema has **no device field at all**. At session tool-schema-build time, `tools_registry::build_tool_schemas` injects a `plexus_device` property (ADR-071) with an enum populated from connected install sites, and appends `plexus_device` to `required`.
  - **Intrinsic device** — for tools that natively operate across devices (`file_transfer`, `message`), the device field IS part of the source schema. `file_transfer` uses `plexus_src_device` + `plexus_dst_device`; `message` uses `plexus_device`. Each source stub has `enum: ["server"]`. At merge time, each such enum is **extended** with connected device names.
- **Reserved `plexus_` prefix.** The routing field name MUST use the `plexus_` prefix and MUST NOT be just `device` / `src_device` / `dst_device`. Why: the merger would otherwise clobber an MCP tool's native `device` arg (e.g., a tool selecting a GPU). The reserved prefix makes collision impossible.
- **Marker, not heuristic.** Every intrinsic-device field in a source schema carries `"x-plexus-device": true` (a JSON Schema extension). The merger detects device-routing fields by this marker, never by enum-shape guessing. The typed helper `plexus_device_field()` in `plexus-common/src/tools/device_field.rs` produces the canonical fragment — source-schema authors use it instead of hand-writing.
- **Tools_registry merge invariants:** the merge performs exactly one of two mutations per source schema:
  - **Inject:** add a brand-new `plexus_device` property (string, `enum` of install sites, marker `x-plexus-device: true`) and append `plexus_device` to `required`. Applies to routing-only tools.
  - **Extend:** for every property carrying `x-plexus-device: true`, replace its enum with the extended list of install sites. Applies to intrinsic-device tools.
  - Nothing else mutates. All other property names, types, descriptions, non-device enums, and the rest of `required` are strictly pass-through. See pseudocode in the Cross-cutting concerns section below.
- **Three crate locations for tool code:**
  - **Shared schemas** → `plexus-common/src/tools/<tool>.rs`
  - **Shared tool implementations** → both `plexus-server/src/tools/<tool>.rs` and `plexus-client/src/tools/<tool>.rs` (each side runs natively when the agent dispatches with the matching `device`).
  - **Server-only tools** → schema + implementation in `plexus-server/src/tools/<tool>.rs`
  - **Client-only tools** → schema + implementation in `plexus-client/src/tools/<tool>.rs`
- **Every tool implements the `Tool` trait** (ADR-077): `name`, `schema`, `max_output_chars` (default 16k via the trait), `execute`.
- **Default result cap is 16,000 characters** (ADR-076). Tools that need more override `max_output_chars`. Truncation is head-only with `\n... (truncated)` marker.
- **Timeouts are per-tool** (ADR-075). No central dispatcher wrapper. Some tools expose `timeout` in their schema (agent-tunable); others enforce internal-only timeouts.
- **Paths are absolute** (ADR-043). The first segment of a server-side path identifies the workspace; client-side paths reference the device's local filesystem.
- **Workspace writes funnel through `workspace_fs`** server-side (ADR-045). It owns quota check, SKILL.md validation, skills-cache invalidation, and symlink-escape protection.

---

## Inventory

| Name | Type | Source schema in | Implementation in | Purpose |
|------|------|------------------|-------------------|---------|
| `read_file` | shared | plexus-common | server + client | Read file content from a device's workspace |
| `write_file` | shared | plexus-common | server + client | Write file content; auto-create parent dirs |
| `edit_file` | shared | plexus-common | server + client | Replace text via 3-level fuzzy match |
| `delete_file` | shared | plexus-common | server + client | Remove a single file |
| `delete_folder` | shared | plexus-common | server + client | Recursively remove a folder and contents |
| `list_dir` | shared | plexus-common | server + client | List a directory's entries |
| `glob` | shared | plexus-common | server + client | Match files by glob pattern |
| `grep` | shared | plexus-common | server + client | Search file contents |
| `message` | server-only | plexus-server | plexus-server | Deliver text/media to a channel chat |
| `file_transfer` | server-only | plexus-server | plexus-server | Copy or move files within or across devices |
| `cron` | server-only | plexus-server | plexus-server | Schedule recurring or one-shot agent invocations |
| `web_fetch` | server-only | plexus-server | plexus-server | HTTP request with RFC-1918 block |
| `shell` | client-only | plexus-client | plexus-client | Execute a shell command on a device |
| `mcp_<server>_<tool>` | dynamic | (rmcp) | server or client side that hosts the MCP | Wrapped MCP-provided tool |

8 shared + 4 server-only + 1 client-only = 13 first-class tools, plus any number of MCP-wrapped tools.

---

## Shared tools

All shared tools accept a `plexus_device` argument (injected at merge time per ADR-071) selecting which workspace tree the operation targets:

- `plexus_device="server"` → routes to `workspace_fs` on the server. Path's first segment names the workspace (personal or shared).
- `plexus_device="<client_name>"` → dispatched over WebSocket to the named device, where the client-side implementation runs against the local filesystem inside `fs_policy` bounds.

### `read_file`

**Lives in:**
- Schema: `plexus-common/src/tools/read_file.rs`
- Server impl: `plexus-server/src/tools/read_file.rs`
- Client impl: `plexus-client/src/tools/read_file.rs`

**Purpose:** Read file content. The agent's primary way to inspect any file the user or system has stored.

**Source schema:**
```json
{
  "name": "read_file",
  "description": "Read the contents of a file. Returns up to ~32k tokens of content; large files may be truncated. Use list_dir / glob / grep for discovery before reading.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "Absolute path to the file."
      },
      "start_line": {
        "type": "integer",
        "description": "Optional 1-indexed line to start from. Default 1.",
        "minimum": 1
      },
      "end_line": {
        "type": "integer",
        "description": "Optional 1-indexed last line to include (inclusive). Default end of file.",
        "minimum": 1
      }
    },
    "required": ["path"],
    "additionalProperties": false
  }
}
```

**Mechanism:**
- **Server side:** path resolved against `PLEXUS_WORKSPACE_ROOT`. First path segment names the workspace; user must be authorized for that workspace (their personal workspace, or shared workspace they're a member of). Symlink resolution must remain inside the workspace boundary (ADR-072).
- **Client side:** path is absolute on the client's filesystem; if `fs_policy="sandbox"`, must resolve inside the device's `workspace_path`.
- **Blocked device paths** (per nanobot pattern): `/dev/zero`, `/dev/random`, `/dev/urandom`, `/dev/full`, `/dev/stdin/out/err`, `/dev/tty`, `/proc/<pid>/fd/[012]` — refuse read to avoid hangs.
- **Line slicing** is server-/client-side; `start_line`/`end_line` clamp at the file's actual line count.
- **Output** is the requested content as UTF-8. Binary content is returned as best-effort UTF-8 with a notice prepended if non-text bytes were present.

**Timeout:** 30s internal, no agent override (ADR-075).
**Result cap:** **128,000 characters** — overrides the global default (ADR-076). Reading a large file is the canonical case where the global 16k is too tight.
**Errors:** `WorkspaceError::NotFound`, `WorkspaceError::PermissionDenied`, `WorkspaceError::SymlinkEscape`, `WorkspaceError::BlockedPath`.
**Related ADRs:** 038 (shared schemas), 041 (device routing), 042 (path policy), 071 (merge), 072 (file ops are byte-level), 076 (result cap override).

---

### `write_file`

**Lives in:**
- Schema: `plexus-common/src/tools/write_file.rs`
- Server impl: `plexus-server/src/tools/write_file.rs`
- Client impl: `plexus-client/src/tools/write_file.rs`

**Purpose:** Write or replace a file's full content. Creates the file if it doesn't exist; replaces it entirely if it does.

**Source schema:**
```json
{
  "name": "write_file",
  "description": "Write content to a file. Creates the file (and any missing parent directories) if absent; overwrites existing content if present. For partial edits prefer edit_file.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "Absolute path to the file."
      },
      "content": {
        "type": "string",
        "description": "Full file content as UTF-8."
      }
    },
    "required": ["path", "content"],
    "additionalProperties": false
  }
}
```

**Mechanism:**
- **Implicit `mkdir -p`** on the path's parent (ADR-088). `tokio::fs::create_dir_all(path.parent())` runs before the write.
- **Server side:** routes through `workspace_fs::write` which performs (in order): workspace authorization, lock check (`SoftLocked` if `bytes_used > quota`), single-op cap (`UploadTooLarge` if `content.size > quota * 0.8`), the actual write, then `bytes_used` update.
- **SKILL.md validation:** if `path` matches `skills/*/SKILL.md` (exactly one level deep, exact filename), run the YAML-frontmatter validator before the write commits. Reject malformed input with `WorkspaceError::InvalidSkillFormat`. Folder name must match frontmatter `name` (ADR-082).
- **Skills cache invalidation:** any successful write under `skills/` invalidates the user's skills cache entry (ADR-085).
- **Client side:** subject to `fs_policy`; sandbox confines all writes to the device's `workspace_path`.

**Timeout:** 30s internal.
**Result cap:** 16,000 characters (default — usually a brief success message).
**Errors:** `WorkspaceError::SoftLocked`, `WorkspaceError::UploadTooLarge`, `WorkspaceError::InvalidSkillFormat`, `WorkspaceError::PermissionDenied`, `WorkspaceError::SymlinkEscape`.
**Related ADRs:** 045 (single write path), 078 (quota), 082 (SKILL.md validation), 085 (skills cache), 088 (mkdir -p).

---

### `edit_file`

**Lives in:**
- Schema: `plexus-common/src/tools/edit_file.rs`
- Server impl: `plexus-server/src/tools/edit_file.rs`
- Client impl: `plexus-client/src/tools/edit_file.rs`

**Purpose:** Replace text inside a file using nanobot's 3-level fuzzy matcher. Cheaper than rewriting the whole file with `write_file`. Also serves as a "create new file" shortcut when used with empty `old_text`.

**Source schema:**
```json
{
  "name": "edit_file",
  "description": "Replace old_text with new_text in the file. Uses fuzzy matching: exact substring, then line-trimmed match (handles indentation drift), then smart-quote normalization. Set replace_all=true if multiple matches are intentional. Special case: empty old_text on a non-existent file creates the file with new_text.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "Absolute path to the file." },
      "old_text": { "type": "string", "description": "Text to find. Empty string + non-existent path = create file." },
      "new_text": { "type": "string", "description": "Replacement text." },
      "replace_all": { "type": "boolean", "default": false, "description": "Replace every match instead of erroring on multiple matches." }
    },
    "required": ["path", "old_text", "new_text"],
    "additionalProperties": false
  }
}
```

**Mechanism:**
- **Three-level fuzzy match** (ADR-042), in order, lives in `plexus-common/src/tools/edit_file/matcher.rs` so server + client share it:
  1. Exact substring match.
  2. Line-trimmed sliding window — strips leading/trailing whitespace per line for the comparison while preserving original indentation in the replacement.
  3. Smart-quote normalization — treats `'`/`'`/`"`/`"` as equivalent to ASCII `'`/`"`.
- **Multiple matches:** if more than one match is found and `replace_all=false`, return `ToolError::AmbiguousEdit` listing match locations. With `replace_all=true`, replace every match.
- **Create-file shortcut:** `old_text=""` AND file doesn't exist → create file with `new_text`. Useful for one-call file creation while staying inside `edit_file` semantics.
- **Quota check on server:** computes `delta = new_text.len() - old_text.len()` (or `len(new_text)` for the create case); if positive, treats as a write of that many bytes for cap purposes. Refunds on shrink.
- **SKILL.md validation:** same rule as `write_file`. An edit to `skills/*/SKILL.md` runs the validator on the post-edit content; reject if invalid.
- **Skills cache invalidation:** same as write.

**Timeout:** 30s internal.
**Result cap:** 16,000 characters (typically a short confirmation + match locations).
**Errors:** `ToolError::AmbiguousEdit`, `ToolError::NoMatch`, `WorkspaceError::SoftLocked`, `WorkspaceError::UploadTooLarge`, `WorkspaceError::InvalidSkillFormat`.
**Related ADRs:** 042 (matcher), 045, 078, 082, 085.

---

### `delete_file`

**Lives in:**
- Schema: `plexus-common/src/tools/delete_file.rs`
- Server impl: `plexus-server/src/tools/delete_file.rs`
- Client impl: `plexus-client/src/tools/delete_file.rs`

**Purpose:** Remove a single file. Always allowed regardless of quota lock state (deletes only release space).

**Source schema:**
```json
{
  "name": "delete_file",
  "description": "Delete a single file. Use delete_folder for directories.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "Absolute path to the file." }
    },
    "required": ["path"],
    "additionalProperties": false
  }
}
```

**Mechanism:**
- **Server side:** routes through `workspace_fs::delete`. Reads file size, deletes via `tokio::fs::remove_file`, decrements `bytes_used`. If the path is a directory, return `ToolError::IsDirectory` (directs to `delete_folder`).
- **Symlink handling:** delete the link itself, never follow.
- **Skills cache invalidation:** if the deleted path is under `skills/`, invalidate the cache.
- **Lock interaction:** delete is allowed even when `bytes_used > quota_bytes` (ADR-078). Once usage drops back under, lock auto-lifts on next non-delete attempt.

**Timeout:** 10s internal.
**Result cap:** 16,000 characters.
**Errors:** `WorkspaceError::NotFound`, `ToolError::IsDirectory`.
**Related ADRs:** 078 (lock state), 045, 085.

---

### `delete_folder`

**Lives in:**
- Schema: `plexus-common/src/tools/delete_folder.rs`
- Server impl: `plexus-server/src/tools/delete_folder.rs`
- Client impl: `plexus-client/src/tools/delete_folder.rs`

**Purpose:** Recursively delete a folder and everything inside it. The companion to `delete_file` for tree-scoped removal.

**Source schema:**
```json
{
  "name": "delete_folder",
  "description": "Recursively delete a folder and all its contents. Always recursive — use delete_file for individual files.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "Absolute path to the folder." }
    },
    "required": ["path"],
    "additionalProperties": false
  }
}
```

**Mechanism:**
- **Always recursive, no flag** (ADR-086). The tool's only purpose is recursive deletion; a non-recursive variant is `rmdir` and too niche for v1.
- **Server side:** sums all file bytes inside the tree (single walk), removes via `tokio::fs::remove_dir_all`, applies one `bytes_used -= total` DB update. Lock auto-lifts if this brings usage under quota.
- **Client side:** subject to `fs_policy` like other writes. In sandbox mode, can only remove inside `workspace_path`.
- **Rejects** if `path` is a file (suggests `delete_file`) or doesn't exist.
- **Symlinks inside** the tree are unlinked, never followed outside.
- **Skills cache invalidation:** if the deleted path was `skills/` or under it, invalidate.

**Timeout:** 60s internal — recursive delete on large trees can take meaningful time.
**Result cap:** 16,000 characters.
**Errors:** `WorkspaceError::NotFound`, `ToolError::IsFile`.
**Related ADRs:** 078, 086.

---

### `list_dir`

**Lives in:**
- Schema: `plexus-common/src/tools/list_dir.rs`
- Server impl: `plexus-server/src/tools/list_dir.rs`
- Client impl: `plexus-client/src/tools/list_dir.rs`

**Purpose:** Enumerate a directory's contents. The agent's primary discovery tool; absolute paths require knowing what's there before reading or writing.

**Source schema:**
```json
{
  "name": "list_dir",
  "description": "List the contents of a directory. Set recursive=true to walk nested structure. Common noise directories (.git, node_modules, __pycache__, etc.) are auto-ignored.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "Absolute path to the directory." },
      "recursive": { "type": "boolean", "default": false, "description": "Recursively walk nested folders." },
      "max_entries": { "type": "integer", "default": 200, "minimum": 1, "description": "Maximum entries returned. Output is truncated with a note if exceeded." }
    },
    "required": ["path"],
    "additionalProperties": false
  }
}
```

**Mechanism:**
- **Auto-ignored noise dirs** (mirror of nanobot's list): `.git`, `node_modules`, `__pycache__`, `.venv`, `venv`, `dist`, `build`, `.tox`, `.mypy_cache`, `.pytest_cache`, `.ruff_cache`, `.coverage`, `htmlcov`.
- **Non-recursive output:** entries with a `📁 ` / `📄 ` prefix per entry (visual, LLM-friendly).
- **Recursive output:** flat list of relative paths, with trailing `/` for directories.
- **`max_entries` cap:** if exceeded, output truncated with `(truncated, showing first X of Y entries)` note.
- **Reject** if path doesn't exist or is a file (`ToolError::NotADirectory`).

**Timeout:** 10s internal.
**Result cap:** 16,000 characters.
**Errors:** `WorkspaceError::NotFound`, `ToolError::NotADirectory`.
**Related ADRs:** 042 (path policy).

---

### `glob`

**Lives in:**
- Schema: `plexus-common/src/tools/glob.rs`
- Server impl: `plexus-server/src/tools/glob.rs`
- Client impl: `plexus-client/src/tools/glob.rs`

**Purpose:** Find files by name pattern. Faster than recursive `list_dir` when looking for known shapes.

**Source schema:**
```json
{
  "name": "glob",
  "description": "Find files matching a glob pattern (e.g. **/*.rs, src/**/*.{ts,tsx}). Returns paths sorted by modification time, newest first.",
  "input_schema": {
    "type": "object",
    "properties": {
      "pattern": { "type": "string", "description": "Glob pattern, can use ** for recursive directory match." },
      "path": { "type": "string", "description": "Absolute base directory to search under. Required." }
    },
    "required": ["pattern", "path"],
    "additionalProperties": false
  }
}
```

**Mechanism:**
- Implementation uses `glob` crate or similar for pattern matching.
- Auto-ignores the same noise dirs as `list_dir`.
- Sorted by modification time, most recent first (helps the agent surface freshly-changed files).
- Cap at 200 results by default; if exceeded, truncate with note.

**Timeout:** 30s internal.
**Result cap:** 16,000 characters.
**Errors:** `WorkspaceError::NotFound`, `ToolError::InvalidGlob`.

---

### `grep`

**Lives in:**
- Schema: `plexus-common/src/tools/grep.rs`
- Server impl: `plexus-server/src/tools/grep.rs`
- Client impl: `plexus-client/src/tools/grep.rs`

**Purpose:** Search file contents by regex. Built on ripgrep semantics for speed and respect of ignore files.

**Source schema:**
```json
{
  "name": "grep",
  "description": "Search file contents by regex pattern. Powered by ripgrep — supports standard regex, file-type filtering, and ignore-file rules.",
  "input_schema": {
    "type": "object",
    "properties": {
      "pattern": { "type": "string", "description": "Regular expression to search for." },
      "path": { "type": "string", "description": "Absolute path to a file or directory to search in." },
      "output_mode": {
        "type": "string",
        "enum": ["content", "files_with_matches", "count"],
        "default": "files_with_matches",
        "description": "content: matching lines with context. files_with_matches: just file paths. count: per-file match counts."
      },
      "case_insensitive": { "type": "boolean", "default": false },
      "context": { "type": "integer", "default": 0, "description": "Lines of context around each match (only used in content mode)." },
      "glob": { "type": "string", "description": "Optional glob to restrict which files are searched (e.g. *.rs)." }
    },
    "required": ["pattern", "path"],
    "additionalProperties": false
  }
}
```

**Mechanism:**
- Implementation wraps `ripgrep` via the `grep`/`grep-regex`/`grep-searcher` crates, OR shells out to `rg` if installed (decide at impl time).
- Respects `.gitignore` and the noise-dir ignore list.
- `content` mode is the verbose default; nudge the agent toward `files_with_matches` or `count` for broad searches.
- Cap at 200 results by default.

**Timeout:** 60s internal — full-tree regex on large workspaces can take time.
**Result cap:** 16,000 characters.
**Errors:** `ToolError::InvalidRegex`, `WorkspaceError::NotFound`.

---

## Server-only tools

These four tools have no client-side counterpart. Their implementations live entirely in `plexus-server/src/tools/`. The agent reaches them by NOT specifying a `device` argument (or by the schema not having one), since they are inherently server-orchestrated.

### `message`

**Lives in:** `plexus-server/src/tools/message.rs`

**Purpose:** Deliver a text + media payload to a channel chat. The cross-channel reach mechanism (the within-session reply path uses the session's own `channel`/`chat_id` automatically per ADR-020).

**Source schema:**
```json
{
  "name": "message",
  "description": "Deliver a message to a specific channel + chat_id. Used for cross-channel reach (e.g., agent on Discord wants to also notify Telegram). For replying within the current conversation, just emit text — routing is automatic. This is the ONLY way to deliver files (images, documents, audio, video) to the user; do NOT use read_file to send files.",
  "input_schema": {
    "type": "object",
    "properties": {
      "channel": { "type": "string", "enum": ["discord", "telegram"], "description": "Channel to deliver to." },
      "chat_id": { "type": "string", "description": "Target chat identifier in the channel's namespace." },
      "content": { "type": "string", "description": "Message text." },
      "plexus_device": {
        "type": "string",
        "enum": ["server"],
        "description": "Device where the media files live. Defaults to server. All media paths in one call must come from this device.",
        "x-plexus-device": true
      },
      "media": {
        "type": "array",
        "items": { "type": "string" },
        "default": [],
        "description": "Optional list of absolute paths on `plexus_device` to attach as media."
      }
    },
    "required": ["channel", "chat_id", "content"],
    "additionalProperties": false
  }
}
```

**Merge-time injection:** `plexus_device.enum` is **extended** with currently-connected device names — e.g., post-merge it becomes `["server", "alice-laptop", "alice-phone"]`. Source schema stays as `["server"]` only. Detection is via the `x-plexus-device: true` marker, not enum shape (ADR-071).

**Mechanism:**
- Looks up the user's config for the target channel (`discord_configs` / `telegram_configs`); if none, return `ToolError::ChannelNotConfigured`.
- For each media path:
  - If `plexus_device="server"`: opens via `workspace_fs::read` (validates user authorization, symlink boundary). Handles base64-in-DB images per ADR-059 / ADR-044.
  - If `plexus_device="<client_name>"`: server fetches the file from the named client over the device WebSocket and forwards into the channel adapter. The file is not staged into the workspace (this is direct delivery; use `file_transfer` first if persistence is wanted).
- Emits as `Outbound::Final` with `channel`/`chat_id` set to the target. The corresponding channel adapter does the actual delivery.
- Returns delivery status per channel.

**Timeout:** 30s internal.
**Result cap:** 16,000 characters.
**Errors:** `ToolError::ChannelNotConfigured`, `WorkspaceError::NotFound`, `ToolError::DeliveryFailed`.
**Related ADRs:** 015 (Outbound shape), 020 (routing), 044 (workspace as media source), 090 (channel configs).

---

### `file_transfer`

**Lives in:** `plexus-server/src/tools/file_transfer.rs`

**Purpose:** Copy or move files (single files or whole folders) within or between devices. The unified cross-device byte mover.

**Source schema:**
```json
{
  "name": "file_transfer",
  "description": "Transfer a file or folder between devices, or rename within a single device. Source and destination can be any connected device (including the server). Use mode='copy' to leave source intact, mode='move' to remove source after successful transfer. Folder transfers are recursive. Destination is rejected if it already exists.",
  "input_schema": {
    "type": "object",
    "properties": {
      "plexus_src_device": {
        "type": "string",
        "enum": ["server"],
        "description": "Device where the source file or folder lives.",
        "x-plexus-device": true
      },
      "src_path": { "type": "string", "description": "Absolute path on plexus_src_device." },
      "plexus_dst_device": {
        "type": "string",
        "enum": ["server"],
        "description": "Device where the file or folder should land.",
        "x-plexus-device": true
      },
      "dst_path": { "type": "string", "description": "Absolute path on plexus_dst_device. Must not already exist." },
      "mode": {
        "type": "string",
        "enum": ["copy", "move"],
        "description": "copy: source intact. move: source deleted after successful transfer. Same-device move is atomic (rename). Cross-device move is copy-then-delete; if delete fails after a successful copy, both copies exist and the result flags a warning."
      }
    },
    "required": ["plexus_src_device", "src_path", "plexus_dst_device", "dst_path", "mode"],
    "additionalProperties": false
  }
}
```

**Merge-time injection:** both `plexus_src_device.enum` and `plexus_dst_device.enum` are **extended** with currently-connected device names. Post-merge example: `["server", "alice-laptop", "alice-phone"]` for both fields. Detection is via the `x-plexus-device: true` marker on each field, not enum shape.

**Mechanism:**
- Source schema lists only `["server"]` for both device fields. The merge step is what makes other devices selectable, exactly as the user's session has them connected.
- **Behavior matrix (ADR-087):**
  - Same-device, `copy`: native filesystem copy (`tokio::fs::copy` for files, walk for folders).
  - Same-device, `move`: `tokio::fs::rename` (atomic on the same filesystem; folder moves are O(1) directory-entry relinks).
  - Cross-device, `copy`: server orchestrates streaming pull-and-push over the device WebSockets; source intact.
  - Cross-device, `move`: same stream copy, then delete source on success. If delete fails, tool result includes a warning naming both surviving copies.
- **Folder semantics:** recursive. Cross-device folder transfer streams each entry; on mid-transfer failure, partial dst is cleaned up.
- **Quota:** applies when `plexus_dst_device="server"`. Reserve-then-write through `workspace_fs`; refund on move-from-server.
- **SKILL.md validation — applies to BOTH single-file and folder transfers:**
  - Before any bytes move, the server enumerates every destination path the transfer would produce.
  - Any path that would match `skills/*/SKILL.md` (exactly one level deep, exact filename, per ADR-082) has its source content validated up-front.
  - **Single-file transfer with malformed SKILL.md** → reject.
  - **Folder transfer with any malformed SKILL.md** → reject the **entire transfer atomically**. No partial copy lands. This closes the loophole where recursive folder transfer could smuggle invalid skills into the scanner path.
  - Non-SKILL.md files and files outside `skills/` are untouched by this validator.
- **Reject** if `dst_path` already exists, or `src_path` doesn't exist.

**Timeout:** stall-detection — abort if no bytes flow for 30s. Same-device move is atomic, returns instantly.
**Result cap:** 16,000 characters (typically a short status + byte count + warnings).
**Errors:** `WorkspaceError::NotFound`, `ToolError::DestinationExists`, `WorkspaceError::SoftLocked`, `WorkspaceError::UploadTooLarge`, `ToolError::TransferFailed`, `ToolError::PartialTransferDeleteFailed` (the warning case for cross-device move).
**Related ADRs:** 040 (server-only), 044, 045, 078, 082, 087.

---

### `cron`

**Lives in:** `plexus-server/src/tools/cron.rs`

**Purpose:** Schedule a recurring or one-shot agent invocation. The job fires by injecting a synthesized user message into a dedicated session per ADR-053.

**Source schema:**
```json
{
  "name": "cron",
  "description": "Schedule an agent invocation. Inserts a cron job that fires on the given schedule and creates a dedicated session for each firing. The reply lands on the channel + chat_id of the conversation where the cron was created (per ADR-053).",
  "input_schema": {
    "type": "object",
    "properties": {
      "schedule": {
        "type": "string",
        "description": "Cron expression (e.g. '0 9 * * *' for daily 9am) or natural-language ('every Monday at 10am'). Server parses both; rejects invalid input."
      },
      "description": {
        "type": "string",
        "description": "What the agent should do when the job fires. Becomes the synthesized user message at firing time."
      },
      "one_shot": {
        "type": "boolean",
        "default": false,
        "description": "If true, deletes itself after firing once."
      }
    },
    "required": ["schedule", "description"],
    "additionalProperties": false
  }
}
```

**Mechanism:**
- Inserts a row in `cron_jobs` with `user_id`, `channel`, `chat_id` (both from the calling session per ADR-053), `schedule` (parsed and stored as cron expression), `description`, `one_shot`, `last_fired_at` (NULL initially).
- A server-side ticker scans `cron_jobs` periodically, fires due jobs by synthesizing an `InboundMessage` with `session_key_override = "cron:<job_id>"` (ADR-010, ADR-012). The synthesized message's content is the `description` field.
- Each firing creates / continues a dedicated cron session; the reply routes to the channel + chat_id stored on the row.
- One-shot jobs delete their row after the firing inserts the synthesized message. Recurring jobs update `last_fired_at`.

**Timeout:** 10s — this is a DB write op, fast.
**Result cap:** 16,000 characters (typically a short success + the parsed schedule for confirmation).
**Errors:** `ToolError::InvalidSchedule`, `ToolError::DBError`.
**Related ADRs:** 010 (autonomous flows), 012 (synthesizers), 053 (cron channel/chat inheritance).

---

### `web_fetch`

**Lives in:** `plexus-server/src/tools/web_fetch.rs`

**Purpose:** Make an HTTP request from the server. Hardcoded SSRF protection blocks RFC-1918 / link-local / loopback / CGNAT.

**Source schema:**
```json
{
  "name": "web_fetch",
  "description": "Make an HTTP request and return the response. Server-side only; private network IPs are blocked. For network calls inside a private network, run shell on a client device with appropriate ssrf_whitelist instead.",
  "input_schema": {
    "type": "object",
    "properties": {
      "url": { "type": "string", "description": "Full URL including scheme. Only http:// and https:// allowed." },
      "method": {
        "type": "string",
        "enum": ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"],
        "default": "GET"
      },
      "headers": {
        "type": "object",
        "additionalProperties": { "type": "string" },
        "description": "Optional request headers."
      },
      "body": { "type": "string", "description": "Optional request body for POST/PUT/PATCH." }
    },
    "required": ["url"],
    "additionalProperties": false
  }
}
```

**Mechanism:**
- Parses the URL, resolves DNS, then checks the resolved IP against the hardcoded blocklist (ADR-052):
  - RFC-1918 (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16)
  - Link-local (169.254.0.0/16)
  - Loopback (127.0.0.0/8, ::1)
  - Carrier-grade NAT (100.64.0.0/10)
- Re-resolves before connecting (mitigates DNS rebinding) and verifies the actual connect-target IP against the blocklist.
- Uses `reqwest` with explicit timeouts (10s connect, 30s total).
- Returns `{status, headers, body}`. Body returned as text if Content-Type indicates text/JSON, else as base64 with a notice.
- Marks the response body as untrusted — the system prompt teaches the agent to treat fetched content as data, not instructions.

**Timeout:** 30s total, 10s connect.
**Result cap:** 16,000 characters.
**Errors:** `NetworkError::PrivateAddressBlocked`, `NetworkError::DNSFailed`, `NetworkError::Timeout`, `NetworkError::HttpError`.
**Related ADRs:** 052 (RFC-1918 block), 074 (untrusted content treatment).

---

## Client-only tools

### `shell`

**Lives in:** `plexus-client/src/tools/shell.rs`

**Purpose:** Execute a shell command on the device. The agent's escape hatch for everything not covered by file ops (git, build commands, system queries, network from inside a private network, etc.).

**Source schema:**
```json
{
  "name": "shell",
  "description": "Execute a shell command on the device. Subject to fs_policy: in sandbox mode, runs inside bwrap with workspace_path as the root. Environment is stripped to a minimal whitelist (PATH, HOME, LANG, TERM).",
  "input_schema": {
    "type": "object",
    "properties": {
      "cmd": { "type": "string", "description": "Shell command to run. Executed via /bin/sh -c." },
      "cwd": { "type": "string", "description": "Working directory. Defaults to the device's workspace_path. Must be inside workspace_path in sandbox mode." },
      "timeout": {
        "type": "integer",
        "description": "Timeout in seconds. Default 60. Max bounded by device.shell_timeout_max (admin-set per device).",
        "minimum": 1
      }
    },
    "required": ["cmd"],
    "additionalProperties": false
  }
}
```

**Merge-time injection:** `plexus_device` is added as a brand-new top-level property (carrying `x-plexus-device: true`) with an enum listing **only connected client devices** (no `"server"` — the server is not a code execution environment per ADR-072), and is appended to `required`. If no clients are connected, `shell` is omitted from the merged tool list entirely.

**Mechanism:**
- **fs_policy=sandbox (default, Linux):** wraps the command in `bwrap` per ADR-073:
  - `workspace_path` mounted read-write at workspace_path.
  - `/usr`, `/bin`, `/lib`, `/lib64`, `/etc/ssl/certs` mounted read-only (minimum to make subprocesses function).
  - Tmpfs everything else.
  - No `$HOME` access outside workspace, no host env access beyond the whitelist.
- **fs_policy=unrestricted:** runs the command directly with the client process's full privileges. Only set after the user types the device name to confirm (ADR-051).
- **Environment stripping** applies even in unrestricted mode: only `PATH`, `HOME`, `LANG`, `TERM` pass through. Secrets in `$GITHUB_TOKEN` etc. don't leak into agent-run subprocesses.
- **Output capture:** combined stdout + stderr. Both streamed; on timeout, process is killed (SIGTERM, then SIGKILL after 1s grace).
- **Result shape:** `{exit_code, stdout, stderr}` where stdout/stderr are truncated head-only to fit the result cap.

**Timeout:** agent-tunable. Default 60s. Max bounded by `device.shell_timeout_max` (admin-set per device, ADR-050). Schema's `timeout` field is the only agent-overridable timeout in Plexus.
**Result cap:** 16,000 characters (combined; head-only truncation per ADR-076).
**Errors:** `ToolError::ShellTimeout`, `ToolError::SandboxFailure`, `ToolError::CwdOutsideWorkspace`.
**Related ADRs:** 039 (client-only schema), 050 (per-device config), 051 (unrestricted confirmation), 073 (sandbox).

---

## MCP tools

MCP-provided tools are wrapped at install time and exposed to the agent through the same merge pipeline as native tools.

### Wrapping

For each MCP server `<server_name>` and each tool `<tool_name>` it advertises:

- **Wrapped name:** `mcp_<server_name>_<tool_name>` (ADR-048).
- **Source schema:** the MCP-provided schema is taken **as-is** — wrap is purely a name prefix. No parameter injection, no enum modification, no description rewrite at wrap time.
- **Merge-time injection:** at session tool-schema-build time, `plexus_device` is added as a brand-new top-level property (with `x-plexus-device: true`), enum listing every install site of this MCP, appended to `required` (same mechanism as the routing-only-device pattern for shared tools, ADR-071). The reserved `plexus_` prefix ensures no collision with any MCP tool's native args — even if an MCP advertises a field named `device`, the merger's injected field never overwrites it.
- **Lives in:** `plexus-common/src/mcp/` provides the wrapping. Server-side admin-installed MCPs are managed in `plexus-server/src/mcp/`; client-side per-device MCPs in `plexus-client/src/mcp/`.

**Worked example.** A tool `web_search` from MCP server `minimax` whose source schema is:

```json
{
  "name": "web_search",
  "input_schema": {
    "type": "object",
    "properties": { "query": { "type": "string" } },
    "required": ["query"]
  }
}
```

Post-wrap (name only) becomes `mcp_minimax_web_search` with the schema otherwise unchanged.

Post-merge, the agent sees:

```json
{
  "name": "mcp_minimax_web_search",
  "input_schema": {
    "type": "object",
    "properties": {
      "query": { "type": "string" },
      "plexus_device": {
        "type": "string",
        "enum": ["server", "alice-laptop"],
        "x-plexus-device": true,
        "description": "Which install site to execute on."
      }
    },
    "required": ["query", "plexus_device"]
  }
}
```

`plexus_device` enum lists every device (and "server" if admin-installed) where `minimax` is mounted. The agent picks one to dispatch to. The reserved `plexus_` prefix is the collision-proof guarantee: even if an MCP tool had its own `device` field (say, selecting a GPU), the merge step would not touch it.

### Schema-collision handling

If the same `mcp_<server>_<tool>` name is reported with different schemas across install sites (e.g. an admin-installed `mcp_github_*` and a per-device install of the same MCP at a different version), the install is rejected with HTTP 409 Conflict (ADR-049). User must rename one of the installs to keep both side-by-side.

### Dispatch

When the agent calls an MCP-wrapped tool, the server looks up which install site matches the `plexus_device` enum value and forwards the call to that site's `McpSession` (server-side or via a `ToolCall` frame to the client).

### Timeout

Per-MCP. The MCP's own session timeout governs; rmcp's defaults apply unless overridden in the MCP server's config.

### Related ADRs

047 (shared MCP client), 048 (naming), 049 (collision rejection), 071 (merge).

---

## Cross-cutting concerns

### Tool trait

Every tool implements:

```rust
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    fn max_output_chars(&self) -> usize { DEFAULT_MAX_TOOL_RESULT_CHARS }  // 16_000
    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}
```

`ToolContext` carries: `user_id`, `session_id`, `plexus_device` (for shared/MCP tools), and references to shared state (workspace_fs, channel registry, MCP manager).

### Schema merging at session start

Every agent-loop iteration step 4a (per ADR-021) calls `tools_registry::get_tool_schemas(user_id)`. The registry:

1. Lists all source schemas: shared tool schemas from `plexus-common`, server-only tools, client-side schemas advertised at handshake (`ClientToServer::RegisterTools`), MCP-wrapped schemas from both server and client sides.
2. Groups by `(fully_qualified_name, canonical_schema)`.
3. For each group, emits one merged schema:
   - Routing-only tools (shared, shell, MCP) have `plexus_device` injected as a new property (with `x-plexus-device: true` marker) with enum of install sites, and `plexus_device` appended to `required`.
   - Intrinsic-device tools (`file_transfer`, `message`) have every property carrying `x-plexus-device: true` — `plexus_src_device`/`plexus_dst_device` for `file_transfer`, `plexus_device` for `message` — extended with connected devices.
4. Source-schema collisions across install sites with the same name but different schemas → reject (logged, surfaced to admin/user via UI for MCP cases per ADR-049).

### Device-field helper + reserved name

Every device-routing field uses the reserved `plexus_` prefix and carries the `x-plexus-device: true` JSON Schema extension marker. A typed helper in `plexus-common/src/tools/device_field.rs` produces the canonical fragment:

```rust
pub const DEVICE_FIELD_NAME: &str = "plexus_device";

/// Use this to construct any device-routing field in a source schema.
pub fn plexus_device_field(description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "enum": ["server"],
        "description": description,
        "x-plexus-device": true
    })
}
```

The merger algorithm:

```python
def build_tool_schemas(user_id):
    connected = get_connected_devices(user_id)   # e.g. ["alice-laptop", "alice-phone"]
    merged = []

    # 1. Shared tools — inject plexus_device, enum = ["server"] + connected
    for tool in SHARED_TOOLS:
        s = deep_copy(tool.schema)
        inject_device_routing(s, sites=["server"] + connected)
        merged.append(s)

    # 2. Client-only tools (shell) — inject plexus_device, clients only (no "server")
    if connected:
        for tool in CLIENT_ONLY_TOOLS:
            s = deep_copy(tool.schema)
            inject_device_routing(s, sites=connected)
            merged.append(s)

    # 3. Server-only tools — extend any x-plexus-device field; pure server tools no-op
    for tool in SERVER_ONLY_TOOLS:
        s = deep_copy(tool.schema)
        extend_plexus_device_enums(s, extra=connected)
        merged.append(s)

    # 4. MCP tools — inject plexus_device, enum = install sites
    for group in collect_mcp_groups(user_id):
        if not all_canonical_schemas_match(group):
            reject_install(group)             # ADR-049 collision
            continue
        s = deep_copy(group.canonical_schema)
        inject_device_routing(s, sites=group.install_sites)
        merged.append(s)

    return merged


def inject_device_routing(schema, sites):
    """Add a brand-new plexus_device property; append to required."""
    schema["properties"]["plexus_device"] = {
        "type": "string",
        "enum": list(sites),
        "description": "Which install site to execute on.",
        "x-plexus-device": True,
    }
    schema["required"].append("plexus_device")


def extend_plexus_device_enums(schema, extra):
    """Extend every property marked x-plexus-device: true with extra device names."""
    for prop in schema["properties"].values():
        if prop.get("x-plexus-device") is True:
            prop["enum"] = prop["enum"] + list(extra)
```

The merger never inspects enum contents to decide what to mutate — only the explicit marker.

Cache is per-user `DashMap<user_id, Vec<MergedSchema>>`. Invalidates on device connect/disconnect, MCP install/uninstall, device config change.

### Result cap + truncation

- Default cap: `16_000` chars (ADR-076).
- Per-tool override via `max_output_chars()` — currently only `read_file` overrides (to 128k).
- Truncation is head-only with `\n... (truncated)` marker. Helper lives in `plexus-common/src/tools/truncate.rs` (single implementation).

### Timeout enforcement

- Decentralized per-tool (ADR-075). Each tool's `execute()` owns its own `tokio::time::timeout` wrapping.
- The dispatch layer does not impose a default timeout.
- Only `shell` (and some MCP tools) expose `timeout` in the schema for agent override; everything else has fixed internal timeouts as listed above.
- Runaway protection comes from the iteration hard cap (200, ADR-036) + trap-in-loop detection, NOT per-tool timeouts.

### Error model

All tools return errors via the `ToolResult` shape (per provider tool spec) with `is_error: true` and explanatory `content`. Typed errors in `plexus-common/src/errors/`:

- `WorkspaceError` — file ops, quota, paths.
- `ToolError` — tool-internal failures (timeout, ambiguous edit, transfer failures).
- `NetworkError` — web_fetch, MCP transport.
- `McpError` — MCP-specific.
- `ProtocolError` — wire-level.

Each implements `fn code(&self) -> ErrorCode` for the stable wire-level enum.

The agent sees errors as normal tool results and adapts on the next iteration (ADR-031). The loop never breaks on tool failure.

---

## What is explicitly NOT in the tool surface

- **Server-side `shell` / `exec` / `python` / `eval`** — by design, the server is not a code execution environment for the agent (ADR-072). Anything that needs to run is run on a client device.
- **`save_memory` / `edit_memory` / `update_soul`** — specialty tools dropped per Appendix A principle 1 ("generic over specialty"). MEMORY.md and SOUL.md are files, edited via `edit_file` / `write_file`.
- **`install_skill`** — dropped per ADR-084. Skills are installed via `file_transfer` from a client (where the user runs the installer) or via the web UI.
- **`read_skill`** — same. Skills are read via `read_file`.
- **`bulk_*` operations** — single-file ops only (ADR-067, superseded by ADR-087 for the rename case).
- **Server `web_fetch` with private-IP whitelist** — server SSRF block is hardcoded (ADR-052). Per-device whitelists exist for clients only.
- **`mkdir`** — implicit via `write_file` (ADR-088).
- **`rmdir`** — covered by `delete_folder` (no separate empty-only variant; too niche).

---

## Change discipline

When adding, removing, or modifying a tool:

1. Update this doc FIRST (the spec).
2. Update the relevant ADR(s) in `DECISIONS.md`. New tool = new ADR. Schema/behavior change = update existing ADR or add a successor.
3. Implement.
4. If the implementation deviates from the doc/ADR during coding, fix one or the other before merging.

The catalog and the ADRs are the source of truth. Code is always downstream.
