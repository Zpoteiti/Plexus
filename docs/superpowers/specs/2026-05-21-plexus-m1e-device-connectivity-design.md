# Plexus M1e Device Connectivity Sub-Spec

**Status:** Draft
**Parent:** [Plexus M1 Living Design Spec](2026-05-12-plexus-m1-living-design.md)
**Branch:** `rebuild-m1-M1e`
**Base:** `rebuild-m1`
**Authors:** brainstormed in collaborative session 2026-05-21

---

## 1. Goal

M1e implements the server-side foundation for client devices. A user can
register a device, receive a device token, connect that device to
`/ws/device`, complete the `hello` / `hello_ack` handshake, and see the device
as reachable while the WebSocket remains healthy.

The success proof is intentionally connection-scoped:

- device rows can be created, listed with full details, updated, regenerated,
  and deleted through authenticated REST APIs;
- device tokens authenticate the WebSocket handshake without ever appearing in
  REST path segments;
- a valid device connection receives `hello_ack` with its persisted
  `DeviceConfig`;
- online state is derived from live WebSocket connections, not persisted DB
  flags;
- heartbeat detects dead connections and marks devices offline;
- one device token maps to at most one active connection;
- the connection registry stores a send handle so later milestones can route
  `WsFrame`s without reshaping the registry.

---

## 2. Non-Goals

M1e does not include:

- production `plexus-client`;
- client reconnect/backoff implementation;
- remote `tool_call` / `tool_result` dispatch;
- in-flight tool-call tracking;
- synthesized `device_unreachable` tool results;
- file transfer slot management or binary transfer plumbing;
- client-device file reads, writes, or message attachment reads;
- `register_mcp` processing, MCP collision handling, or MCP execution;
- frontend settings UI;
- persisted `online` or `last_seen_at` columns.

Those pieces are deferred deliberately:

- M1f: device-routed file/tool execution, offline execution errors, and transfer
  plumbing;
- M1h: admin shared-service MCP and device MCP registration/execution;
- M2: production `plexus-client`.

---

## 3. Contract Corrections

M1e keeps ADR-091's split between secret token and user-facing device name, but
narrows device-name validation from ADR-109's Unicode-friendly identifier rule.

Device names are not pure display labels. They appear in:

- REST paths such as `/api/devices/{name}`;
- future `plexus_device` tool arguments;
- agent-visible device enums;
- logs and routing diagnostics.

For M1e, device names are canonical machine-friendly slugs. This intentionally
matches the network-node style used by systems such as Tailscale machine names
and avoids localization, URL escaping, and Unicode lookalike issues for the
first device-routing implementation.

Workspace names and skill folder names are not changed by this sub-spec.

M1e also intentionally omits `GET /api/devices/{name}`. The list route
returns full device details for all of the user's devices, and the expected
per-user device count is small. Mutating routes still use `{name}` because they
need a single resource target.

---

## 4. Data Model

The existing `devices` table remains the persistence boundary:

```text
token              TEXT PRIMARY KEY
user_id            UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE
name               TEXT NOT NULL
workspace_path     TEXT NOT NULL
fs_policy          TEXT NOT NULL DEFAULT 'sandbox'
shell_timeout_max  INTEGER NOT NULL DEFAULT 300
ssrf_whitelist     JSONB NOT NULL DEFAULT '[]'
mcp_servers        JSONB NOT NULL DEFAULT '{}'
created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
UNIQUE (user_id, name)
```

`devices.token` is stored plaintext. It is the credential and the canonical
internal device identifier. It is returned only on create and explicit
regeneration.

`devices.name` is the REST/tool-routing name. It is unique only within a user.
Two users may both have a device named `laptop`.

M1e may strengthen schema checks for slug names if the canonical schema can do
so without adding a migration framework. Runtime validation remains required
because the server normalizes raw names before insert/update.

---

## 5. Device Names

Device names are normalized to a canonical slug before storage.

Rules:

- trim leading and trailing whitespace;
- lowercase ASCII letters;
- keep ASCII letters and digits;
- convert ASCII whitespace, `_`, and `-` to separators;
- remove ASCII apostrophes;
- reject all other characters;
- collapse repeated `-`;
- remove leading and trailing `-`;
- final stored value must match `^[a-z0-9]+(-[a-z0-9]+)*$`;
- maximum length is 64 characters after normalization;
- `server` is reserved and rejected case-insensitively;
- empty names are rejected;
- names containing non-ASCII letters are rejected in M1e.

Examples:

