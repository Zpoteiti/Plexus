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
- **Path policy** (ADR-043): relative paths are accepted and resolve to the **personal workspace on the target device**. On server, that's `{PLEXUS_WORKSPACE_ROOT}/{user_id}/`; on a client, it's the device's `workspace_path`. Absolute paths are also accepted. **Shared workspaces always require absolute paths** (`/production_department/sprint.md`) — they have no implicit relative base.
- **Workspace writes funnel through `workspace_fs`** server-side (ADR-045). It owns quota check, SKILL.md validation, skills-cache invalidation, and symlink-escape protection.
- **All file tools enforce a workspace boundary in code** (ADR-073, file-tool jail). Every shared file tool (`read_file`, `write_file`, `edit_file`, `delete_file`, `delete_folder`, `list_dir`, `glob`, `grep`, `notebook_edit`) calls `resolve_in_workspace()` in `plexus-common/src/tools/path.rs` before any disk op — canonicalize → check `starts_with(workspace_root)` → reject with `WorkspaceError::PathOutsideWorkspace` otherwise. This is OS-agnostic Rust and active when `fs_policy="sandbox"` (default). It is **the only thing keeping file tools in their lane on macOS and Windows**, where the bwrap subprocess jail doesn't exist. Lifted only when the device is `fs_policy="unrestricted"` (typed-name confirmation per ADR-051).
- **Every tool result is wrapped** (ADR-095): the content string returned to the LLM is prefixed with `[untrusted tool result]: ` at construction time. Uniform across all tools — web_fetch body, exec stdout, read_file output, MCP response, everything. The wrap is the signal; no system-prompt rule.

---

## Inventory

| Name | Type | Source schema in | Implementation in | Purpose |
|------|------|------------------|-------------------|---------|
| `read_file` | shared | plexus-common | server + client | Read file content (text/image/PDF/office doc) |
| `write_file` | shared | plexus-common | server + client | Write file content; auto-create parent dirs |
| `edit_file` | shared | plexus-common | server + client | Replace text via 3-level fuzzy match |
| `delete_file` | shared | plexus-common | server + client | Remove a single file (Plexus addition) |
| `delete_folder` | shared | plexus-common | server + client | Recursively remove a folder + contents (Plexus addition) |
| `list_dir` | shared | plexus-common | server + client | List a directory's entries |
| `glob` | shared | plexus-common | server + client | Find files by glob pattern |
| `grep` | shared | plexus-common | server + client | Search file contents |
| `notebook_edit` | shared | plexus-common | server + client | Edit Jupyter notebook cells |
| `web_fetch` | shared | plexus-common | server + client | HTTP fetch — server has hardcoded private-IP block, clients have default block + per-device whitelist exceptions (ADR-052) |
| `message` | server-only | plexus-server | plexus-server | Deliver text/media/buttons to a channel chat |
| `file_transfer` | server-only | plexus-server | plexus-server | Copy or move files within/across devices (Plexus addition) |
| `cron` | server-only | plexus-server | plexus-server | Add/list/remove scheduled agent invocations |
| `exec` | client-only | plexus-client | plexus-client | Execute a shell command on a device |
| `mcp_<server>_<tool>`, `mcp_<server>_resource_<name>`, `mcp_<server>_prompt_<name>` | dynamic | (rmcp) | wherever the MCP is installed | Wrapped MCP capabilities — tools, resources, prompts (ADR-048) |

10 shared + 3 server-only + 1 client-only = 14 first-class tools, plus any number of MCP-wrapped tools.

Schemas below are the **source** schemas (what gets written in code). The agent sees these plus the merger's additions per ADR-071 (`plexus_device` property on routing-only tools, enum extension on intrinsic-device tools).

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

**Purpose:** Read a file (text, image, or document). Line-based pagination for large text files; PDF/DOCX/XLSX/PPTX parsing built-in; images returned as `image_url` blocks.

**Source schema (matches nanobot):**
```json
{
  "name": "read_file",
  "description": "Read a file (text, image, or document). Text output format: LINE_NUM|CONTENT. Images return visual content for analysis. Supports PDF, DOCX, XLSX, PPTX documents. Use offset and limit for large text files. Reads exceeding ~128K chars are truncated.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "The file path to read"
      },
      "offset": {
        "type": "integer",
        "description": "Line number to start reading from (1-indexed, default 1)",
        "minimum": 1
      },
      "limit": {
        "type": "integer",
        "description": "Maximum number of lines to read (default 2000)",
        "minimum": 1
      },
      "pages": {
        "type": "string",
        "description": "Page range for PDF files, e.g. '1-5' (default: all, max 20 pages)"
      }
    },
    "required": ["path"]
  }
}
```

