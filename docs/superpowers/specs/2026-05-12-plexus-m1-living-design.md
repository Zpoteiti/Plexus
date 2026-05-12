# Plexus M1 Living Design Spec

**Status:** living design, approved for M1 tracking
**Branch:** `rebuild-m1`
**Authors:** brainstormed in collaborative session 2026-05-12
**Supersedes:** none
**Last updated:** 2026-05-12

---

## 1. Purpose

M1 turns the M0 `plexus-common` foundation into a standalone `plexus-server`
product. At the end of M1, Plexus should support real users registering,
configuring the server, chatting with an agent through supported ingress
channels, using server-side tools, connecting devices over WebSocket, and
running cron and heartbeat flows according to the docs.

This document is a living tracker, not a full implementation plan. It records
the current M1 milestone map, dependency order, cross-cutting constraints, and
status. Each sub-milestone gets its own design sub-spec and implementation plan
before coding starts.

The milestone labels below are intentionally flexible. We preserve the `M1a`,
`M1b`, `M1c`, ... shape where practical, but implementation dependencies may
re-cut or reorder small pieces. When that happens, this document is updated
before moving on.

---

## 2. Current Snapshot

| Field | Value |
|---|---|
| Overall M1 state | M1a verified; M1b planning next |
| Current focus | Write the `M1b` LLM provider foundation sub-spec |
| Next implementation slice | `M1b` LLM provider foundation |
| Frontend scope | Out of M1; frontend remains M3 |
| Client scope | Standalone client remains M2, but M1 includes server-side device WebSocket support |
| Discord/Telegram | Required for M1; live tokens supplied by the user for smoke testing when ready |
| LLM credentials | Automated tests use a fake OpenAI-compatible provider; real key only for live smoke |

---

## 3. M1 Goals

M1 is the server milestone. It must produce a working server crate that can be
tested through REST, SSE, WebSocket, and channel adapter entry points.

In scope:

- `plexus-server` crate and executable.
- PostgreSQL-backed persistence using the canonical schema.
- Startup DB initialization for an empty database with `CREATE TABLE IF NOT EXISTS`.
- Authentication, admin configuration APIs, and user-facing REST APIs from `docs/API.yaml`.
- REST message ingress plus SSE delivery for browser sessions.
- OpenAI-compatible LLM provider layer, including admin validation and optional concurrency limiting.
- Server-side workspace/file APIs and server-side shared tools.
- Device token lifecycle and server-side device WebSocket protocol.
- Routing file and tool operations by `plexus_device`, with `server` as the built-in install-site name.
- Discord and Telegram adapters.
- Admin shared-service MCP and device MCP support.
- Cron scheduler and heartbeat delivery.
- Focused automated tests and live smoke paths.

Out of scope:

- M3 frontend implementation.
- M2 standalone client implementation.
- User-scoped server MCP.
- Session-scoped MCP.
- A production migration framework in M1.
- Non-OpenAI-compatible LLM protocols.
- Background job systems beyond the documented cron and heartbeat design.

---

## 4. Global Constraints

These constraints apply to every M1 sub-spec and implementation slice.

### 4.1 Contract Sources

- `docs/API.yaml` is the REST/SSE/admin contract.
- `docs/TOOLS.md` is the tool behavior contract.
- `docs/PROTOCOL.md` is the device WebSocket contract.
- `docs/SCHEMA.md` is the persistence contract.
- `docs/DECISIONS.md` is the ADR source.

Docs are rebuild specs. If implementation discovers a necessary design change,
update the relevant docs and this living tracker before treating the work as
complete.

### 4.2 Test Method

M1 uses API-first and e2e-first tests where practical. For persistence features,
tests must prove that REST writes actually land in PostgreSQL and can be read
back through the API or DB.

External services are not required for automated tests:

- LLM tests use a fake OpenAI-compatible HTTP service.
- Discord and Telegram use local adapter-level tests until live smoke.
- Device WebSocket tests may use an in-process test client.
- MCP tests use fake/local MCP servers where practical.

Real LLM credentials and real Discord/Telegram tokens are only used for live
smoke after the relevant implementation slice is complete.

### 4.3 Persistence

M1 does not introduce a migration framework. On server startup, if the database
has no usable tables, the server applies the canonical schema with
`CREATE TABLE IF NOT EXISTS`.

Schema drift is handled during development by updating `docs/SCHEMA.md` and the
canonical schema together. A migration framework can be revisited after M1 if
the project needs upgrade-in-place production releases.

### 4.4 Device Names and Routing

The literal device name `server` is reserved for the built-in install-site name.
Users cannot create or rename a device to `server`.

File REST APIs and file tools route by `plexus_device`, defaulting to `server`.
Client-device file operations route over device WebSocket and obey the target
device's `workspace_path` and `fs_policy`. Offline target devices return
`device_unreachable`.

Workspace transfer keeps explicit source/destination device fields:
`plexus_src_device` and `plexus_dst_device`.

### 4.5 LLM Provider

All LLM requests go through the shared provider layer. The default concurrency
mode is unlimited. Admins may configure a provider-level semaphore for weaker
providers or installations without a gateway.

When an admin changes LLM endpoint, API key, or model, the server validates the
configuration before writing it to the database:

