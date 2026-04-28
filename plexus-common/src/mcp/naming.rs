//! Wrapped-name builders + parser per ADR-048 (typed-infix convention).
//!
//! Wrapped name formats:
//! - tool: `mcp_<server>_<tool_name>`
//! - resource: `mcp_<server>_resource_<resource_name>`
//! - prompt: `mcp_<server>_prompt_<prompt_name>`
//!
//! The typed infixes (`_resource_` / `_prompt_`) make cross-surface name
//! collisions impossible. Tools stay unprefixed.
//!
//! **Naming constraint:** server names MUST NOT contain underscores OR the
//! literal substrings `_resource_` / `_prompt_`. Validated at MCP install time
//! by ADR-049's collision check; this module's parser assumes the constraint.

const MCP_PREFIX: &str = "mcp_";
const RESOURCE_INFIX: &str = "_resource_";
const PROMPT_INFIX: &str = "_prompt_";

/// MCP capability surface a wrapped name represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpSurface {
    Tool,
    Resource,
    Prompt,
}

/// Components of a parsed wrapped name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedName {
    pub server: String,
    pub surface: McpSurface,
    pub raw_name: String,
}

/// Build the wrapped name for a tool.
pub fn wrap_tool_name(server: &str, tool: &str) -> String {
    format!("{MCP_PREFIX}{server}_{tool}")
}

/// Build the wrapped name for a resource.
pub fn wrap_resource_name(server: &str, resource: &str) -> String {
    format!("{MCP_PREFIX}{server}{RESOURCE_INFIX}{resource}")
}

/// Build the wrapped name for a prompt.
pub fn wrap_prompt_name(server: &str, prompt: &str) -> String {
    format!("{MCP_PREFIX}{server}{PROMPT_INFIX}{prompt}")
}

/// Parse a wrapped name into its components.
///
/// Returns `None` if the input doesn't match any of the three formats
/// (e.g. missing `mcp_` prefix, empty server, empty raw name).
///
/// Resource / prompt parsing uses the typed infix as an unambiguous
/// boundary; tool parsing splits on the FIRST underscore after `mcp_`,
/// which assumes server names contain no underscores (ADR-048 constraint).
pub fn parse_wrapped_name(wrapped: &str) -> Option<WrappedName> {
    let after_prefix = wrapped.strip_prefix(MCP_PREFIX)?;
    if after_prefix.is_empty() {
        return None;
    }

    // Once a typed infix is present, the surface is decided — never fall back
    // to tool parsing, even if server/raw_name validation fails.
    if after_prefix.contains(RESOURCE_INFIX) {
        return parse_with_infix(after_prefix, RESOURCE_INFIX, McpSurface::Resource);
    }
    if after_prefix.contains(PROMPT_INFIX) {
        return parse_with_infix(after_prefix, PROMPT_INFIX, McpSurface::Prompt);
    }

    let (server, raw) = after_prefix.split_once('_')?;
    if server.is_empty() || raw.is_empty() {
        return None;
    }
    Some(WrappedName {
        server: server.to_string(),
        surface: McpSurface::Tool,
        raw_name: raw.to_string(),
    })
}

/// Try to split `after_prefix` on a typed infix; both halves must be non-empty.
fn parse_with_infix(after_prefix: &str, infix: &str, surface: McpSurface) -> Option<WrappedName> {
    let idx = after_prefix.find(infix)?;
    let server = &after_prefix[..idx];
    let raw = &after_prefix[idx + infix.len()..];
    if server.is_empty() || raw.is_empty() {
        return None;
    }
    Some(WrappedName {
        server: server.to_string(),
        surface,
        raw_name: raw.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_tool_name_basic() {
        assert_eq!(wrap_tool_name("google", "search"), "mcp_google_search");
    }

    #[test]
    fn wrap_resource_name_basic() {
        assert_eq!(
            wrap_resource_name("notion", "page"),
            "mcp_notion_resource_page"
        );
    }

    #[test]
    fn wrap_prompt_name_basic() {
        assert_eq!(
            wrap_prompt_name("helper", "code_review"),
            "mcp_helper_prompt_code_review"
        );
    }

    #[test]
    fn parse_tool_round_trip() {
        let wrapped = wrap_tool_name("google", "search");
        let parsed = parse_wrapped_name(&wrapped).unwrap();
        assert_eq!(parsed.server, "google");
        assert_eq!(parsed.surface, McpSurface::Tool);
        assert_eq!(parsed.raw_name, "search");
    }

    #[test]
    fn parse_resource_round_trip() {
        let wrapped = wrap_resource_name("notion", "page");
        let parsed = parse_wrapped_name(&wrapped).unwrap();
        assert_eq!(parsed.server, "notion");
        assert_eq!(parsed.surface, McpSurface::Resource);
        assert_eq!(parsed.raw_name, "page");
    }

    #[test]
    fn parse_prompt_round_trip() {
        let wrapped = wrap_prompt_name("helper", "code_review");
        let parsed = parse_wrapped_name(&wrapped).unwrap();
        assert_eq!(parsed.server, "helper");
        assert_eq!(parsed.surface, McpSurface::Prompt);
        assert_eq!(parsed.raw_name, "code_review");
    }

    #[test]
    fn parse_resource_with_underscore_raw_name() {
        // raw_name with underscore — legal for resource because the infix
        // unambiguously bounds the server segment.
        let wrapped = wrap_resource_name("notion", "page_content");
        let parsed = parse_wrapped_name(&wrapped).unwrap();
        assert_eq!(parsed.server, "notion");
        assert_eq!(parsed.raw_name, "page_content");
    }

    #[test]
    fn parse_no_mcp_prefix_returns_none() {
        assert!(parse_wrapped_name("google_search").is_none());
        assert!(parse_wrapped_name("read_file").is_none());
    }

    #[test]
    fn parse_just_prefix_returns_none() {
        assert!(parse_wrapped_name("mcp_").is_none());
        assert!(parse_wrapped_name("mcp_google").is_none());
    }

    #[test]
    fn parse_empty_string_returns_none() {
        assert!(parse_wrapped_name("").is_none());
    }

    #[test]
    fn parse_resource_with_empty_raw_name_returns_none() {
        // "mcp_notion_resource_" has empty raw name
        assert!(parse_wrapped_name("mcp_notion_resource_").is_none());
    }
}
