# Plexus

**Run AI agents on one server. Execute tools on any machine.**

Plexus is a distributed AI agent platform built in Rust. The core idea: separate *thinking* (LLM orchestration) from *doing* (tool execution). Your agent brain lives on a central server. Your tools run on remote machines — dev laptops, production servers, cloud VMs, wherever you need them.

```
You ── Browser ── Server ── Client (your laptop)
                       ├── Client (prod server)
                       ├── Client (cloud VM)
                       └── Client (...)
```

Heavily inspired by [nanobot](https://github.com/nanobot-ai/nanobot) — a brilliant Python-based personal AI assistant framework. Plexus takes the patterns we learned from studying nanobot (multi-channel support, tool orchestration, memory, skills, cron, security) and re-architects them for distributed, multi-user, multi-machine deployment in Rust.

## Why Plexus?

Most AI agent frameworks assume everything runs on one machine. That breaks when:

- You want one agent managing tools across **multiple machines** (your laptop + a server + a cloud instance)
- You need **hundreds of users** sharing a single platform with proper isolation
- You want **shared workspaces** — knowledge bases your whole team can read and edit
- You want your agent accessible from a **web browser, Discord, Telegram** — all at once
- You need **real security** — sandboxed execution, SSRF protection, env isolation

Plexus solves all of these by splitting the architecture: the server handles the agent loop, memory, sessions, and LLM calls. Lightweight clients on remote machines just expose and execute tools.

## Features

- **Distributed tool execution** — connect any machine as an execution node
- **Multi-channel** — talk to your agent via web UI, Discord, or Telegram
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

### 2. Start the server

```bash
# Set required env vars (or use a .env file)
export DATABASE_URL=postgres://user:pass@localhost/plexus
export JWT_SECRET=your-jwt-secret-at-least-32-chars
export ADMIN_TOKEN=your-admin-secret
export SERVER_PORT=8080
export PLEXUS_WORKSPACE_ROOT=/var/lib/plexus/workspaces

# Start the server (also serves the web UI in release builds)
cd plexus-server && cargo run
```

The server is a single binary serving everything: REST API, SSE streams for the browser, WebSocket for devices, and the embedded React frontend. No separate gateway process — put nginx/Caddy in front for TLS in production.

### 3. Set up via the web UI

Open `http://localhost:8080` in your browser. Anyone registering with the matching `ADMIN_TOKEN` becomes an admin; multiple admins are possible — just share the token with whoever should have admin rights. Regular users register without the token.

First-time admin flow:

1. **Register** your admin account, supplying the `ADMIN_TOKEN`
2. **Configure LLM** — point to any OpenAI-compatible API (OpenAI, Anthropic via proxy, local models, etc.)
3. **Set workspace defaults** — personal quota, shared workspace cap, and other platform settings
4. **Create your first device** — give it a name (e.g. "laptop") and copy the generated token to connect a client

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
plexus-common/     Shared protocol types, error codes, constants, MCP client
plexus-server/     Orchestration hub — agent loop, DB, auth, channels, tools, web UI
plexus-client/     Execution node — tool runtime, MCP, shell sandbox
plexus-frontend/   React web UI — chat, settings, admin panel (embedded in server binary in release builds)
```

| Component | Role | Scales to |
|-----------|------|-----------|
| **Server** | Agent brain, LLM calls, memory, sessions, web UI, browser SSE | 1K users, 500 concurrent sessions |
| **Client** | Tool execution, file I/O, shell commands | Thousands of connections per server |
| **Frontend** | Chat UI, settings, admin | Embedded in server binary; Vite dev server in development |

## How It Works

1. You send a message — browser uses REST + SSE, Discord/Telegram come in through their SDKs, devices ride a WebSocket back to the server
2. The server's agent loop builds a system prompt with your soul, memory, available tools, skills, and accessible workspaces
3. The LLM responds — either with text (done) or tool calls (continue)
4. Tool calls get routed to the right client by device name (or run server-side for file ops on the workspace)
5. The client executes the tool (with security guards) and returns the result
6. Loop back to step 3 until the LLM says it's done (or hits 200 iterations)

All messages, tool results, and memory are persisted in PostgreSQL. Context compression kicks in automatically when the conversation gets long.

## Security

Plexus uses a server-authoritative security model — the server defines policy, clients enforce it.

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