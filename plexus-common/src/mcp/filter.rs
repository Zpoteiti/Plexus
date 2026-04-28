//! `EnabledFilter` — `enabled` glob filter per ADR-100.
//!
//! Each MCP server config carries an optional `enabled: [<glob>...]` field.
//! When present, only entries whose post-wrap name matches at least one
//! glob register. When absent, every advertised capability registers
//! (default-allow). Empty list = nothing matches = nothing registers.
//!
//! Globs are compiled via the `globset` crate — same matcher Cargo uses.

use globset::{Glob, GlobSet, GlobSetBuilder};

/// Compiled filter for the `enabled` field of a McpServerConfig.
///
/// `None` inner = default-allow (no filter set in config).
/// `Some(set)` = only post-wrap names matching the set are accepted.
pub struct EnabledFilter(Option<GlobSet>);

impl EnabledFilter {
    /// Compile a filter from optional glob patterns.
    ///
    /// `None` produces a default-allow filter that accepts every name.
    /// `Some(empty)` produces a deny-all filter (no patterns to match).
    /// `Some(non_empty)` compiles each pattern via `globset::Glob::new`;
    /// any invalid pattern returns `Err(message)`.
    pub fn from_patterns(patterns: Option<&[String]>) -> Result<Self, String> {
        let Some(pats) = patterns else {
            return Ok(Self(None));
        };
        let mut builder = GlobSetBuilder::new();
        for (idx, pat) in pats.iter().enumerate() {
            // globset accepts empty strings as patterns (matches nothing), but
            // an empty pattern is almost certainly a config mistake, so reject
            // it explicitly.
            if pat.is_empty() {
                return Err(format!("empty pattern at index {idx}"));
            }
            let glob = Glob::new(pat).map_err(|e| format!("invalid glob '{pat}': {e}"))?;
            builder.add(glob);
        }
        let set = builder
            .build()
            .map_err(|e| format!("glob set build: {e}"))?;
        Ok(Self(Some(set)))
    }

    /// Test whether `wrapped_name` is accepted by this filter.
    pub fn accepts(&self, wrapped_name: &str) -> bool {
        match &self.0 {
            None => true,
            Some(set) => set.is_match(wrapped_name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(patterns: Option<&[&str]>) -> EnabledFilter {
        let owned: Option<Vec<String>> =
            patterns.map(|ps| ps.iter().map(|s| s.to_string()).collect());
        EnabledFilter::from_patterns(owned.as_deref()).expect("compile ok")
    }

    #[test]
    fn none_patterns_default_allows_everything() {
        let f = filter(None);
        assert!(f.accepts("mcp_google_search"));
        assert!(f.accepts("mcp_notion_resource_page"));
        assert!(f.accepts("anything_at_all"));
    }

    #[test]
    fn empty_patterns_deny_everything() {
        let f = filter(Some(&[]));
        assert!(!f.accepts("mcp_google_search"));
        assert!(!f.accepts("mcp_notion_resource_page"));
    }

    #[test]
    fn server_wildcard_matches_all_entries_of_server() {
        let f = filter(Some(&["mcp_notion_*"]));
        assert!(f.accepts("mcp_notion_search"));
        assert!(f.accepts("mcp_notion_resource_page"));
        assert!(f.accepts("mcp_notion_prompt_review"));
        assert!(!f.accepts("mcp_google_search"));
    }

    #[test]
    fn surface_wildcard_matches_resources_only() {
        let f = filter(Some(&["mcp_*_resource_*"]));
        assert!(f.accepts("mcp_notion_resource_page"));
        assert!(f.accepts("mcp_google_resource_index"));
        assert!(!f.accepts("mcp_notion_search"));
        assert!(!f.accepts("mcp_helper_prompt_review"));
    }

    #[test]
    fn multiple_patterns_union() {
        let f = filter(Some(&["mcp_notion_search", "mcp_notion_resource_*"]));
        assert!(f.accepts("mcp_notion_search"));
        assert!(f.accepts("mcp_notion_resource_page"));
        assert!(!f.accepts("mcp_notion_other_tool"));
        assert!(!f.accepts("mcp_google_search"));
    }

    #[test]
    fn literal_pattern_exact_match() {
        let f = filter(Some(&["mcp_google_search"]));
        assert!(f.accepts("mcp_google_search"));
        assert!(!f.accepts("mcp_google_search_v2"));
        assert!(!f.accepts("mcp_google_searchx"));
    }

    #[test]
    fn invalid_glob_returns_error() {
        // Unbalanced bracket — globset rejects.
        let result = EnabledFilter::from_patterns(Some(&["mcp_[invalid".to_string()]));
        assert!(result.is_err(), "expected glob compile error");
    }

    #[test]
    fn empty_pattern_string_rejected() {
        // globset rejects empty strings as patterns.
        let result = EnabledFilter::from_patterns(Some(&[String::new()]));
        assert!(result.is_err());
    }
}
