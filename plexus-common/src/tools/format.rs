//! Output formatting helpers for file tools.
//!
//! - [`prepend_line_numbers`] — render `LINE_NUM|content` for read_file output.
//! - [`truncate_head`] — head-only character clipping with marker.
//!
//! Both are pure UTF-8 safe.

const TRUNCATION_MARKER: &str = "\n... (truncated)";

/// Render `text` with each line prefixed by its 1-indexed line number,
/// using `LINE_NUM|` as the separator (matches nanobot's read_file output).
///
/// Trailing newlines are preserved. Blank lines are numbered.
pub fn prepend_line_numbers(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let has_trailing_newline = text.ends_with('\n');
    let body = if has_trailing_newline {
        &text[..text.len() - 1]
    } else {
        text
    };
    let mut out = String::with_capacity(text.len() + 16);
    for (i, line) in body.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("{n}|{line}", n = i + 1));
    }
    if has_trailing_newline {
        out.push('\n');
    }
    out
}

/// Truncate `text` to at most `max_chars` Unicode characters, appending
/// the truncation marker if cut. UTF-8-safe: never splits a codepoint.
///
/// `max_chars` counts Unicode scalar values (Rust `char`s), not bytes.
pub fn truncate_head(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str(TRUNCATION_MARKER);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepend_line_numbers_empty() {
        assert_eq!(prepend_line_numbers(""), "");
    }

    #[test]
    fn prepend_line_numbers_single_line_no_trailing_newline() {
        assert_eq!(prepend_line_numbers("hello"), "1|hello");
    }

    #[test]
    fn prepend_line_numbers_single_line_with_trailing_newline() {
        assert_eq!(prepend_line_numbers("hello\n"), "1|hello\n");
    }

    #[test]
    fn prepend_line_numbers_multiple_lines() {
        let input = "alpha\nbeta\ngamma";
        let expected = "1|alpha\n2|beta\n3|gamma";
        assert_eq!(prepend_line_numbers(input), expected);
    }

    #[test]
    fn prepend_line_numbers_blank_lines_are_numbered() {
        let input = "a\n\nb";
        let expected = "1|a\n2|\n3|b";
        assert_eq!(prepend_line_numbers(input), expected);
    }

    #[test]
    fn truncate_head_short_input_unchanged() {
        let input = "short text";
        assert_eq!(truncate_head(input, 100), "short text");
    }

    #[test]
    fn truncate_head_exact_length_unchanged() {
        let input = "exact-len";
        assert_eq!(truncate_head(input, 9), "exact-len");
    }

    #[test]
    fn truncate_head_long_input_clipped_with_marker() {
        let input = "a".repeat(200);
        let result = truncate_head(&input, 50);
        assert!(result.starts_with(&"a".repeat(50)));
        assert!(result.ends_with("\n... (truncated)"));
    }

    #[test]
    fn truncate_head_does_not_split_multibyte_codepoint() {
        // "héllo" — 'é' is 2 bytes (U+00E9 = 0xC3 0xA9).
        // If max=2 chars (in our byte-based truncation), we must not split mid-codepoint.
        let input = "héllo";
        let result = truncate_head(input, 2);
        // Either "h" (truncate before 'é') or "hé" (truncate after 'é'),
        // never mid-codepoint. Both end with the marker.
        assert!(
            result == "h\n... (truncated)" || result == "hé\n... (truncated)",
            "unexpected truncation: {:?}",
            result
        );
    }

    #[test]
    fn truncate_head_zero_max_produces_only_marker() {
        let input = "anything";
        let result = truncate_head(input, 0);
        assert_eq!(result, "\n... (truncated)");
    }

    #[test]
    fn truncate_head_empty_input_unchanged() {
        assert_eq!(truncate_head("", 100), "");
    }
}
