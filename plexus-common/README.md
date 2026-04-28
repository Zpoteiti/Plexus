# plexus-common

Shared types, errors, protocol, and tool infrastructure for the Plexus rebuild.

`plexus-common` is the foundation crate of the Plexus workspace. It is consumed
by `plexus-server` (M1) and `plexus-client` (M2) and contains everything that
both sides need to agree on — wire formats, error codes, tool schemas, MCP
wrapping, secret handling.

This crate has **no async runtime of its own** and **no IO**. It declares the
shapes; the runtime crates implement against them.

## Modules

- **`consts`** — wire-level reserved string constants (`[untrusted message from <name>]:`,
  the `plexus_` field prefix, etc.).
- **`version`** — `PROTOCOL_VERSION` and a `crate_version()` helper.
- **`secrets`** — redacting newtypes (`DeviceToken`, `JwtSecret`, `LlmApiKey`)
  wrapping `secrecy::SecretString`. Their `Debug`/`Display` impls always
  return `"<redacted>"`.
- **`errors`** — six typed error enums (`Workspace`, `Tool`, `Auth`, `Protocol`,
  `Mcp`, `Network`) all implementing `Code → ErrorCode` for stable wire-level
  mapping.
- **`protocol`** — WebSocket frame structs (`WsFrame` enum + 12 variants),
  binary-transfer header layout, frame inner types (`DeviceConfig`,
  `McpServerConfig`, `McpSchemas`, etc.).
- **`tools`** — the `Tool` trait, file-tool jail (`resolve_in_workspace`),
  result wrap helper (`wrap_result`), output formatters
  (`with_line_numbers`, `truncate_head`), all 14 first-class tool schemas
  as `LazyLock<serde_json::Value>` (and matching `LazyLock<Validator>` for
  cached JSON Schema validation), plus `validate_args` / `validate_with`.
- **`mcp`** — typed-infix wrapped names (`mcp_<server>_<tool>` / `_resource_<n>` /
  `_prompt_<n>`), URI template parser, `enabled` glob filter, `McpSession`
  wrapping `rmcp` 1.5.0's client, and the `spawn_mcp` / `teardown_mcp`
  lifecycle helpers.

## Design references

- Spec: [`docs/superpowers/specs/2026-04-28-plexus-m0-design.md`](../docs/superpowers/specs/2026-04-28-plexus-m0-design.md)
- Architecture decisions: [`docs/DECISIONS.md`](../docs/DECISIONS.md)
- WebSocket protocol: [`docs/PROTOCOL.md`](../docs/PROTOCOL.md)
- Tool catalog: [`docs/TOOLS.md`](../docs/TOOLS.md)

## Status

M0 deliverable. The public API surface is **frozen** here — additions during M1
or M2 should be exceptional and flagged as M0 underscoping. If a real new
need emerges, it goes through an ADR before landing in common.

## License

Apache 2.0.