```text
MacBook Pro      -> macbook-pro
John's iPhone 6S -> johns-iphone-6s
lab_pc_01        -> lab-pc-01
lab--machine     -> lab-machine
办公室电脑        -> reject
Server           -> reject
```

`PATCH /api/devices/{name}/config` may rename a device by accepting a new raw
name and storing its canonical slug. After a rename, the old REST path is no
longer valid.

---

## 6. Device Config

M1e persists and returns the config fields already modeled by
`plexus_common::protocol::DeviceConfig`:

```text
workspace_path
fs_policy
shell_timeout_max
ssrf_whitelist
mcp_servers
```

Defaults on create:

```text
workspace_path: "~/plexus/workspace"
fs_policy: "sandbox"
shell_timeout_max: 300
ssrf_whitelist: []
mcp_servers: {}
```

`workspace_path` is optional on `POST /api/devices`. If omitted, the server
stores the literal string `~/plexus/workspace` for every OS.

Users may override the default with an explicit path, for example
`/tmp/plexus-testing-path`, `/srv/agent`, or `D:\projects`. The server stores
the provided string verbatim after validation; it does not expand `~`, does not
canonicalize, and does not convert path separators. The future production client
expands and validates the path on the target device.

M1e sends the latest config in `hello_ack`. `PATCH /api/devices/{name}/config`
updates persisted config and, when the device is online, sends a
`config_update` frame through the connection registry. If that send fails, the
server treats the registry entry as stale and removes it.

---

## 7. REST API

All REST routes require the existing browser/user JWT. Device token auth is only
for `/ws/device`.

M1e implements:

```text
POST   /api/devices
GET    /api/devices
PATCH  /api/devices/{name}/config
POST   /api/devices/{name}/regenerate-token
DELETE /api/devices/{name}
```

`POST /api/devices`:

- accepts raw `name` and optional config fields;
- stores the canonical slug;
- generates a new `plexus_dev_...` token;
- returns the created device plus the plaintext token exactly once.

`GET /api/devices`:

- returns every device for the authenticated user;
- returns full device details, including config fields and derived online status;
- does not paginate in M1e because per-user device count is expected to be small;
- returns `token_hint` such as `plexus_dev_...abcd` for user confirmation;
- never returns the plaintext token.

`PATCH /api/devices/{name}/config`:

- may update name, `workspace_path`, `fs_policy`, `shell_timeout_max`,
  `ssrf_whitelist`, and `mcp_servers`;
- validates and stores a canonical slug when name changes;
- enforces `(user_id, name)` uniqueness;
- never changes the token.

`POST /api/devices/{name}/regenerate-token`:

- keeps the existing device row and config;
- generates and stores a new token;
- returns the new plaintext token exactly once;
- closes any active connection for the old token with close code `4401`.

`DELETE /api/devices/{name}`:

- deletes the row;
- closes any active connection with close code `4401`;
- relies on `ON DELETE CASCADE` for user-owned rows.

M1e keeps the explicit regenerate endpoint. A revoke-then-create flow would
discard device config and produce unnecessary user work.

---

## 8. WebSocket Protocol

Endpoint:

```text
GET /ws/device
Authorization: Bearer <PLEXUS_DEVICE_TOKEN>
```

Fallback for clients that cannot set headers during upgrade:

```text
GET /ws/device?token=<PLEXUS_DEVICE_TOKEN>
```

The server uses `plexus_common::protocol::WsFrame` and
`plexus_common::version::PROTOCOL_VERSION`. M1e must not define duplicate frame
types in `plexus-server`.

Handshake:

1. Extract the device token from the bearer header or `token` query parameter.
2. Look up the device row by `devices.token`.
3. If lookup fails, close with `4401` and `{"code":"unauthorized"}`.
4. Wait for a text `hello` frame.
5. If `hello.version` differs from `PROTOCOL_VERSION`, close with `4409`.
6. Send `hello_ack` with the same `id`, the stored `device_name`, `user_id`,
   and current `DeviceConfig`.
7. Register the connection as online.

M1e handles these text frames:

- `hello` during handshake;
- `pong` as heartbeat response;
- `ping` by echoing `pong`;
- outgoing `config_update` after REST config changes;
- `error` by logging and keeping the connection open unless the socket closes.

Other valid `WsFrame` variants may be ignored with a protocol-level `error`
frame or logged as unsupported in M1e. Business handling for `tool_call`,
`tool_result`, incoming `register_mcp`, and transfer frames belongs to later
milestones. M1e sends `config_update` after REST config changes but does not
process client-sent `config_update` frames.

