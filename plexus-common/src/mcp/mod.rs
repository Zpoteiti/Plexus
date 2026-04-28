//! Shared MCP infrastructure. See ADR-047, ADR-048, ADR-049, ADR-099, ADR-100, ADR-105.
//!
//! - [`naming`] — typed-infix wrapped names (`mcp_<server>_<tool>` etc.).
//! - [`filter`] — `enabled` glob filter (ADR-100).
//! - [`wrap`] — URI template parsing (ADR-099) and substitution.
//! - [`session`] — `McpSession` wrapper hiding `rmcp::RunningService`.
//! - [`lifecycle`] — `spawn_mcp` / `teardown_mcp` helpers (ADR-105).

pub mod filter;
pub mod lifecycle;
pub mod naming;
pub mod session;
pub mod wrap;
