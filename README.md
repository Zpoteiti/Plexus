# Plexus

Distributed AI agent system. Rebuild from scratch, informed by what worked and what didn't in the prior codebase.

## What this is

Plexus separates **orchestration** (which lives on a server) from **execution** (which runs on connected client devices). An LLM-powered ReAct agent receives user messages across multiple channels (browser chat via REST/SSE, Discord, Telegram), invokes tools on the server or dispatches them to the user's devices, and delivers responses back through the same channels.

Inspired by (but not affiliated with) [nanobot](https://github.com/obot-platform/nanobot) — many design patterns are adopted directly.

## Architecture at a glance

```
                         ┌─────────────────────────────┐
 Browser (REST+SSE)────▶ │                             │
 Discord (SDK)────────▶  │        plexus-server        │───────▶ LLM provider
 Telegram (SDK)───────▶  │                             │       (OpenAI/Anthropic/
                         │ • REST API                  │        local/etc.)
                         │ • SSE outbound streams      │
                         │ • PostgreSQL                │◀──WS──▶ plexus-client
                         │ • Agent loop + context      │          (user's device)
                         │ • Channels + tools registry │            • shell tool
                         │ • Workspace_fs              │            • file tools (local)
                         │ • MCP coordinator           │            • bwrap sandbox
                         │ • Frontend static files     │            • MCP client
                         │ • JWT issuance              │
                         └─────────────────────────────┘
```

**Three Rust crates:**
- **plexus-common** — shared types: protocol, errors, tool schemas, fuzzy matcher, network policy, MCP client, MIME detection.
- **plexus-server** — orchestration hub: agent loop, channels, REST API, SSE, device WebSocket, DB, workspace, JWT, frontend hosting.
- **plexus-client** — user device: tool execution, bwrap sandbox, MCP client, WS connection.

**Plus a React frontend** (`plexus-frontend`), compiled by Vite and either embedded into the server binary (release) or proxied by Vite dev server in development.

## Milestones

| Milestone | Scope |
|---|---|
| **M0** | `plexus-common` — protocol types, errors, tool schemas, shared utilities (fuzzy matcher, network policy, MCP wrapper). Foundation for the other crates. |
| **M1** | `plexus-client` — tool executor, bwrap sandbox, MCP client. Connects to server over WebSocket. 100% nanobot-shaped file tools + shell. |
| **M2** | `plexus-server` — agent loop, channels (Discord + Telegram), cron + heartbeat, REST API, SSE, workspace_fs, tools_registry, auth. DB schema canonical. |
| **M3** | `plexus-frontend` — React UI: chat, workspace browser, settings, admin. Served by plexus-server in release. REST + SSE only. |

## Reading order for contributors

1. **[DECISIONS.md](./DECISIONS.md)** — 70+ ADRs capturing design decisions. Read first. Everything else is derivable.
2. **`specs/`** (as specs land) — per-subsystem design documents: entrance, agent loop, context, tools, MCP, workspace, channels.
3. **`reference/`** — material ported from the prior Plexus codebase for feature-parity reference.

## Key design principles (summarized — see DECISIONS.md for detail)

1. **Generic over specialty.** Workspace + generic file tools replace specialty endpoints (no save_memory, no update_soul).
2. **Workspace is the single source of truth for user files.** No parallel caches.
3. **DB is the single source of truth for conversation state.** Every state change persists immediately. No in-memory session actors.
4. **Autonomous flows = user messages.** Cron and heartbeat synthesize InboundMessage into dedicated sessions. No `EventKind` branches.
5. **Crash recovery is passive.** JIT repair on next inbound message; no startup scans.
6. **No speculative scaffolding.** Fields without consumers get deleted.
7. **Follow nanobot where it makes sense.** Copy, don't reinterpret.

## Status

Docs-first. Implementation hasn't started yet.

## License

TBD.
