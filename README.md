# Plexus

**Run AI agents on one server. Execute tools on any machine.**

Plexus is a distributed AI agent platform built in Rust. The core idea: separate *thinking* (LLM orchestration) from *doing* (tool execution). Your agent brain lives on a central server. Your tools run on remote machines — dev laptops, production servers, cloud VMs, wherever you need them.

```
You ── Browser ── Gateway ── Server ── Client (your laptop)
                                    ├── Client (prod server)
                                    ├── Client (cloud VM)
                                    └── Client (...)
```

Heavily inspired by [nanobot](https://github.com/nanobot-ai/nanobot) — a brilliant Python-based personal AI assistant framework. Plexus takes the patterns we learned from studying nanobot (multi-channel support, tool orchestration, memory, skills, cron, security) and re-architects them for distributed, multi-user, multi-machine deployment in Rust.

## Why Plexus?

Most AI agent frameworks assume everything runs on one machine. That breaks when:

- You want one agent managing tools across **multiple machines** (your laptop + a server + a cloud instance)
- You need **hundreds of users** sharing a single platform with proper isolation
- You want your agent accessible from a **web browser, Discord, Telegram** — all at once
- You need **real security** — sandboxed execution, rate limiting, SSRF protection, env isolation

Plexus solves all of these by splitting the architecture: the server handles the agent loop, memory, sessions, and LLM calls. Lightweight clients on remote machines just expose and execute tools.

## Features

- **Distributed tool execution** — connect any machine as an execution node
- **Multi-channel** — talk to your agent via web UI, Discord, or Telegram
- **ReAct agent loop** — up to 200 iterations, tool call deduplication, automatic rethink
- **Per-device security policies** — two-tier: Sandbox (workspace only) or Unrestricted, enforced server-side
- **Shell sandbox** — dangerous pattern blocking, env isolation, optional bubblewrap (Linux)
- **MCP support** — mount any MCP server on any client, tools auto-discovered
- **Skills system** — agent creates reusable skills via file tools; auto-discovered by dream from conversation patterns
- **Per-user workspace** — each user gets `{workspace_root}/{user_id}/` with SOUL.md, MEMORY.md, HEARTBEAT.md, skills/, uploads/; 5 GB quota with soft-lock at 100%
- **Memory & context compression** — persistent MEMORY.md per user, automatic conversation compression; dream autonomously consolidates memory every 2 hours
- **Dream** — idle-gated autonomous pass every 2 hours: consolidates recent conversations into long-term memory, extracts reusable skills; zero LLM cost when nothing happened
- **Heartbeat** — every 30 minutes, reads the user's HEARTBEAT.md task list and decides whether to wake the agent; evaluator-gated delivery (Discord → Telegram → silence; never interrupts active browser sessions)
- **Workspace browser** — full file-manager UI at `/settings/workspace`: tree view, markdown render/edit, image preview, drag-drop upload, quota bar, confirm-modal delete
- **Shared post-run evaluator** — small LLM call decides whether cron / heartbeat output warrants a user ping; default-silence on error (the "4 AM guard")
- **Account lifecycle** — users delete their own accounts (password-gated); admins delete any user (JWT-gated); full teardown: bots stop, browsers kicked, in-memory state evicted, workspace wiped, DB cascades
- **Admin user management** — Users tab in the admin panel with search filter and delete button
- **Cron jobs** — schedule recurring tasks, one-shot reminders, cross-channel delivery
- **Rate limiting** — per-user message throttle, admin-configurable
- **Built for scale** — DashMap-based routing, concurrent DB pool, designed for 1K users and 500 concurrent sessions

## Quick Start

### Prerequisites

- Rust 1.85+ (edition 2024)
- PostgreSQL 15+
- Node.js 18+ (for building the web frontend)

### 1. Build the frontend

```bash
cd plexus-frontend
npm install && npm run build
```

### 2. Start the server and gateway

```bash
# Set required env vars (or use a .env file)
export DATABASE_URL=postgres://user:pass@localhost/plexus
export ADMIN_TOKEN=your-admin-secret
export JWT_SECRET=your-jwt-secret-at-least-32-chars
export SERVER_PORT=8080
export GATEWAY_PORT=9090
export PLEXUS_GATEWAY_WS_URL=ws://localhost:9090/ws/plexus
export PLEXUS_GATEWAY_TOKEN=your-gateway-secret
export PLEXUS_SERVER_API_URL=http://localhost:8080
export PLEXUS_WORKSPACE_ROOT=/var/lib/plexus/workspace

# Start the server
cd plexus-server && cargo run &

# Start the gateway (serves the web UI automatically)
cd plexus-gateway && cargo run &
```

### 3. Set up via the web UI

Open `http://localhost:9090` in your browser. The gateway serves the frontend automatically. The first user to register becomes the admin and is guided through the setup wizard:

1. **Register** your admin account
2. **Configure LLM** — point to any OpenAI-compatible API (OpenAI, Anthropic via proxy, local models, etc.)
3. **Set rate limits** and other platform settings
4. **Create a device token** — gives you a token to connect your first client

### 4. Connect a client

```bash
cd plexus-client

export PLEXUS_SERVER_WS_URL=ws://localhost:8080/ws
export PLEXUS_AUTH_TOKEN=<paste your device token here>

cargo run
```

Your machine is now an execution node. The agent can run shell commands, read/write files, and use any MCP servers you configure — all on your machine. Connect as many machines as you want.

### 5. Start chatting

Go back to the web UI and send your first message. The agent is ready.

## Architecture

```
plexus-common/     Shared protocol types, error codes, constants
plexus-server/     Orchestration hub — agent loop, DB, auth, channels, tools
plexus-client/     Execution node — tool runtime, MCP, shell sandbox
plexus-gateway/    Browser WebSocket bridge — proxies between web UI and server
plexus-frontend/   React web UI — chat, settings, admin panel
```

| Component | Role | Scales to |
|-----------|------|-----------|
| **Server** | Agent brain, LLM calls, memory, sessions | 1K users, 500 concurrent sessions |
| **Client** | Tool execution, file I/O, shell commands | Thousands of connections per server |
| **Gateway** | Browser ↔ Server bridge, JWT auth | Thousands of browser sessions |
| **Frontend** | Chat UI, settings, admin | Served as static files |

## How It Works

**Interactive flow:**

1. You send a message (from browser, Discord, or Telegram — all chat goes through WebSocket)
2. The server's agent loop builds a system prompt from your SOUL.md, MEMORY.md, available tools, and skills
3. The LLM responds — either with text (done) or tool calls (continue)
4. Tool calls get routed to the right client by device name
5. The client executes the tool (with security guards) and returns the result
6. Loop back to step 3 until the LLM says it's done (or hits 200 iterations)

All messages, tool results, and workspace files are persisted. Context compression kicks in automatically when the conversation gets long.

**Autonomous subsystems:**

- **Dream** (every 2 hours, idle-gated) — reads recent conversations, consolidates insights into MEMORY.md, and extracts reusable skills into the skills/ directory. Does nothing — and costs nothing — if there's been no activity since the last run.
- **Heartbeat** (every 30 minutes) — reads HEARTBEAT.md, uses a lightweight LLM call to decide whether to wake the full agent, then runs if warranted. Output is delivered via Discord or Telegram; browser sessions are never interrupted. A shared post-run evaluator ("4 AM guard") silences pings unless the result is genuinely worth surfacing.
- **Cron** — user-scheduled tasks run on their own timers, delivered through the same evaluator-gated channel pipeline.

## Security

Plexus uses a server-authoritative security model — the server defines policy, clients enforce it.

- **Filesystem policy** — per-device, two-tier: Sandbox (workspace only) or Unrestricted; enforced server-authoritative, not by the client
- **Shell guards** — dangerous pattern blocking (`rm -rf`, `mkfs`, fork bombs, etc.), environment variable isolation (always on, even in Unrestricted mode)
- **Bubblewrap sandbox** — optional Linux process isolation, workspace rw + system ro + secrets hidden
- **SSRF protection** — blocks private IPs, link-local, CGNAT on both client shell and server web_fetch
- **Rate limiting** — per-user message throttle, admin-configurable
- **Untrusted content flagging** — web_fetch results marked as data, not instructions
- **Channel access control** — Discord allowlist per user

Security model inspired by [nanobot's defense-in-depth approach](https://github.com/nanobot-ai/nanobot/blob/main/SECURITY.md).

## Acknowledgements

Plexus wouldn't exist without [nanobot](https://github.com/nanobot-ai/nanobot). We're not affiliated with the nanobot project — we're just fans who learned a ton from studying their codebase. Many design decisions in Plexus — the tool execution model, cron scheduling, skill system, SSRF protection, shell guardrails, channel architecture — were directly informed by nanobot's battle-tested patterns. If you need a lightweight single-machine AI assistant, check out nanobot. If you need distributed multi-user deployment, that's where Plexus comes in.

## License

MIT