**Mechanism (nanobot-aligned):**
- Path resolution follows ADR-043: relative paths resolve to the target device's personal workspace root; absolute paths are used as-is. Server-side, absolute is required for shared workspaces.
- **Default text response:** `limit=2000` lines, output prefixed `LINE_NUM| <line>`. Tail includes `(Showing lines X-Y of Z. Use offset=X+1 to continue.)` — self-documenting pagination.
- **128k char hard cap** applied on top of line-based limit; safety net for pathological line lengths.
- **Blocked device paths** (nanobot pattern): `/dev/zero`, `/dev/random`, `/dev/urandom`, `/dev/full`, `/dev/stdin/out/err`, `/dev/tty`, `/proc/<pid>/fd/[012]` — refused to avoid hangs.
- **PDFs:** text extraction via `pages` arg; max 20 pages per call.
- **Office docs** (`.docx`/`.xlsx`/`.pptx`): text extraction via built-in parsers.
- **Images** (detected by mime): returned as `image_url` content blocks, not text.
- **Dedup:** if the file's `mtime` + `offset` + `limit` are unchanged since the last read, return `[File unchanged since last read: path]` instead of full content — saves tokens on idempotent re-reads.
- Tool result is wrapped by the shared helper with `[untrusted tool result]: ` per ADR-095 before reaching the LLM.

**Timeout:** 30s internal, no agent override (ADR-075).
**Result cap:** 128,000 characters (ADR-076 override).
**Errors:** `WorkspaceError::NotFound`, `WorkspaceError::PermissionDenied`, `WorkspaceError::SymlinkEscape`, `WorkspaceError::BlockedPath`.
**Related ADRs:** 038, 041, 043, 071, 072, 076, 095.

---

### `write_file`

**Lives in:**
- Schema: `plexus-common/src/tools/write_file.rs`
- Server impl: `plexus-server/src/tools/write_file.rs`
- Client impl: `plexus-client/src/tools/write_file.rs`

**Purpose:** Write or replace a file's full content. Creates the file if it doesn't exist; replaces it entirely if it does.

**Source schema (matches nanobot):**
```json
{
  "name": "write_file",
  "description": "Write content to a file. Overwrites if the file already exists; creates parent directories as needed. For partial edits, prefer edit_file instead.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "The file path to write to"
      },
      "content": {
        "type": "string",
        "description": "The content to write"
      }
    },
    "required": ["path", "content"]
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

**Source schema (matches nanobot):**
```json
{
  "name": "edit_file",
  "description": "Edit a file by replacing old_text with new_text. Tolerates minor whitespace/indentation differences and curly/straight quote mismatches. If old_text matches multiple times, you must provide more context or set replace_all=true. Shows a diff of the closest match on failure.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "The file path to edit" },
      "old_text": { "type": "string", "description": "The text to find and replace" },
      "new_text": { "type": "string", "description": "The text to replace with" },
      "replace_all": { "type": "boolean", "description": "Replace all occurrences (default false)" }
    },
    "required": ["path", "old_text", "new_text"]
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

**Purpose:** Enumerate a directory's contents. The agent's primary discovery tool before reading or writing files.

**Source schema (matches nanobot):**
```json
{
  "name": "list_dir",
  "description": "List the contents of a directory. Set recursive=true to explore nested structure. Common noise directories (.git, node_modules, __pycache__, etc.) are auto-ignored.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "The directory path to list" },
      "recursive": { "type": "boolean", "description": "Recursively list all files (default false)" },
      "max_entries": { "type": "integer", "description": "Maximum entries to return (default 200)", "minimum": 1 }
    },
    "required": ["path"]
  }
}
```

