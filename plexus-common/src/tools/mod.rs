//! Shared tool infrastructure. See ADR-038, ADR-077, ADR-095.
//!
//! - [`result`] — wrap_result() for the [untrusted tool result]: prefix.
//! - [`path`] — resolve_in_workspace() for the file-tool jail.
//! - [`format`] — line-numbered output and head-only truncation helpers.
//! - [`schemas`] — hardcoded JSON schemas for the 14 first-class tools.
//! - [`validate`] — JSON Schema validation for tool_call args.
//!
//! The [`Tool`] trait itself lands in this module in Task 8.

pub mod format;
pub mod path;
pub mod result;
pub mod schemas;
pub mod validate;
