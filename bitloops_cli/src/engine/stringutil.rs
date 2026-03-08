//! UTF-8 safe string manipulation utilities.

use regex::Regex;
use std::sync::OnceLock;

/// Replaces runs of whitespace with a single space and trims ends.
///
/// Treat tabs/newlines/CRLF as
/// whitespace and normalize the output to a single-line string.
pub fn collapse_whitespace(s: &str) -> String {
    static WHITESPACE_RE: OnceLock<Regex> = OnceLock::new();
    let regex = WHITESPACE_RE.get_or_init(|| Regex::new(r"\s+").expect("valid whitespace regex"));
    regex.replace_all(s, " ").trim().to_string()
}

/// Truncates a string to at most `max_runes` Unicode scalar values and appends
/// `suffix` only when truncation occurs.
///
/// Truncation budget is rune-based,
/// suffix budget is reserved, and UTF-8 boundaries are preserved.
pub fn truncate_runes(s: &str, max_runes: usize, suffix: &str) -> String {
    if s.chars().count() <= max_runes {
        return s.to_string();
    }

    let suffix_runes = suffix.chars().count();
    let truncate_at = max_runes.saturating_sub(suffix_runes);
    let prefix: String = s.chars().take(truncate_at).collect();

    let mut out = prefix;
    out.push_str(suffix);
    out
}

/// Capitalizes the first Unicode scalar value and leaves the remainder unchanged.
///
/// Empty input is returned as-is and
/// non-cased leading characters remain unchanged.
pub fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return s.to_string();
    };
    let upper = first.to_uppercase().next().unwrap_or(first);
    let mut out = String::new();
    out.push(upper);
    out.push_str(chars.as_str());
    out
}

#[allow(non_snake_case)]
pub fn CollapseWhitespace(s: &str) -> String {
    collapse_whitespace(s)
}

#[allow(non_snake_case)]
pub fn TruncateRunes(s: &str, maxRunes: usize, suffix: &str) -> String {
    truncate_runes(s, maxRunes, suffix)
}

#[allow(non_snake_case)]
pub fn CapitalizeFirst(s: &str) -> String {
    capitalize_first(s)
}

#[cfg(test)]
mod tests {
    use super::{capitalize_first, collapse_whitespace, truncate_runes};

    #[test]
    fn test_collapse_whitespace() {
        let tests = [
            ("no whitespace changes needed", "hello world", "hello world"),
            ("newlines to space", "hello\nworld", "hello world"),
            ("multiple newlines", "hello\n\n\nworld", "hello world"),
            ("tabs to space", "hello\tworld", "hello world"),
            ("mixed whitespace", "hello\n\t  world", "hello world"),
            (
                "leading and trailing whitespace",
                "  hello world  ",
                "hello world",
            ),
            (
                "multiline text",
                "Fix the bug\nin the login\npage",
                "Fix the bug in the login page",
            ),
            ("empty string", "", ""),
            ("only whitespace", "  \n\t  ", ""),
            ("carriage return", "hello\r\nworld", "hello world"),
        ];

        for (name, input, want) in tests {
            let got = collapse_whitespace(input);
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn test_truncate_runes() {
        let tests = [
            (
                "ascii no truncation needed",
                "hello",
                10usize,
                "...",
                "hello",
            ),
            ("ascii truncation", "hello world", 8usize, "...", "hello..."),
            (
                "emoji no truncation needed",
                "hello 🎉",
                10usize,
                "...",
                "hello 🎉",
            ),
            (
                "emoji truncation preserves emoji",
                "hello 🎉 world",
                10usize,
                "...",
                "hello 🎉...",
            ),
            ("chinese characters", "你好世界", 3usize, "...", "..."),
            (
                "chinese characters longer",
                "你好世界再见",
                5usize,
                "...",
                "你好...",
            ),
            (
                "mixed unicode needs truncation",
                "hello 世界 🎉 more",
                10usize,
                "...",
                "hello 世...",
            ),
            ("empty string", "", 10usize, "...", ""),
            ("exact length", "hello", 5usize, "...", "hello"),
            ("no suffix", "hello world", 5usize, "", "hello"),
        ];

        for (name, input, max_runes, suffix, want) in tests {
            let got = truncate_runes(input, max_runes, suffix);
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn test_capitalize_first() {
        let tests = [
            ("ascii lowercase", "hello", "Hello"),
            ("ascii already uppercase", "Hello", "Hello"),
            ("empty string", "", ""),
            ("single char", "h", "H"),
            ("unicode lowercase", "über", "Über"),
            ("starts with emoji", "🎉party", "🎉party"),
            ("chinese character", "你好", "你好"),
            ("greek lowercase", "αβγ", "Αβγ"),
        ];

        for (name, input, want) in tests {
            let got = capitalize_first(input);
            assert_eq!(got, want, "{name}");
        }
    }
}
