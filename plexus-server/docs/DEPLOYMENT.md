# plexus-server Deployment Guide

## Build

```bash
# Release binary (single static binary, ~30MB)
cargo build --release --package plexus-server

# Binary location
ls -la target/release/plexus-server
```

Requires Rust 1.85+ (edition 2024).

## Required Environment Variables

These are checked at startup. Missing required vars cause a panic with a descriptive message.

| Variable | Required | Default | Description |
|---|---|---|---|
| `DATABASE_URL` | **yes** | -- | PostgreSQL connection string. Example: `postgres://plexus:secret@localhost/plexus` |
| `ADMIN_TOKEN` | **yes** | -- | Token for creating admin users during registration |
| `JWT_SECRET` | **yes** | -- | HMAC-SHA256 signing key for JWTs. Use at least 32 random chars |
| `SERVER_PORT` | **yes** | -- | HTTP listen port (e.g. `8080`) |
| `PLEXUS_GATEWAY_WS_URL` | **yes** | -- | Gateway WebSocket URL (e.g. `ws://gateway:9090/ws/plexus`) |
| `PLEXUS_GATEWAY_TOKEN` | **yes** | -- | Token for server-to-gateway auth |
| `PLEXUS_WORKSPACE_ROOT` | **yes** | -- | Root directory for all user workspaces (`{root}/{user_id}/`) |

A `.env` file in the working directory is loaded automatically via `dotenvy`.

## PostgreSQL Setup

```bash
# Create database and user
sudo -u postgres psql <<'SQL'
CREATE USER plexus WITH PASSWORD 'your-secure-password';
CREATE DATABASE plexus OWNER plexus;
GRANT ALL PRIVILEGES ON DATABASE plexus TO plexus;
SQL
```

Tables are created automatically on startup via `db::init_db`. No manual migrations needed.

Connection pool: **200 max connections** (hardcoded in `main.rs`). Make sure PostgreSQL's `max_connections` is at least this + headroom for other clients.

```bash
# Check PostgreSQL max_connections
sudo -u postgres psql -c "SHOW max_connections;"
# Increase if needed (in postgresql.conf):
# max_connections = 300
```

## Systemd Service

```ini
[Unit]
Description=PLEXUS Server
After=network.target postgresql.service
Requires=postgresql.service

[Service]
Type=simple
User=plexus
Group=plexus
WorkingDirectory=/opt/plexus
ExecStart=/opt/plexus/plexus-server
Restart=always
RestartSec=5

# Environment (or use EnvironmentFile)
EnvironmentFile=/opt/plexus/.env

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/plexus/workspace
PrivateTmp=false

# Resource limits
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

```bash
# Install
sudo cp plexus-server.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now plexus-server
sudo journalctl -u plexus-server -f
```

## Docker

```dockerfile
# Build stage
FROM rust:1.85-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --package plexus-server

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/plexus-server /usr/local/bin/plexus-server
EXPOSE 8080
CMD ["plexus-server"]
```

```yaml
# docker-compose.yml
services:
  postgres:
    image: postgres:16
    environment:
      POSTGRES_USER: plexus
      POSTGRES_PASSWORD: secret
      POSTGRES_DB: plexus
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U plexus"]
      interval: 5s
      timeout: 5s
      retries: 5

  plexus-server:
    build: .
    ports:
      - "8080:8080"
    environment:
      DATABASE_URL: postgres://plexus:secret@postgres/plexus
      ADMIN_TOKEN: change-me-in-production
      JWT_SECRET: generate-a-random-32-char-string
      PLEXUS_GATEWAY_TOKEN: also-change-this
    depends_on:
      postgres:
        condition: service_healthy

volumes:
  pgdata:
```

## Health Checks

The server does not have a dedicated `/health` endpoint. Use:

```bash
# TCP check (server is listening)
curl -sf http://localhost:8080/api/auth/login -X POST -H 'Content-Type: application/json' -d '{}' || true
# Expected: 401 Unauthorized (server is alive)

# Or just check if the port is open
nc -z localhost 8080
```

The fallback handler returns `404 Not Found` for any unmatched route, which also confirms the server is running.

## Production Recommendations

### PostgreSQL Connection Pool

The pool is set to 200 max connections. For heavy workloads with many concurrent agent loops:

- Each agent loop holds a connection during DB operations (message saves, checkpoint writes)
- Cron scheduler polls every 10 seconds
- Config changes pushed to clients immediately (no DB queries on heartbeat)

If you see connection pool exhaustion, increase PostgreSQL's `max_connections` and restart.

### Heartbeat Timeout

Default 60s. The client sends heartbeats every 15s. The reaper checks every 30s.

- For flaky networks: increase to 120s
- For quick failover: keep at 60s (detect dead devices within ~90s worst case)

### Rate Limiting

Set via `PUT /api/admin/rate-limit`. The value is cached for 60s in memory. 0 = unlimited.

Reasonable defaults:
- Personal use: 0 (unlimited)
- Shared instance: 10-30 per minute per user

### File Storage (Unified Workspace)

All user files live under `{PLEXUS_WORKSPACE_ROOT}/{user_id}/`. Per-message attachments are stored in `.attachments/` within each user's workspace with a 30-day TTL. The workspace service (`WorkspaceFs`) enforces per-user quota (default 5 GB) and path isolation.

For production deployments, mount a persistent volume at `PLEXUS_WORKSPACE_ROOT`:

```bash
# In systemd — grant write access to the workspace directory
ReadWritePaths=/var/lib/plexus/workspace
```

### Graceful Shutdown

The server handles `SIGINT` (Ctrl+C) and `SIGTERM`:

1. Stops accepting new HTTP connections
2. Signals the message bus to shut down
3. Stops all channels (Discord bots, gateway connections) with a 10s timeout
4. Closes the database pool
5. Exits

### Logging

Uses `tracing_subscriber::fmt`. Control verbosity with `RUST_LOG`:

```bash
RUST_LOG=info           # Default: info level
RUST_LOG=debug          # Verbose: includes tool calls, LLM requests
RUST_LOG=plexus_server=debug,sqlx=warn  # Debug server, quiet sqlx
```

### Security Checklist

- [ ] Change `PLEXUS_GATEWAY_TOKEN` from `dev-token`
- [ ] Use a strong `JWT_SECRET` (32+ random chars)
- [ ] Use a strong `ADMIN_TOKEN`
- [ ] Run behind a reverse proxy (nginx/caddy) with TLS
- [ ] Restrict PostgreSQL access to the server host only
- [ ] Set appropriate rate limits for multi-user deployments
- [ ] Review device filesystem policies (default: sandbox)
