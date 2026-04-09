# Gateway Deployment

## Build

```bash
cd PLEXUS
cargo build --release --package plexus-gateway
# Binary at: target/release/plexus-gateway
```

Static linking (fully portable binary):

```bash
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --package plexus-gateway --target x86_64-unknown-linux-gnu
```

## Environment Variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `PLEXUS_GATEWAY_TOKEN` | Yes | -- | Shared secret for plexus-server auth (constant-time compared) |
| `JWT_SECRET` | Yes | -- | HMAC secret for browser JWT validation (must match server) |
| `GATEWAY_PORT` | Yes | -- | Listen port (e.g. `9090`) |
| `PLEXUS_SERVER_API_URL` | Yes | -- | Upstream plexus-server base URL for REST proxy (e.g. `http://server:8080`) |
| `PLEXUS_FRONTEND_DIR` | No | `../plexus-frontend/dist` | Path to built frontend static files |

A `.env` file in the working directory is loaded automatically via `dotenvy`.

## Deployment Topology

### Same machine (simplest)

```
Browser --[wss]--> nginx:443 ---> plexus-gateway:9090 --[ws]--> plexus-server:8080
                                       |
                                       +--> /api/* proxied to plexus-server:8080
```

Gateway and server on the same box. Gateway serves the frontend static files via `PLEXUS_FRONTEND_DIR`. Nginx handles TLS.

### Edge deployment

```
Browser --[wss]--> edge-gateway:443 --[ws over WAN]--> plexus-server:8080
```

Gateway at the edge (close to users), server in a datacenter. Higher latency on the server link, but browser connections are snappy. The gateway buffers nothing -- messages are forwarded immediately.

## Nginx Reverse Proxy

```nginx
upstream gateway {
    server 127.0.0.1:9090;
}

server {
    listen 443 ssl http2;
    server_name plexus.example.com;

    ssl_certificate     /etc/letsencrypt/live/plexus.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/plexus.example.com/privkey.pem;

    # WebSocket endpoints
    location /ws/ {
        proxy_pass http://gateway;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_read_timeout 86400s;  # keep WS alive for 24h
        proxy_send_timeout 86400s;
    }

    # API + frontend (everything else)
    location / {
        proxy_pass http://gateway;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # For file uploads (proxy max body = 25MB, matching gateway limit)
        client_max_body_size 25m;
    }
}
```

## TLS with Caddy (zero-config alternative)

```
plexus.example.com {
    reverse_proxy localhost:9090
}
```

Caddy auto-provisions Let's Encrypt certs and handles WebSocket upgrade headers automatically.

## CORS

The gateway applies a permissive CORS policy via `tower-http`:

```rust
CorsLayer::new()
    .allow_origin(Any)
    .allow_methods(Any)
    .allow_headers(Any)
```

This is fine when the gateway is behind a reverse proxy that handles origin restrictions. For direct exposure, consider restricting `allow_origin` to your domain.

## Systemd Service

```ini
[Unit]
Description=plexus-gateway
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=plexus
Group=plexus
WorkingDirectory=/opt/plexus
ExecStart=/opt/plexus/plexus-gateway
EnvironmentFile=/opt/plexus/.env
Restart=always
RestartSec=3

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadOnlyPaths=/opt/plexus
PrivateTmp=true

# Allow enough file descriptors for many browser connections
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

```bash
sudo cp plexus-gateway.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now plexus-gateway
journalctl -u plexus-gateway -f  # tail logs
```

## Frontend Serving

The gateway serves static files from `PLEXUS_FRONTEND_DIR` as a fallback route, with SPA support (unknown paths serve `index.html`). Build the frontend first:

```bash
cd plexus-frontend && npm run build
# Output in plexus-frontend/dist/
```

Set `PLEXUS_FRONTEND_DIR=./plexus-frontend/dist` (or absolute path) in the gateway's `.env`.
