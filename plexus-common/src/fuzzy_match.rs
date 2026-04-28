//! Multi-level text matcher used by `edit_file` on both server and client.
//!
//! Ported from nanobot's `_find_match` / `_find_matches` chain
//! (`nanobot/agent/tools/filesystem.py`). Applies three progressively looser
//! strategies in order and reports the best-similarity window when none match.

/// Successful match against the file content.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchResult {
    /// Exact text as it appears in `content` (may differ from the caller's
    /// `old_text` via whitespace or quote normalization).
    pub matched_text: String,
    /// Number of matches found at the level where matching first succeeded.
    pub count: usize,
}

/// Failure diagnostic when no level produced a hit.
#[derive(Debug, Clone)]
pub struct MatchFailure {
    /// Best similarity ratio (0.0-1.0) against any N-line window in content.
    pub best_ratio: f64,
    /// Human-readable hints explaining why the near-match failed
    /// (e.g. "letter case differs", "whitespace differs").
    pub hints: Vec<String>,
}

/// Locate `old_text` within `content` using a multi-level fallback chain.
///
/// Levels (applied in order):
///   1. Exact substring match.
///   2. Line-trimmed sliding window (tolerates indentation drift).
///   3. Smart-quote normalization, then re-run levels 1 and 2.
///
/// Both inputs are CRLF-normalized to LF before matching. On total miss,
/// returns a `MatchFailure` with the best-window similarity ratio plus
/// diagnostic hints.
#[must_use = "match results must be inspected; failures carry diagnostics"]
pub fn find_match(content: &str, old_text: &str) -> Result<MatchResult, MatchFailure> {
    let content = normalize_crlf(content);
    let old = normalize_crlf(old_text);

    // Level 1: exact substring.
    if let Some(r) = exact_match(&content, &old) {
        return Ok(r);
    }

    // Level 2: line-trimmed sliding window.
    if let Some(r) = line_trimmed_match(&content, &old, false) {
        return Ok(r);
    }

    // Level 3: smart-quote normalization, then re-run 1 and 2.
    let norm_content = normalize_quotes(&content);
    let norm_old = normalize_quotes(&old);
    if norm_content != content || norm_old != old {
        if let Some(r) = exact_match_with_original(&norm_content, &norm_old, &content) {
            return Ok(r);
        }
        if let Some(r) = line_trimmed_match(&content, &old, true) {
            return Ok(r);
        }
    }

    // No match — produce a best-window diagnosis.
    let (best_ratio, best_window) = best_fuzzy_window(&content, &old);
    let hints = diagnose_near_match(&old, &best_window);
    Err(MatchFailure { best_ratio, hints })
}

fn normalize_crlf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

fn normalize_quotes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\u{201C}' | '\u{201D}' => out.push('"'),
            '\u{2018}' | '\u{2019}' => out.push('\''),
            other => out.push(other),
        }
    }
    out
}

fn exact_match(content: &str, old: &str) -> Option<MatchResult> {
    if old.is_empty() || !content.contains(old) {
        return None;
    }
    Some(MatchResult {
        matched_text: old.to_string(),
        count: count_occurrences(content, old),
    })
}

/// Like `exact_match` but searches in `normalized` and returns the matched
/// fragment taken from `original` (at the same byte offset and length).
/// Used at level 3 when smart-quote normalization is active.
fn exact_match_with_original(normalized: &str, old: &str, original: &str) -> Option<MatchResult> {
    if old.is_empty() {
        return None;
    }
    let idx = normalized.find(old)?;
    // Normalization is char-for-char (single-char → single-char), so byte
    // offsets are aligned as long as quote chars are in the ASCII fast path.
    // The curly quotes we normalize are multi-byte, so we must slice by
    // matching char offsets rather than byte offsets.
    let char_start = normalized[..idx].chars().count();
    let char_len = old.chars().count();
    let matched: String = original.chars().skip(char_start).take(char_len).collect();
    let count = count_occurrences(normalized, old);
    Some(MatchResult {
        matched_text: matched,
        count,
    })
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0usize;
    let mut start = 0usize;
    while let Some(i) = haystack[start..].find(needle) {
        count += 1;
        let pos = start + i;
        start = pos + needle.len().max(1);
        if start >= haystack.len() {
            break;
        }
    }
    count
}

/// Split a string into its lines without consuming the trailing newlines.
/// Matches Python's `str.splitlines()` semantics that we port from nanobot.
fn split_lines(s: &str) -> Vec<&str> {
    s.split('\n').collect::<Vec<_>>()
}

