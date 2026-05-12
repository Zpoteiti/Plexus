# Plexus

**In-progress distributed AI agent platform in Rust.**

Plexus is being rebuilt as a distributed AI agent platform. The target idea is
to separate *thinking* (LLM orchestration) from *doing* (tool execution): the
agent brain will live on a central server, and tools will run on remote
machines such as dev laptops, production servers, or cloud VMs.

Target architecture sketch:

```
You ── Browser ── Server ── Client (your laptop)
                       ├── Client (prod server)
                       ├── Client (cloud VM)
                       └── Client (...)
```

Heavily inspired by [nanobot](https://github.com/nanobot-ai/nanobot) — a brilliant Python-based personal AI assistant framework. Plexus takes the patterns we learned from studying nanobot (multi-channel support, tool orchestration, memory, skills, cron, security) and re-architects them for distributed, multi-user, multi-machine deployment in Rust.

## Current M1b Status

The `rebuild-m1-M1b` branch is not a usable end-user agent yet. It currently
contains the server foundation and the OpenAI-compatible LLM provider
foundation:

- PostgreSQL-backed `plexus-server` startup and admin/auth foundations from
  M1a.
- Admin LLM config keys for `llm_endpoint`, `llm_api_key`, `llm_model`, and
  `llm_max_concurrent_requests`.
- Validate-before-commit provider identity checks through `GET
  {llm_endpoint}/models`.
- Write-only admin API behavior for `llm_api_key`, with configured keys shown
  as `"<redacted>"` in admin responses.
- Internal non-streaming Chat Completions call mechanics in
  `plexus-server/src/openai.rs` with `stream=false`.
- Hermetic Rust tests using an in-process fake provider, plus an optional
  sibling FastAPI mock for local/manual provider smoke testing.

Not implemented in M1b: browser chat, SSE chat delivery, persisted
conversation workflows, agent orchestration, tool execution, context
compaction, cron, heartbeat, Discord/Telegram adapters, the production
frontend, and the standalone client. The sections below describe the target
product unless they explicitly mention M1b.

## Why Plexus?

Most AI agent frameworks assume everything runs on one machine. That breaks when:

- You want one agent managing tools across **multiple machines** (your laptop + a server + a cloud instance)
- You need **hundreds of users** sharing a single platform with proper isolation
- You want **shared workspaces** — knowledge bases your whole team can read and edit
- You want your agent accessible from a **web browser, Discord, Telegram** — all at once
- You need **real security** — sandboxed execution, SSRF protection, env isolation

The target architecture solves these by splitting responsibilities: the server
will handle the agent loop, memory, sessions, and LLM calls, while lightweight
clients on remote machines expose and execute tools.

## Target Features

These are intended product capabilities, not all current M1b behavior.

- **Distributed tool execution** — connect machines as execution nodes
- **Multi-channel** — web UI, Discord, and Telegram ingress
- **ReAct agent loop** — up to 200 iterations with trap-in-loop detection
- **Per-device security policies** — sandbox or unrestricted filesystem access
- **Shell sandbox** — bubblewrap on Linux, env isolation always on
- **MCP support** — admin-configured server-side MCPs + per-device MCPs on each client
- **Skills system** — install via file transfer or web UI, always-on or on-demand, progressive disclosure
- **Personal + shared workspaces** — private workspace per user, plus opt-in shared knowledge bases for team collaboration
- **Memory & context compression** — persistent memory per user, automatic conversation compression
- **Full-fidelity history** — images stored inline as base64, conversations replay forever
- **Cron jobs** — schedule recurring tasks, one-shot reminders, cross-channel delivery
- **Built for scale** — DashMap-based routing, concurrent DB pool, designed for 1K users and 500 concurrent sessions

## M1b Development Quick Start

### Prerequisites

- Rust 1.85+ (edition 2024)
- PostgreSQL 15+

### Dev DB reset and test helper

For this dev box, `scripts/reset-postgres18-and-test.sh` removes every Docker
container, starts a clean PostgreSQL 18 container named `plexus` with database,
user, and password all set to `plexus`, runs `cargo test --workspace` against
it, then prints any remaining `plexus%` databases and public tables in the
persistent `plexus` database.

```bash
scripts/reset-postgres18-and-test.sh
```

### Mock LLM for M1b development

The deterministic OpenAI-compatible mock service lives beside this repository
at `../Plexus-mock-llm`. It is a local/manual dev target for M1b provider
validation and non-streaming external-call testing; Plexus Rust tests use an
in-process fake provider instead.

```bash
cd ../Plexus-mock-llm
conda activate Plexus
uvicorn app.main:app --host 127.0.0.1 --port 8089
```

Use these admin config values:

```json
{
  "llm_endpoint": "http://127.0.0.1:8089/v1",
  "llm_api_key": "plexus-mock-key",
  "llm_model": "plexus-fake-qa",
  "llm_max_concurrent_requests": 0
}
```

### 1. Start the server

```bash
# Set required env vars (or use a .env file)
export DATABASE_URL=postgres://user:pass@localhost/plexus
export JWT_SECRET=your-jwt-secret-at-least-32-chars
export ADMIN_TOKEN=your-admin-secret
export SERVER_PORT=8080
export PLEXUS_WORKSPACE_ROOT=/var/lib/plexus/workspaces

# Start the M1b server foundation
cd plexus-server && cargo run
```

The M1b server foundation exposes auth/admin REST behavior and LLM provider
configuration mechanics. Use an API client or automated tests to register an
admin and exercise `PATCH /api/admin/config` with the mock values above.

The web UI, browser chat, SSE delivery, device WebSocket flow, standalone
client, and agent execution loop are later milestones and are not available in
M1b.

## Target Architecture

```
plexus-common/     Shared protocol types, error codes, constants, MCP client
plexus-server/     Server hub - DB, auth, admin config, and target orchestration
plexus-client/     Planned execution node - tool runtime, MCP, shell sandbox
plexus-frontend/   Planned React web UI - chat, settings, admin panel
```

| Component | Role | Scales to |
|-----------|------|-----------|
| **Server** | Current M1b: DB/auth/admin config and LLM provider foundation. Target: agent brain, LLM calls, memory, sessions, browser SSE. | 1K users, 500 concurrent sessions |
| **Client** | Planned tool execution, file I/O, and shell commands. | Thousands of connections per server |
| **Frontend** | Planned chat UI, settings, and admin panel. | Embedded in server binary in target release builds |

## Target Runtime Flow

When later M1 slices are implemented:

1. A user sends a message through browser REST + SSE, Discord/Telegram, or another channel.
2. The server's agent loop builds a system prompt with user memory, available tools, skills, and accessible workspaces.
3. The LLM responds with either text or tool calls.
4. Tool calls route to the right client by device name or to server-side workspace file tools.
5. The client executes the tool with security guards and returns the result.
6. The loop continues until the LLM is done or hits its iteration cap.

The target design persists messages, tool results, and memory in PostgreSQL.
Context compression is planned for later orchestration work, not M1b.

## Security

Plexus is designed around a server-authoritative security model: the server
defines policy, and clients enforce it once the client/device slices exist.

- **Filesystem policy** — per-device: Sandbox (workspace only) or Unrestricted (typed-name confirmation required to flip)
- **Bubblewrap sandbox** — Linux process isolation, workspace rw + minimal system ro + secrets hidden
- **Environment isolation** — host env stripped to a small whitelist before exec, even in Unrestricted mode
- **SSRF protection** — server `web_fetch` has hardcoded RFC-1918 + link-local + CGNAT block; per-device whitelists govern client-side network calls
- **Server doesn't execute user content** — agents have no shell on the server, only file ops; user-uploaded scripts can never run on the server
- **Workspace isolation** — personal workspaces are strictly per-user; shared workspaces only see their explicit allow-list
- **Untrusted content flagging** — messages from non-partners arrive wrapped (`[untrusted message from <name>]:`) so the agent treats them as data, not instructions
- **Channel access control** — Discord allowlist per user

Security model inspired by [nanobot's defense-in-depth approach](https://github.com/nanobot-ai/nanobot/blob/main/SECURITY.md).

## Acknowledgements

Plexus wouldn't exist without [nanobot](https://github.com/nanobot-ai/nanobot). We're not affiliated with the nanobot project — we're just fans who learned a ton from studying their codebase. Many design decisions in Plexus — the tool execution model, cron scheduling, skill system, SSRF protection, shell guardrails, channel architecture — were directly informed by nanobot's battle-tested patterns. If you need a lightweight single-machine AI assistant, check out nanobot. If you need distributed multi-user deployment, that's where Plexus comes in.

## License

MIT
