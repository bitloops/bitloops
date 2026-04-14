use anyhow::{Context, Result};
use regex::Regex;

#[derive(Debug, Clone)]
pub(super) struct PathPatternMatcher {
    patterns: Vec<Regex>,
}

impl PathPatternMatcher {
    pub(super) fn new(patterns: Vec<String>) -> Result<Self> {
        let patterns = patterns
            .into_iter()
            .map(|pattern| normalize_pattern(&pattern))
            .filter(|pattern| !pattern.is_empty())
            .collect::<Vec<_>>();
        let compiled = patterns
            .iter()
            .map(|pattern| compile_pattern(pattern))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { patterns: compiled })
    }

    pub(super) fn is_match(&self, path: &str) -> bool {
        let path = normalize_relative_path(path);
        !path.is_empty() && self.patterns.iter().any(|pattern| pattern.is_match(&path))
    }
}

fn normalize_pattern(pattern: &str) -> String {
    let mut normalized = pattern.trim().replace('\\', "/");
    let anchored_to_root = normalized.starts_with("./") || normalized.starts_with('/');
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    while normalized.starts_with('/') {
        normalized.remove(0);
    }
    if normalized.ends_with('/') {
        normalized.push_str("**");
    }
    if anchored_to_root && !normalized.is_empty() {
        normalized.insert(0, '/');
    }
    normalized
}

fn normalize_relative_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    while normalized.starts_with('/') {
        normalized.remove(0);
    }
    let mut segments = Vec::new();
    for segment in normalized.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value),
        }
    }
    segments.join("/")
}

fn compile_pattern(pattern: &str) -> Result<Regex> {
    let (anchored_to_root, pattern) = split_root_anchor(pattern);
    if pattern.is_empty() {
        return Regex::new(r"^$").with_context(|| format!("compiling path pattern `{pattern}`"));
    }
    if is_literal_pattern(pattern) {
        let escaped = regex::escape(pattern);
        let prefix = if !anchored_to_root && is_basename_pattern(pattern) {
            "(?:.*/)?"
        } else {
            ""
        };
        let regex = format!("^{prefix}{escaped}(?:/.*)?$");
        return Regex::new(&regex).with_context(|| format!("compiling path pattern `{pattern}`"));
    }

    let mut regex = String::with_capacity(pattern.len() * 2 + 8);
    regex.push('^');
    if !anchored_to_root
        && (is_basename_pattern(pattern) || is_single_dir_descendant_pattern(pattern))
    {
        regex.push_str("(?:.*/)?");
    }
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '*' {
            if index + 1 < chars.len() && chars[index + 1] == '*' {
                while index + 1 < chars.len() && chars[index + 1] == '*' {
                    index += 1;
                }
                if index + 1 < chars.len() && chars[index + 1] == '/' {
                    regex.push_str("(?:.*/)?");
                    index += 2;
                    continue;
                }
                regex.push_str(".*");
                index += 1;
                continue;
            }
            regex.push_str("[^/]*");
            index += 1;
            continue;
        }
        match chars[index] {
            '?' => regex.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' => {
                regex.push('\\');
                regex.push(chars[index]);
            }
            other => regex.push(other),
        }
        index += 1;
    }
    if should_match_folder_descendants(pattern) {
        regex.push_str("(?:/.*)?");
    }
    regex.push('$');
    Regex::new(&regex).with_context(|| format!("compiling path pattern `{pattern}`"))
}

fn split_root_anchor(pattern: &str) -> (bool, &str) {
    if let Some(stripped) = pattern.strip_prefix('/') {
        (true, stripped)
    } else {
        (false, pattern)
    }
}

fn is_literal_pattern(pattern: &str) -> bool {
    !pattern.contains('*') && !pattern.contains('?')
}

fn is_basename_pattern(pattern: &str) -> bool {
    !pattern.contains('/')
}

fn is_single_dir_descendant_pattern(pattern: &str) -> bool {
    pattern.ends_with("/**") && !pattern[..pattern.len().saturating_sub(3)].contains('/')
}

fn should_match_folder_descendants(pattern: &str) -> bool {
    if pattern.ends_with("/**") {
        return false;
    }
    let Some(last_segment) = pattern.rsplit('/').next() else {
        return false;
    };
    !last_segment.is_empty() && !last_segment.contains('*') && !last_segment.contains('?')
}