- `GET {llm_endpoint}/models`
- reject unreachable endpoints
- reject unauthorized responses
- reject malformed model responses
- reject when configured `llm_model` is absent

### 4.6 MCP Tenancy

M1 supports only two MCP scopes:

- Admin shared-service MCP: configured by admins, shared credentials, available
  through `plexus_device="server"`, intended for stateless or low-state service
  tools.
- Device MCP: configured on a user device row, runs on that device, available
  through `plexus_device="<device-name>"`.

M1 explicitly excludes user-scoped server MCP and session-scoped MCP.

Admin shared-service MCP uses one runtime per configured MCP server with a
bounded per-MCP queue. There is no `pool_size` field in the M1 config contract.

### 4.7 Cron and Heartbeat

Cron writes can enter only through the agent tool path or the user REST API.
Both paths must share one write helper that validates the job and notifies the
ticker.

Cron delivery goes to the session that created the cron job. Heartbeat delivery
goes to a dedicated read-only heartbeat session for the user.

Cron drift should be rejected at write time where possible. The scheduler cap
scan covers all users and therefore uses a conservative interval rather than a
per-user fast loop.

---

## 5. Sub-Milestone Map

This is the initial sequence. It is expected to evolve as implementation
dependencies become clearer. The first map already applies one
dependency-driven re-cut: the LLM provider foundation comes before browser chat
because the chat path needs a provider contract.

| ID | Status | Scope | Depends on | Exit criteria |
|---|---|---|---|---|
| `M1a` | Verified | Server crate, startup, DB bootstrap, canonical schema application, real auth, basic REST/admin persistence, test harness | M0 | Verified by `cargo test -p plexus-server`, `cargo test --workspace`, `cargo fmt --all -- --check`, and `cargo check --workspace` |
| `M1b` | Planned | LLM provider foundation, admin config validation, fake provider test strategy, concurrency semaphore | `M1a` | Invalid provider config is rejected before DB write; valid fake provider can complete chat calls |
| `M1c` | Planned | Browser chat path: REST message ingress, session storage, SSE history replay/live stream, fake LLM-backed response loop | `M1a`, `M1b` | API test sends a message and receives agent response through SSE |
| `M1d` | Planned | Server workspace/file REST APIs, server-side workspace FS, quota reporting, server-side shared file tools | `M1a` | REST and tool tests create/read/edit/list server workspace files and report quota |
| `M1e` | Planned | Device token lifecycle, device naming rules, device WebSocket handshake/control frames | `M1a` | Device can be registered, connect over WS, and appear reachable |
| `M1f` | Planned | Device-routed file and tool execution over WS, offline handling, transfer plumbing | `M1d`, `M1e` | REST/tool call reaches connected test device; offline target returns `device_unreachable` |
| `M1g` | Planned | Discord and Telegram adapters | `M1b`, `M1c` | Adapter tests pass; live smoke works with user-provided bot tokens |
| `M1h` | Planned | Admin shared-service MCP and device MCP registration/execution | `M1e`, `M1f` | Fake MCP server exposes tools through `server`; device MCP tools route to a device |
| `M1i` | Planned | Cron scheduler and heartbeat delivery | `M1b`, `M1c` | Cron fires into creator session; heartbeat posts to read-only heartbeat session |
| `M1j` | Planned | Hardening, live smoke, docs sync, release readiness | All prior M1 slices | Full M1 smoke passes; docs and NotebookLM are current |

---

## 6. Sub-Spec Contract

Each M1 sub-spec must include:

- Goal and non-goals.
- API endpoints, tool contracts, or protocol frames touched.
- Data model changes and persistence behavior.
- Runtime components and ownership boundaries.
- Error handling and error codes.
- Test plan with automated tests first.
- Live smoke needs, if any.
- Exit criteria.
- Docs that must be updated after implementation.

Each sub-spec should be small enough to implement and verify in one focused
branch segment.

---

## 7. Status Rules

Allowed sub-milestone statuses:

- `Planned`: not designed in detail yet.
- `Designing`: sub-spec in progress.
- `Approved`: sub-spec approved; implementation plan next.
- `Implementing`: code in progress.
- `Verified`: automated checks and relevant smoke checks passed.
- `Blocked`: waiting on user input, credentials, dependency, or design decision.
- `Deferred`: intentionally moved out of the current sequence.

After each sub-milestone, update this document with:

- final status
- important deviations from the original plan
- test evidence
- follow-up work
- docs sync status

---

## 8. Current Handoff to M1b

`M1a` is verified. The server crate exists, empty PostgreSQL databases bootstrap
from the canonical schema, real auth works through REST, and the M1a admin
config subset persists to `system_config`.

The next document to write is the `M1b` LLM provider foundation sub-spec. Its
core proof should be that provider identity changes are validated through
`GET {llm_endpoint}/models` before DB write, valid fake OpenAI-compatible
providers can serve chat-completion calls, and the optional provider-level
concurrency semaphore is enforced in the shared provider layer.

M1a verification evidence:

- `cargo test -p plexus-server -- --nocapture`
- `cargo test --workspace`
- `cargo fmt --all -- --check`
- `cargo check --workspace`
- schema/docs consistency search for canonical tables, indexes, `server_mcp`,
  `llm_max_concurrent_requests`, and reserved `server` device-name constraint