**Mechanism:**
- Path resolution per ADR-043.
- **Auto-ignored noise dirs** (mirror of nanobot's list): `.git`, `node_modules`, `__pycache__`, `.venv`, `venv`, `dist`, `build`, `.tox`, `.mypy_cache`, `.pytest_cache`, `.ruff_cache`, `.coverage`, `htmlcov`.
- **Non-recursive output:** entries with a `📁 ` / `📄 ` prefix per entry (visual, LLM-friendly).
- **Recursive output:** flat list of relative paths, with trailing `/` for directories.
- **`max_entries` cap:** if exceeded, output truncated with `(truncated, showing first X of Y entries)` note.
- **Reject** if path doesn't exist or is a file (`ToolError::NotADirectory`).

**Timeout:** 10s internal.
**Result cap:** 16,000 characters.
**Errors:** `WorkspaceError::NotFound`, `ToolError::NotADirectory`.
**Related ADRs:** 043 (path policy), 095 (result wrap).

---

### `glob`

**Lives in:**
- Schema: `plexus-common/src/tools/glob.rs`
- Server impl: `plexus-server/src/tools/glob.rs`
- Client impl: `plexus-client/src/tools/glob.rs`

**Purpose:** Find files by name pattern. Faster than recursive `list_dir` when looking for known shapes. Sorted by modification time (newest first).

**Source schema (matches nanobot, full arg set):**
```json
{
  "name": "glob",
  "description": "Find files matching a glob pattern (e.g. '*.py', 'tests/**/test_*.py'). Results are sorted by modification time (newest first). Skips .git, node_modules, __pycache__, and other noise directories.",
  "input_schema": {
    "type": "object",
    "properties": {
      "pattern": {
        "type": "string",
        "description": "Glob pattern to match, e.g. '*.py' or 'tests/**/test_*.py'",
        "minLength": 1
      },
      "path": {
        "type": "string",
        "description": "Directory to search from (default '.')"
      },
      "max_results": {
        "type": "integer",
        "description": "Legacy alias for head_limit",
        "minimum": 1,
        "maximum": 1000
      },
      "head_limit": {
        "type": "integer",
        "description": "Maximum number of matches to return (default 250)",
        "minimum": 0,
        "maximum": 1000
      },
      "offset": {
        "type": "integer",
        "description": "Skip the first N matching entries before returning results",
        "minimum": 0,
        "maximum": 100000
      },
      "entry_type": {
        "type": "string",
        "enum": ["files", "dirs", "both"],
        "description": "Whether to match files, directories, or both (default files)"
      }
    },
    "required": ["pattern"]
  }
}
```

**Mechanism:**
- `path` defaults to `.` (which per ADR-043 means the target's personal workspace root).
- Auto-ignores the same noise dirs as `list_dir`.
- Results sorted by mtime, newest first.
- `head_limit` (default 250) caps total results; `offset` skips the first N for paginated scroll-through.
- `max_results` is a legacy alias for `head_limit`.
- `entry_type` controls whether the match returns files, directories, or both.

**Timeout:** 30s internal.
**Result cap:** 16,000 characters.
**Errors:** `WorkspaceError::NotFound`, `ToolError::InvalidGlob`.
**Related ADRs:** 043, 095.

---

### `grep`

**Lives in:**
- Schema: `plexus-common/src/tools/grep.rs`
- Server impl: `plexus-server/src/tools/grep.rs`
- Client impl: `plexus-client/src/tools/grep.rs`

**Purpose:** Regex content search across files. Built on ripgrep semantics for speed and respect of ignore files.

**Source schema (matches nanobot, full arg set):**
```json
{
  "name": "grep",
  "description": "Search file contents with a regex pattern. Default output_mode is files_with_matches (file paths only); use content mode for matching lines with context. Skips binary and files >2 MB. Supports glob/type filtering.",
  "input_schema": {
    "type": "object",
    "properties": {
      "pattern": {
        "type": "string",
        "description": "Regex or plain text pattern to search for",
        "minLength": 1
      },
      "path": {
        "type": "string",
        "description": "File or directory to search in (default '.')"
      },
      "glob": {
        "type": "string",
        "description": "Optional file filter, e.g. '*.py' or 'tests/**/test_*.py'"
      },
      "type": {
        "type": "string",
        "description": "Optional file type shorthand, e.g. 'py', 'ts', 'md', 'json'"
      },
      "case_insensitive": {
        "type": "boolean",
        "description": "Case-insensitive search (default false)"
      },
      "fixed_strings": {
        "type": "boolean",
        "description": "Treat pattern as plain text instead of regex (default false)"
      },
      "output_mode": {
        "type": "string",
        "enum": ["content", "files_with_matches", "count"],
        "description": "content: matching lines with optional context; files_with_matches: only matching file paths; count: matching line counts per file. Default: files_with_matches"
      },
      "context_before": {
        "type": "integer",
        "description": "Number of lines of context before each match",
        "minimum": 0,
        "maximum": 20
      },
      "context_after": {
        "type": "integer",
        "description": "Number of lines of context after each match",
        "minimum": 0,
        "maximum": 20
      },
      "max_matches": {
        "type": "integer",
        "description": "Legacy alias for head_limit in content mode",
        "minimum": 1,
        "maximum": 1000
      },
      "max_results": {
        "type": "integer",
        "description": "Legacy alias for head_limit in files_with_matches or count mode",
        "minimum": 1,
        "maximum": 1000
      },
      "head_limit": {
        "type": "integer",
        "description": "Maximum number of results to return. In content mode this limits matching line blocks; in other modes it limits file entries. Default 250",
        "minimum": 0,
        "maximum": 1000
      },
      "offset": {
        "type": "integer",
        "description": "Skip the first N results before applying head_limit",
        "minimum": 0,
        "maximum": 100000
      }
    },
    "required": ["pattern"]
  }
}
```

**Mechanism:**
- Wraps ripgrep (via `grep`/`grep-regex`/`grep-searcher` crates, or shells out to `rg` if installed).
- Skips binary files and files >2 MB automatically.
- Respects `.gitignore` and the noise-dir ignore list.
- `output_mode=files_with_matches` is the default — favor it for broad searches to stay scoped.
- `fixed_strings=true` escapes regex metacharacters (treat pattern as literal text).
- `type` accepts ripgrep's shorthands (e.g. `py`, `ts`, `md`, `json`).

**Timeout:** 60s internal — full-tree regex on large workspaces can take time.
**Result cap:** 16,000 characters.
**Errors:** `ToolError::InvalidRegex`, `WorkspaceError::NotFound`.
**Related ADRs:** 043, 095.

---

### `notebook_edit`

**Lives in:**
- Schema: `plexus-common/src/tools/notebook_edit.rs`
- Server impl: `plexus-server/src/tools/notebook_edit.rs`
- Client impl: `plexus-client/src/tools/notebook_edit.rs`

**Purpose:** Edit a Jupyter notebook (`.ipynb`) cell — replace source, insert a new cell after an index, or delete an existing cell.

**Source schema (matches nanobot):**
```json
{
  "name": "notebook_edit",
  "description": "Edit a Jupyter notebook (.ipynb) cell. Modes: replace (default) replaces cell content, insert adds a new cell after the target index, delete removes the cell at the index. cell_index is 0-based.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "Path to the .ipynb notebook file" },
      "cell_index": { "type": "integer", "description": "0-based index of the cell to edit", "minimum": 0 },
      "new_source": { "type": "string", "description": "New source content for the cell" },
      "cell_type": { "type": "string", "description": "Cell type: 'code' or 'markdown' (default: code)", "enum": ["code", "markdown"] },
      "edit_mode": { "type": "string", "description": "Mode: 'replace' (default), 'insert' (after target), or 'delete'", "enum": ["replace", "insert", "delete"] }
    },
    "required": ["path", "cell_index"]
  }
}
```

**Mechanism:**
- Parses the notebook JSON, operates on the specified cell, writes the modified notebook back through `workspace_fs` on server (so quota + SKILL.md validation edge cases still apply if someone puts a SKILL.md-shaped file inside a .ipynb, though that's an odd case).
- `edit_mode=replace` (default): replaces `source` of cell at `cell_index`. `new_source` required in this mode.
- `edit_mode=insert`: inserts a new cell AFTER `cell_index`. `cell_type` optional (default `code`). `new_source` required.
- `edit_mode=delete`: removes cell at `cell_index`. `new_source` / `cell_type` ignored.

**Timeout:** 30s internal.
**Result cap:** 16,000 characters.
**Errors:** `WorkspaceError::NotFound`, `ToolError::InvalidNotebook`, `ToolError::CellIndexOutOfRange`.
**Related ADRs:** 043, 095.

---

### `web_fetch`

**Lives in:**
- Schema: `plexus-common/src/tools/web_fetch.rs`
- Server impl: `plexus-server/src/tools/web_fetch.rs` — applies the unconditional server block-list.
- Client impl: `plexus-client/src/tools/web_fetch.rs` — applies the same default block-list, plus per-device `ssrf_whitelist` exceptions.

**Purpose:** Fetch a URL and extract readable content (HTML → markdown/text). Available on server and any connected client; the agent picks the dispatch site via `plexus_device` (ADR-052). Use the server site for public URLs; use a client site to reach declared internal services in the user's network (e.g. an internal company API at `10.180.20.30:8080`).

**Source schema (matches nanobot):**
```json
{
  "name": "web_fetch",
  "description": "Fetch a URL and extract readable content (HTML → markdown/text). Output is capped at maxChars (default 50 000). Works for most web pages and docs; may fail on login-walled or JS-heavy sites.",
  "input_schema": {
    "type": "object",
    "properties": {
      "url": {
        "type": "string",
        "description": "URL to fetch"
      },
      "extractMode": {
        "type": "string",
        "enum": ["markdown", "text"],
        "default": "markdown"
      },
      "maxChars": {
        "type": "integer",
        "minimum": 100
      }
    },
    "required": ["url"]
  }
}
```

**Merge-time injection:** standard shared-tool injection — `plexus_device` is added with enum = `["server"] + connected_clients`. The agent picks where the fetch dispatches.

**Mechanism:**
- Both sites parse the URL, resolve DNS, then check the resolved IP against the policy. Re-resolve before connecting (mitigates DNS rebinding) and verify the actual connect-target IP against the policy.
- **Server site — block-list (no exception):**
  - RFC-1918 (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16)
  - 100.64.0.0/10 carrier-grade NAT (covers Tailscale's 100.x range)
  - Link-local (169.254.0.0/16)
  - Loopback (127.0.0.0/8, ::1)
  - IPv6 ULA `fc00::/7` and link-local `fe80::/10`
- **Client site:** same default block-list, plus `device.ssrf_whitelist` (host or `host:port` entries) overrides the block. Whitelist editable via `PATCH /api/devices/{name}/config`.
- **Capability declaration, not sandbox** (ADR-052, ADR-073): the client whitelist is a declarative tool path for known internal services. The agent can still bypass it via `exec curl <ip>` because `bwrap` does not isolate network. Documented as such.
- Fetches via `reqwest`, 10s connect + 30s total timeout. Uses a readability extractor (jina/readability-style) to convert HTML → `extractMode` output. Output capped at `maxChars` (default 50,000, agent-overridable).
- Tool result content is wrapped per ADR-095 with `[untrusted tool result]: ` before the LLM sees it — uniform with all other tool results.

**Timeout:** 30s total, 10s connect.
**Result cap:** 50,000 characters (tool's own cap via `maxChars`). Shared 16k global cap (ADR-076) doesn't apply — web_fetch's cap is explicit in schema.
**Errors:** `NetworkError::PrivateAddressBlocked`, `NetworkError::WhitelistMiss`, `NetworkError::DNSFailed`, `NetworkError::Timeout`, `NetworkError::HttpError`.
**Related ADRs:** 050 (per-device config), 052 (block-list + per-device whitelist), 073 (network-not-isolated), 074 (untrusted content treatment), 095 (result wrap).

---

## Server-only tools

These three tools have no client-side counterpart. Their implementations live entirely in `plexus-server/src/tools/`. The agent reaches them by NOT specifying a `plexus_device` argument (or by the schema not having one), since they are inherently server-orchestrated.

### `message`

**Lives in:** `plexus-server/src/tools/message.rs`

**Purpose:** Send a message to the user, optionally with file attachments or inline keyboard buttons. `content` is required; `channel` and `chat_id` default to the current session's values. Specify them explicitly for cross-channel reach.

**Source schema (matches nanobot, with `plexus_device` added for multi-device media sources):**
```json
{
  "name": "message",
  "description": "Send a message to the user, optionally with file attachments. This is the ONLY way to deliver files (images, documents, audio, video) to the user. Use the 'media' parameter with file paths to attach files. Do NOT use read_file to send files — that only reads content for your own analysis.",
  "input_schema": {
    "type": "object",
    "properties": {
      "content": {
        "type": "string",
        "description": "The message content to send"
      },
      "channel": {
        "type": "string",
        "description": "Optional: target channel (telegram, discord, etc.). Defaults to current session's channel."
      },
      "chat_id": {
        "type": "string",
        "description": "Optional: target chat/user ID. Defaults to current session's chat_id."
      },
      "plexus_device": {
        "type": "string",
        "enum": ["server"],
        "description": "Device where the media files live. Defaults to server. All media paths in one call must come from this device.",
        "x-plexus-device": true
      },
      "media": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Optional: list of file paths to attach (images, audio, documents)"
      },
      "buttons": {
        "type": "array",
        "items": {
          "type": "array",
          "items": {
            "type": "string",
            "description": "Button label"
          }
        },
        "description": "Optional: inline keyboard buttons as list of rows, each row is list of button labels."
      }
    },
    "required": ["content"]
  }
}
```

**Merge-time injection:** `plexus_device.enum` is extended with currently-connected device names. Source stays as `["server"]`. Detection via `x-plexus-device: true` marker (ADR-071).

**Mechanism:**
- **Routing (ADR-020):**
  - If `channel` + `chat_id` omitted → delivers to the current session's channel + chat_id. Equivalent target as a direct text reply, but with access to `media` / `buttons`.
  - If `channel` + `chat_id` specified → delivers to that target. Cross-channel reach.
- Looks up the user's config for the target channel (`discord_configs` / `telegram_configs`); if none, returns `ToolError::ChannelNotConfigured`.
- For each media path:
  - If `plexus_device="server"`: opens via `workspace_fs::read` (validates user authorization, symlink boundary). Handles base64-in-DB images per ADR-059 / ADR-044.
  - If `plexus_device="<client_name>"`: server fetches the file from the named client over the device WebSocket and forwards into the channel adapter. The file is not staged into the workspace (this is direct delivery; use `file_transfer` first if persistence is wanted).
- `buttons` renders as inline keyboard rows on channels that support it (Telegram, Discord's button components); plain text channels ignore the param with no error.
- Emits as `Outbound::Final` with `channel`/`chat_id` set to the resolved target.

**Timeout:** 30s internal.
**Result cap:** 16,000 characters.
**Errors:** `ToolError::ChannelNotConfigured`, `WorkspaceError::NotFound`, `ToolError::DeliveryFailed`.
**Related ADRs:** 015 (Outbound shape), 020 (routing + defaults), 044 (workspace as media source), 090 (channel configs), 095 (result wrap).

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

**Purpose:** Schedule reminders and recurring tasks. A single tool with an `action` enum — add, list, or remove jobs. Each firing injects a synthesized user message into a dedicated cron session per ADR-053.

**Source schema (matches nanobot):**
```json
{
  "name": "cron",
  "description": "Schedule reminders and recurring tasks. Actions: add, list, remove. If tz is omitted, cron expressions and naive ISO times default to UTC.",
  "input_schema": {
    "type": "object",
    "properties": {
      "action": {
        "type": "string",
        "description": "Action to perform",
        "enum": ["add", "list", "remove"]
      },
      "name": {
        "type": "string",
        "description": "Optional short human-readable label for the job (e.g., 'weather-monitor', 'daily-standup'). Defaults to first 30 chars of message."
      },
      "message": {
        "type": "string",
        "description": "REQUIRED when action='add'. Instruction for the agent to execute when the job triggers (e.g., 'Send a reminder to WeChat: xxx' or 'Check system status and report'). Not used for action='list' or action='remove'."
      },
      "every_seconds": {
        "type": "integer",
        "description": "Interval in seconds (for recurring tasks)"
      },
      "cron_expr": {
        "type": "string",
        "description": "Cron expression like '0 9 * * *' (for scheduled tasks)"
      },
      "tz": {
        "type": "string",
        "description": "Optional IANA timezone for cron expressions (e.g. 'America/Vancouver'). When omitted with cron_expr, the tool's default timezone applies."
      },
      "at": {
        "type": "string",
        "description": "ISO datetime for one-time execution (e.g. '2026-02-12T10:30:00'). Naive values use the tool's default timezone."
      },
      "deliver": {
        "type": "boolean",
        "description": "Whether to deliver the execution result to the user channel (default true)",
        "default": true
      },
      "job_id": {
        "type": "string",
        "description": "REQUIRED when action='remove'. Job ID to remove (obtain via action='list')."
      }
    },
    "required": ["action"],
    "description": "Action-specific parameters: add requires a non-empty message plus one schedule (every_seconds, cron_expr, or at); remove requires job_id; list only needs action. Per-action requirements are enforced at runtime (see field descriptions) so the top-level schema stays compatible with providers (e.g. OpenAI Codex/Responses) that reject oneOf/anyOf/allOf/enum/not at the root of function parameters."
  }
}
```

**Mechanism:**
- **`action="add"`** — requires `message` plus exactly one of `every_seconds`, `cron_expr`, or `at`. Inserts a row in `cron_jobs` with `user_id`, `channel`, `chat_id` (from the calling session per ADR-053), the schedule parameters, `message`, `name`, `deliver`, `tz`. Returns the created row's `job_id` and a human-readable confirmation.
- **`action="list"`** — returns a summary of the user's cron jobs: `job_id`, `name`, schedule (as stored), next-fire estimate, `last_fired_at`.
- **`action="remove"`** — requires `job_id`. Deletes the row (and cancels pending fires).
- A server-side ticker scans `cron_jobs` periodically, fires due jobs by synthesizing an `InboundMessage` with `session_key_override = "cron:<job_id>"` (ADR-010, ADR-012). The synthesized message's `content` is the job's `message` field. If the job has `at` (one-shot), the row is deleted after firing; otherwise `last_fired_at` updates.
- Each firing creates / continues a dedicated cron session; the reply routes to the channel + chat_id stored on the row. If `deliver=false`, the result is logged to the cron session but not delivered to the user-facing channel.

**Timeout:** 10s — DB write ops, fast.
**Result cap:** 16,000 characters.
**Errors:** `ToolError::InvalidSchedule`, `ToolError::MissingRequiredField`, `ToolError::DBError`, `ToolError::CronJobNotFound`.
**Related ADRs:** 010 (autonomous flows), 012 (synthesizers), 053 (cron channel/chat inheritance), 095 (result wrap).

---

## Client-only tools

### `exec`

**Lives in:** `plexus-client/src/tools/exec.rs`

**Purpose:** Execute a shell command on the device. The agent's escape hatch for everything not covered by file ops (git, build commands, system queries, network from inside a private network, etc.). Renamed from `shell` for nanobot alignment.

**Source schema (matches nanobot):**
```json
{
  "name": "exec",
  "description": "Execute a shell command and return its output. Prefer read_file/write_file/edit_file over cat/echo/sed, and grep/glob over shell find/grep. Use -y or --yes flags to avoid interactive prompts. Output is truncated at 10 000 chars; timeout defaults to 60s.",
  "input_schema": {
    "type": "object",
    "properties": {
      "command": { "type": "string", "description": "The shell command to execute" },
      "working_dir": { "type": "string", "description": "Optional working directory for the command" },
      "timeout": {
        "type": "integer",
        "description": "Timeout in seconds. Increase for long-running commands like compilation or installation (default 60, max 600).",
        "minimum": 1,
        "maximum": 600
      }
    },
    "required": ["command"]
  }
}
```

**Merge-time injection:** `plexus_device` is added as a brand-new top-level property (carrying `x-plexus-device: true`) with an enum listing **only connected client devices** (no `"server"` — the server is not a code execution environment per ADR-072), and is appended to `required`. If no clients are connected, `exec` is omitted from the merged tool list entirely.

**Mechanism:**
- **fs_policy=sandbox (default, Linux):** wraps the command in `bwrap` per ADR-073. Mount list matches nanobot exactly, all `--ro-bind-try`:
  - `/usr`, `/bin`, `/lib`, `/lib64` — binaries + dynamic linker.
  - `/etc/alternatives` — Debian alternatives symlinks.
  - `/etc/ssl/certs` — TLS root certificates.
  - `/etc/resolv.conf` — DNS resolution (without this, `curl example.com` fails inside the jail).
  - `/etc/ld.so.cache` — dynamic linker cache.

  Plus: `workspace_path` bind-mounted read-write at its same absolute path; `tmpfs` over the client config directory (`~/.config/plexus/` per ADR-073) so the device token is unreadable from the jail; `tmpfs` everywhere else. No `$HOME` access outside the workspace.
- **Network is NOT isolated** (ADR-073). `curl`, `pip install`, `npm install` work from inside the jail. The sandbox blocks filesystem exfiltration, not network probes.
- **fs_policy=unrestricted:** runs the command directly with the client process's full privileges. Only set after the user types the device name to confirm (ADR-051).
- **Environment stripping** applies even in unrestricted mode: only `PATH`, `HOME`, `LANG`, `TERM` pass through. Secrets in `$GITHUB_TOKEN` etc. don't leak into agent-run subprocesses.
- **Output capture:** combined stdout + stderr. Both streamed; on timeout, process is killed (SIGTERM, then SIGKILL after 1s grace).
- **Result shape:** `{exit_code, stdout, stderr}` where stdout/stderr are truncated head-only per the cap.
- Tool result content is wrapped per ADR-095.

**Timeout:** agent-tunable via `timeout` field. Default 60s. Max 600s (nanobot-aligned) AND bounded further by `device.shell_timeout_max` (admin-set per device, ADR-050) when that's smaller. This is the only agent-overridable timeout in Plexus.
**Result cap:** 10,000 characters (nanobot's cap for exec; combined stdout+stderr, head-only truncation).
**Errors:** `ToolError::ExecTimeout`, `ToolError::SandboxFailure`, `ToolError::CwdOutsideWorkspace`.
**Related ADRs:** 039 (client-only schema), 050 (per-device config), 051 (unrestricted confirmation), 073 (sandbox), 095 (result wrap).

---

## MCP tools, resources, prompts

MCP servers advertise three capability surfaces — **tools**, **resources**, **prompts**. Plexus wraps all three uniformly into the per-user tool registry (ADR-047), so the agent sees one flat list of callable entries. Naming by surface (ADR-048):

| Surface | Wrapped name | Action when called |
|---|---|---|
| Tool | `mcp_<server>_<tool_name>` | `call_tool(name, args)` |
| Resource | `mcp_<server>_resource_<resource_name>` | `read_resource(uri)` (URI built from agent args per ADR-099) |
| Prompt | `mcp_<server>_prompt_<prompt_name>` | `get_prompt(name, args)` → text-joined messages per ADR-048 |

The typed infixes (`_resource_` / `_prompt_`) make cross-surface name collisions impossible by construction. Tools stay unprefixed for back-compat with the original ADR-048 convention.

### Wrapping — tools

- **Source schema:** the MCP-provided `input_schema` is taken **as-is** — wrap is purely a name rewrite.
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

Post-wrap becomes `mcp_minimax_web_search`, source schema unchanged. Post-merge, the agent sees:

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

`plexus_device` enum lists every site where `minimax` is mounted. The reserved `plexus_` prefix is the collision-proof guarantee: even if an MCP tool had its own `device` field (say, selecting a GPU), the merge step would not touch it.

### Wrapping — resources

- **Source schema:** auto-generated from the resource's URI (ADR-099). Static URIs produce a zero-arg schema; URI templates are parsed for `{var}` placeholders, each becoming a required `string` property.
- **At call time:** the wrapper substitutes agent-supplied values back into the URI before invoking `read_resource`. Static resources call `read_resource` with the literal URI.
- **Merge-time injection:** identical to tools — `plexus_device` injected at the top level.

**Worked example — static URI.** Resource `index` at `notion://workspace/index` from MCP server `notion`:

```json
{
  "name": "mcp_notion_resource_index",
  "input_schema": { "type": "object", "properties": {}, "required": [] }
}
```

Post-merge adds `plexus_device` (the only required arg). Calling it returns the resource's content as `tool_result`.

**Worked example — URI template.** Resource `page` at `notion://page/{page_id}`:

```json
{
  "name": "mcp_notion_resource_page",
  "input_schema": {
    "type": "object",
    "properties": {
      "page_id": { "type": "string", "description": "URI template variable: page_id" }
    },
    "required": ["page_id"]
  }
}
```

Post-merge adds `plexus_device`. Agent calls `mcp_notion_resource_page(page_id="abc", plexus_device="server")` → wrapper computes `notion://page/abc` → `read_resource("notion://page/abc")`.

If a template variable name collides with the reserved `plexus_device`, wrapping fails at install time with a clear error (the MCP author renames the placeholder).

### Wrapping — prompts

- **Source schema:** auto-generated from the prompt's `arguments` array. Each `{name, description, required}` becomes a string property; `required` flag is honored.
- **At call time:** invokes `get_prompt(name, args)`. The result is a list of `PromptMessage` objects; the wrapper extracts text content from every message and joins with `"\n"` (matches nanobot `mcp.py:408–421`). Non-text content blocks are stringified via `Display`. Empty result → `"(no output)"`.
- **Merge-time injection:** identical to tools.

**Worked example.** Prompt `code_review` from MCP server `helper` with arguments `[{name:"language", required:true}, {name:"style", required:false}]`:

```json
{
  "name": "mcp_helper_prompt_code_review",
  "input_schema": {
    "type": "object",
    "properties": {
      "language": { "type": "string" },
      "style": { "type": "string" }
    },
    "required": ["language"]
  }
}
```

Calling returns the rendered prompt messages as a single concatenated string in the `tool_result.content` (then wrapped per ADR-095).

### Collision handling

Two cases, both rejected at install time (ADR-049):

1. **Within-server dup** — same MCP server advertises two capabilities that wrap to the same name (only intra-surface dups can fire, since `_resource_` / `_prompt_` infixes prevent cross-surface collisions). Plexus diverges from nanobot's silent overwrite — install is rejected with a structured error.
2. **Cross-install schema drift** — same wrapped name (e.g. `mcp_minimax_web_search`) is reported with a different schema across install sites. Returns `409 Conflict` with a structured diff body covering all three surfaces. User renames one of the installs to keep both side-by-side.

### Dispatch

When the agent calls any MCP-wrapped entry, the server looks up which install site matches the `plexus_device` enum value and forwards the call to that site's `McpSession` (server-side or via a `tool_call` frame to the client). Resources and prompts dispatch identically to tools.

### `enabled` filter (ADR-100)

Each MCP server config carries an optional `enabled: [<glob>...]` allow-list, matched against the post-wrap name. When present, only matching entries register; when absent, every advertised capability registers (default-allow). Single field, three surfaces.

Examples:
- `enabled: ["mcp_notion_*"]` — every notion entry (tools, resources, prompts).
- `enabled: ["mcp_notion_search", "mcp_notion_resource_*"]` — the `search` tool plus every resource.
- `enabled: ["mcp_*_resource_*"]` — every resource from every MCP, no tools or prompts.

### Timeout

Per-MCP. The MCP's own session timeout governs; rmcp's defaults apply unless overridden in the MCP server's config. Same for tools, resources, prompts.

### Related ADRs

047 (shared MCP client + three surfaces), 048 (naming + prompt-output convention), 049 (collision rejection), 071 (merge), 099 (URI template expansion), 100 (`enabled` filter).

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

    # 2. Client-only tools (exec) — inject plexus_device, clients only (no "server")
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

    # 4. MCP entries (tools + resources + prompts) — inject plexus_device,
    #    enum = install sites. ADR-048 typed-infix naming makes the surface
    #    irrelevant here; merger treats every entry uniformly.
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
- Only `exec` (and some MCP tools) expose `timeout` in the schema for agent override; everything else has fixed internal timeouts as listed above.
- Runaway protection comes from the iteration hard cap (200, ADR-036) + trap-in-loop detection, NOT per-tool timeouts.

### Untrusted tool result wrap

Every tool result's `content` string is prefixed with `[untrusted tool result]: ` at construction time, before the `tool_result` block reaches the LLM. Uniform across shared tools, server-only tools, client-only tools, and MCP-wrapped tools. Shared helper in `plexus-common/src/tools/result.rs`.

```rust
// plexus-common/src/tools/result.rs
pub const UNTRUSTED_TOOL_RESULT_PREFIX: &str = "[untrusted tool result]: ";

pub fn wrap_result(raw: &str) -> String {
    format!("{UNTRUSTED_TOOL_RESULT_PREFIX}{raw}")
}
```

The wrap is the signal. No system-prompt rule needed — the agent learns structurally from seeing the prefix, the same way it learned the channel-inbound wrap `[untrusted message from X]:` (ADR-007). See ADR-095 for the decision rationale.

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

- **Server-side `exec` / `python` / `eval`** — by design, the server is not a code execution environment for the agent (ADR-072). Anything that needs to run is run on a client device.
- **`save_memory` / `edit_memory` / `update_soul`** — specialty tools dropped per Appendix A principle 1 ("generic over specialty"). MEMORY.md and SOUL.md are files, edited via `edit_file` / `write_file`.
- **`install_skill`** — dropped per ADR-084. Skills are installed via `file_transfer` from a client (where the user runs the installer) or via the web UI.
- **`read_skill`** — same. Skills are read via `read_file`.
- **`bulk_*` operations** — single-file ops only (ADR-067, superseded by ADR-087 for the rename case).
- **Server `web_fetch` with private-IP whitelist** — server site of `web_fetch` has an unconditional block-list (ADR-052). The per-device `ssrf_whitelist` only applies to the client site.
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
