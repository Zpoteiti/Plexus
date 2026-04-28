//! Output formatting helpers for file tools.
//!
//! - [`with_line_numbers`] — render `LINE_NUM|content` for read_file output.
//! - [`truncate_head`] — head-only character clipping with marker.
//!
//! Both are pure UTF-8 safe.

use std::fmt::Write;

const TRUNCATION_MARKER: &str = "\n... (truncated)";

/// Render `text` with each line prefixed by its 1-indexed line number,
/// using `LINE_NUM|` as the separator (matches nanobot's read_file output).
///
/// Trailing newlines are preserved. Blank lines are numbered. Handles both
/// `\n` and `\r\n` line endings via `str::lines()`.
pub fn with_line_numbers(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    // Each line gains "<number>|" — average 4–7 bytes for files up to ~100K
    // lines. Sized accurately up front to avoid repeated reallocs on large
    // file reads (read_file's default cap is 128K characters).
    let line_count = bytecount_newlines(text) + 1;
    let mut out = String::with_capacity(text.len() + line_count * 8);
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let _ = write!(out, "{}|{line}", i + 1);
    }
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn bytecount_newlines(text: &str) -> usize {
    text.bytes().filter(|&b| b == b'\n').count()
}

/// Truncate `text` to at most `max_chars` Unicode characters, appending
/// the truncation marker if cut. UTF-8-safe: never splits a codepoint.
///
/// `max_chars` counts Unicode scalar values (Rust `char`s), not bytes.
/// Single-pass: walks the string at most once via `char_indices().nth(max_chars)`.
pub fn truncate_head(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        None => text.to_string(),
        Some((byte_idx, _)) => {
            let mut out = String::with_capacity(byte_idx + TRUNCATION_MARKER.len());
            out.push_str(&text[..byte_idx]);
            out.push_str(TRUNCATION_MARKER);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_line_numbers_empty() {
        assert_eq!(with_line_numbers(""), "");
    }

    #[test]
    fn with_line_numbers_single_line_no_trailing_newline() {
        assert_eq!(with_line_numbers("hello"), "1|hello");
    }

    #[test]
    fn with_line_numbers_single_line_with_trailing_newline() {
        assert_eq!(with_line_numbers("hello\n"), "1|hello\n");
    }

    #[test]
    fn with_line_numbers_multiple_lines() {
        let input = "alpha\nbeta\ngamma";
        let expected = "1|alpha\n2|beta\n3|gamma";
        assert_eq!(with_line_numbers(input), expected);
    }

    #[test]
    fn with_line_numbers_blank_lines_are_numbered() {
        let input = "a\n\nb";
        let expected = "1|a\n2|\n3|b";
        assert_eq!(with_line_numbers(input), expected);
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