Binary WebSocket messages are not implemented in M1e. The read loop should
recognize them and return a clear unsupported/protocol error rather than
treating them as malformed UTF-8 text.

---

## 9. Connection Registry

M1e introduces an in-memory `DeviceConnectionRegistry`.

Shape:

```text
token -> ConnHandle {
  user_id,
  device_name,
  connected_at,
  last_seen,
  tx: mpsc::Sender<WsFrame>
}
```

The registry is the only online-state authority. No DB `online` column and no
DB `last_seen_at` column are added in M1e.

The send handle is required now even though M1e only sends handshake,
heartbeat, and config-update frames. M1f will use the same boundary to send `tool_call` frames
without reshaping the registry.

When the socket closes, the connection removes itself from the registry only if
the stored handle still belongs to that socket. This prevents an old connection
cleanup from deleting a newer replacement connection.

---

## 10. Duplicate Connections

Only one active connection is allowed per token.

If a new authenticated connection completes `hello` for a token that already has
an active connection:

- the new connection wins;
- the registry is updated to point at the new `ConnHandle`;
- the old connection is closed with code `1000`;
- the old connection cleanup must not mark the new connection offline.

This supports fast client restart without waiting for heartbeat timeout. It
also makes accidental token reuse explicit: two processes sharing one token will
replace each other rather than appearing as two devices.

---

## 11. Heartbeat

The server sends `ping` every 30 seconds after handshake.

The client replies with `pong` echoing the ping `id`.

M1e marks a device offline and closes the socket with `4408` after two missed
pongs, approximately 70 seconds. The exact implementation may use a monotonic
deadline rather than a counter, but the external behavior should match
`docs/PROTOCOL.md`.

Client-initiated `ping` is optional and not required by M1e. If a client
sends `ping`, M1e responds with `pong` because `docs/PROTOCOL.md` permits
either side to run a liveness probe.

---

## 12. Error Handling

Close codes:

```text
4401 unauthorized
4408 heartbeat timeout
4409 version unsupported
1000 old connection replaced by newer connection
```

`4401` is used for invalid/revoked tokens, regeneration of an active token, and
device deletion. It is not used for duplicate-connection replacement because the
token remains valid.

`4409` is used only when the client speaks an unsupported protocol version.

Malformed text frames receive a protocol `error` frame where possible; if the
connection cannot continue safely, the server closes the socket.

REST errors follow existing `ApiError` conventions and should avoid leaking
tokens in messages or logs.

---

## 13. Testing

M1e uses automated tests first. The test client is an in-process helper under
`plexus-server/tests/support/device_client.rs`, not a production client binary.

REST tests:

- create device returns canonical name, config defaults, and token once;
- list returns full device details for every device;
- list returns `token_hint` and does not return plaintext token;
- explicit `workspace_path` override is stored verbatim;
- slug normalization accepts `MacBook Pro` as `macbook-pro`;
- non-ASCII names reject;
- `server` rejects;
- duplicate name for same user rejects;
- same name for different users is allowed;
- patch can rename and update config without changing token;
- regenerate preserves config and returns a new token;
- delete removes the row.

WebSocket tests:

- missing/invalid token closes `4401`;
- valid `hello` returns `hello_ack` with current config;
- protocol mismatch closes `4409`;
- heartbeat `ping` / `pong` keeps the device online;
- missed pongs close `4408` and remove registry entry;
- duplicate connection replaces the old connection and keeps the new one online;
- regenerate closes an active old-token connection with `4401`;
- delete closes an active connection with `4401`.

Regression tests should cover the stale-cleanup case where an old replaced
connection closes after the new connection has already registered.

---

## 14. Implementation Notes

Keep the implementation thin:

- use `plexus-common` frame/config/token/version types;
- keep REST handlers small and push DB lookups/validation into local helpers;
- keep WebSocket state machine code separate from REST routes;
- do not introduce per-device actors or in-flight call tables in M1e;
- do not add a production client crate target or example binary unless a manual
  debugging need appears during implementation;
- do not persist online state.

The implementation should update canonical schema docs if runtime validation or
schema checks change.

---

## 15. Acceptance Criteria

M1e is complete when:

- all REST device lifecycle tests pass;
- all WebSocket handshake, heartbeat, duplicate-connection, and revoke/delete
  tests pass;
- `cargo test -p plexus-server` passes;
- docs are updated for the slug device-name correction and M1e behavior;
- the branch has no unrelated code or formatting churn.