/// Level 2 matcher: align each line of `old` against a window of N lines in
/// `content` using trimmed (leading+trailing whitespace stripped) comparison.
/// When `normalize_quote_style` is true, also normalize quotes before compare.
fn line_trimmed_match(
    content: &str,
    old: &str,
    normalize_quote_style: bool,
) -> Option<MatchResult> {
    let old_lines = split_lines(old);
    if old_lines.is_empty() {
        return None;
    }
    let content_lines = split_lines(content);
    if content_lines.len() < old_lines.len() {
        return None;
    }

    let stripped_old: Vec<String> = old_lines
        .iter()
        .map(|l| {
            let t = l.trim();
            if normalize_quote_style {
                normalize_quotes(t)
            } else {
                t.to_string()
            }
        })
        .collect();

    let window_size = stripped_old.len();
    let mut hits: Vec<(usize, String)> = Vec::new();

    for i in 0..=(content_lines.len() - window_size) {
        let window = &content_lines[i..i + window_size];
        let comparable: Vec<String> = window
            .iter()
            .map(|l| {
                let t = l.trim();
                if normalize_quote_style {
                    normalize_quotes(t)
                } else {
                    t.to_string()
                }
            })
            .collect();
        if comparable == stripped_old {
            // Join the original window lines back together with '\n'. This
            // reproduces what the original content contains between those
            // line boundaries (sans trailing newline on the last line, which
            // `split('\n')` already consumed).
            let matched = window.join("\n");
            hits.push((i, matched));
        }
    }

    if hits.is_empty() {
        return None;
    }

    // Return the first hit's text; count across all hits.
    Some(MatchResult {
        matched_text: hits[0].1.clone(),
        count: hits.len(),
    })
}

/// Find the content window (same line count as `old_text`) most similar to
/// `old_text` by character-overlap ratio, returning (ratio, window_text).
fn best_fuzzy_window(content: &str, old_text: &str) -> (f64, String) {
    let old_lines = split_lines(old_text);
    let content_lines = split_lines(content);
    let window = old_lines.len().max(1);

    if content_lines.is_empty() {
        return (0.0, String::new());
    }

    let mut best_ratio: f64 = -1.0;
    let mut best_window = String::new();
    let last_start = content_lines.len().saturating_sub(window);
    for i in 0..=last_start {
        let end = (i + window).min(content_lines.len());
        let slice = &content_lines[i..end];
        let candidate = slice.join("\n");
        let ratio = similarity_ratio(old_text, &candidate);
        if ratio > best_ratio {
            best_ratio = ratio;
            best_window = candidate;
        }
    }
    (best_ratio.max(0.0), best_window)
}

/// Character-level similarity in [0.0, 1.0]. Uses a simple bag-of-chars
/// overlap — enough to rank windows for diagnostics without pulling in
/// `difflib`/`strsim`. Empty inputs yield 1.0 when both empty, else 0.0.
fn similarity_ratio(a: &str, b: &str) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    // Build char frequency maps, then sum min-overlaps.
    use std::collections::HashMap;
    let mut freq_a: HashMap<char, usize> = HashMap::new();
    let mut freq_b: HashMap<char, usize> = HashMap::new();
    for c in a.chars() {
        *freq_a.entry(c).or_insert(0) += 1;
    }
    for c in b.chars() {
        *freq_b.entry(c).or_insert(0) += 1;
    }
    let mut overlap = 0usize;
    for (c, n) in &freq_a {
        if let Some(m) = freq_b.get(c) {
            overlap += (*n).min(*m);
        }
    }
    let total = a.chars().count() + b.chars().count();
    (2.0 * overlap as f64) / total as f64
}

fn collapse_internal_whitespace(s: &str) -> String {
    s.split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn diagnose_near_match(old_text: &str, actual_text: &str) -> Vec<String> {
    let mut hints = Vec::new();
    if old_text == actual_text {
        return hints;
    }
    if old_text.to_lowercase() == actual_text.to_lowercase() {
        hints.push("letter case differs".to_string());
    }
    if collapse_internal_whitespace(old_text) == collapse_internal_whitespace(actual_text) {
        hints.push("whitespace differs".to_string());
    }
    if old_text.trim_end_matches('\n') == actual_text.trim_end_matches('\n') {
        hints.push("trailing newline differs".to_string());
    }
    if normalize_quotes(old_text) == normalize_quotes(actual_text) {
        hints.push("quote style differs".to_string());
    }
    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_single() {
        let r = find_match("foo\nbar\nbaz", "bar").unwrap();
        assert_eq!(r.matched_text, "bar");
        assert_eq!(r.count, 1);
    }

    #[test]
    fn line_trimmed_handles_indentation_drift() {
        let content = "    if x {\n        y();\n    }";
        let old = "if x {\n    y();\n}"; // different leading indent
        let r = find_match(content, old).unwrap();
        assert_eq!(r.count, 1);
    }

    #[test]
    fn smart_quote_normalization() {
        let content = "say \u{201C}hi\u{201D}"; // curly
        let old = "say \"hi\""; // straight
        let r = find_match(content, old).unwrap();
        assert_eq!(r.count, 1);
    }

    #[test]
    fn multi_match_reports_count() {
        let r = find_match("a b\na b\na b", "a b").unwrap();
        assert_eq!(r.count, 3);
    }

    #[test]
    fn no_match_returns_error_with_diagnosis() {
        let r = find_match("foo\nbar", "xyz");
        assert!(r.is_err());
    }

    #[test]
    fn crlf_normalized_before_match() {
        let r = find_match("foo\r\nbar\r\nbaz", "foo\nbar").unwrap();
        assert_eq!(r.count, 1);
    }
}
