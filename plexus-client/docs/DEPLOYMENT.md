# plexus-client Deployment Guide

## Build

```bash
# Release binary (from repo root)
cargo build --release --package plexus-client

# Binary lands at:
# target/release/plexus-client
```

Cross-compilation works with standard Rust targets. The binary is statically linkable with musl:
```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --package plexus-client --target x86_64-unknown-linux-musl
```

## Required Environment Variables

From `config.rs::load_config()` (also reads `.env` via dotenvy):

| Variable | Required | Format | Notes |
|---|---|---|---|
| `PLEXUS_SERVER_WS_URL` | yes | `ws://host:port/ws` or `wss://...` | Aliases: `PLEXUS_WS_URL` |
| `PLEXUS_AUTH_TOKEN` | yes | `plexus_dev_` + 32 random chars | Aliases: `PLEXUS_DEVICE_TOKEN`. Created via server admin API |

Optional:

| Variable | Default | Notes |
|---|---|---|
| `RUST_LOG` | (none) | Standard tracing filter, e.g. `info`, `plexus_client=debug` |

> **Note:** Workspace path, filesystem policy, MCP servers, and shell timeout are all configured per-device through the web UI (Settings > Devices). The server sends these to the client on connect.

Token format validation (enforced at startup, panics on mismatch):
- Must start with `plexus_dev_` (the `DEVICE_TOKEN_PREFIX` constant)
- Random segment must be exactly 32 characters (`DEVICE_TOKEN_RANDOM_LEN`)

## Install on a Remote Machine

```bash
# On build machine
cargo build --release --package plexus-client
scp target/release/plexus-client user@remote:/usr/local/bin/

# On remote machine
cat > ~/.plexus/.env << 'EOF'
PLEXUS_SERVER_WS_URL=wss://plexus.example.com/ws
PLEXUS_AUTH_TOKEN=plexus_dev_abcdef1234567890abcdef1234567890
EOF

# Create workspace
mkdir -p ~/.plexus/workspace

# Test
RUST_LOG=info plexus-client
```

## Auto-start with systemd (Linux)

```ini
# /etc/systemd/system/plexus-client.service
[Unit]
Description=PLEXUS Client
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=plexus
Group=plexus
WorkingDirectory=/home/plexus/.plexus
EnvironmentFile=/home/plexus/.plexus/.env
Environment=RUST_LOG=info
ExecStart=/usr/local/bin/plexus-client
Restart=always
RestartSec=5

# Hardening (optional, complements bwrap)
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/home/plexus/.plexus/workspace

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now plexus-client.service
sudo journalctl -u plexus-client -f
```

## Auto-start with launchd (macOS)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.plexus.client</string>

    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/plexus-client</string>
    </array>

    <key>EnvironmentVariables</key>
    <dict>
        <key>PLEXUS_SERVER_WS_URL</key>
        <string>wss://plexus.example.com/ws</string>
        <key>PLEXUS_AUTH_TOKEN</key>
        <string>plexus_dev_abcdef1234567890abcdef1234567890</string>
        <key>RUST_LOG</key>
        <string>info</string>
    </dict>

    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>

    <key>StandardOutPath</key>
    <string>/tmp/plexus-client.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/plexus-client.stderr.log</string>
</dict>
</plist>
```

```bash
cp com.plexus.client.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.plexus.client.plist
launchctl list | grep plexus
```

## MCP Server Configuration

MCP servers are configured **on the server**, not locally on the client. The server pushes the config to the client via `LoginSuccess` and `HeartbeatAck`.

Each `McpServerEntry` includes:

| Field | Type | Notes |
|---|---|---|
| `name` | string | Server identifier, used in tool name prefix |
| `transport_type` | string | `"stdio"` (default, only implemented), `"sse"`, `"streamableHttp"` |
| `command` | string | Binary to spawn (e.g., `npx`, `uvx`, path to binary) |
| `args` | string[] | Command arguments |
| `env` | map | Extra env vars for the MCP server process |
| `tool_timeout` | u64 | Per-tool call timeout in seconds (default: 30) |
| `enabled` | bool | Default true. Set false to disable without removing |

The client:
1. Receives the config on login.
2. Spawns each enabled stdio MCP server as a child process.
3. Runs the MCP `initialize` handshake + `tools/list`.
4. Registers prefixed tools (`mcp_{name}_{tool}`) with the server.
5. On each heartbeat, checks if the config hash changed. If so, reinitializes.

The MCP server process inherits the env from `McpServerConfig.env`, not from the host (the client doesn't apply `env_clear` to MCP servers -- they get their own explicit env).

## Multiple Clients on the Same Machine

Each client needs its own token. The server assigns a `device_name` per token at creation time.

```bash
# Client A
PLEXUS_SERVER_WS_URL=wss://plexus.example.com/ws \
PLEXUS_AUTH_TOKEN=plexus_dev_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
plexus-client

# Client B
PLEXUS_SERVER_WS_URL=wss://plexus.example.com/ws \
PLEXUS_AUTH_TOKEN=plexus_dev_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
plexus-client
```

Each client gets its own:
- WebSocket connection and session
- Workspace directory (configured per-device in the web UI)
- FsPolicy (configured per-device in the web UI)
- MCP server config (set per-device on the server)

Both share the same binary. Different systemd service files (e.g., `plexus-client-a.service`, `plexus-client-b.service`) with different `EnvironmentFile` paths.

## Bubblewrap Installation

bwrap is optional (Sandbox mode works without it -- just no namespace isolation). Linux only.

```bash
# Debian/Ubuntu
sudo apt install bubblewrap

# Fedora/RHEL
sudo dnf install bubblewrap

# Arch
sudo pacman -S bubblewrap

# Verify
bwrap --version
```

The client checks for bwrap availability once at startup. If installed after the client starts, restart the client to pick it up.

On macOS and Windows, bwrap is not available. The client relies on guardrails + env isolation only (no namespace sandbox).

## Connection Behavior

The client reconnects automatically with exponential backoff (1s, 2s, 4s, ... up to 30s). On each reconnect:
1. Full handshake (RequireLogin -> SubmitToken -> LoginSuccess).
2. Tool discovery and re-registration.
3. Heartbeat loop resumes.

No manual intervention needed after network interruptions.
