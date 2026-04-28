//! Tool result wrapping per ADR-095.
//!
//! Every tool's result content is prefixed with `[untrusted tool result]: `
//! at construction time, before reaching the LLM. Uniform across all tools.

use crate::consts::UNTRUSTED_TOOL_RESULT_PREFIX;

/// Wrap a raw tool result with the untrusted-tool-result prefix.
///
/// The prefix is the structural signal to the LLM that the content
/// should not be followed as instructions (ADR-095). Every tool's
/// dispatcher should call this before emitting the `tool_result` block.
pub fn wrap_result(raw: &str) -> String {
    let mut out = String::with_capacity(UNTRUSTED_TOOL_RESULT_PREFIX.len() + raw.len());
    out.push_str(UNTRUSTED_TOOL_RESULT_PREFIX);
    out.push_str(raw);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::UNTRUSTED_TOOL_RESULT_PREFIX;

    #[test]
    fn wrap_empty_string() {
        assert_eq!(wrap_result(""), UNTRUSTED_TOOL_RESULT_PREFIX);
    }

    #[test]
    fn wrap_plain_text() {
        let result = wrap_result("hello");
        assert!(result.starts_with(UNTRUSTED_TOOL_RESULT_PREFIX));
        assert!(result.ends_with("hello"));
        assert_eq!(result.len(), UNTRUSTED_TOOL_RESULT_PREFIX.len() + 5);
    }

    #[test]
    fn wrap_already_wrapped_double_wraps() {
        // Double-wrapping is intentional behavior per ADR-095:
        // every call to wrap_result prepends the prefix unconditionally.
        let once = wrap_result("hello");
        let twice = wrap_result(&once);
        let triple_check = format!("{UNTRUSTED_TOOL_RESULT_PREFIX}{once}");
        assert_eq!(twice, triple_check);
    }

    #[test]
    fn wrap_multiline_text() {
        let raw = "line 1\nline 2\nline 3";
        let wrapped = wrap_result(raw);
        assert_eq!(
            wrapped,
            format!("{UNTRUSTED_TOOL_RESULT_PREFIX}line 1\nline 2\nline 3")
        );
    }

    #[test]
    fn wrap_preserves_unicode() {
        let raw = "héllo 世界 🦀";
        let wrapped = wrap_result(raw);
        assert!(wrapped.contains("héllo 世界 🦀"));
    }
}
