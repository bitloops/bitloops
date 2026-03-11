use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

use super::MAX_BODY_TOKENS;

pub(super) fn normalize_name(name: &str) -> String {
    let tokens = split_identifier_tokens(name);
    if tokens.is_empty() {
        name.trim().to_ascii_lowercase()
    } else {
        tokens.join("_")
    }
}

pub(super) fn split_identifier_tokens(input: &str) -> Vec<String> {
    let regex = semantic_identifier_regex();
    let mut out = Vec::new();
    for capture in regex.find_iter(input) {
        let raw = capture.as_str();
        for piece in split_camel_case_word(raw) {
            let lowered = piece.to_ascii_lowercase();
            if lowered.is_empty() {
                continue;
            }
            out.push(lowered);
        }
    }
    out
}

pub(super) fn build_body_tokens(body: &str) -> Vec<String> {
    dedupe_tokens(split_identifier_tokens(body), MAX_BODY_TOKENS)
}

pub(super) fn normalize_string_list(values: &[String]) -> Vec<String> {
    dedupe_tokens(values.to_vec(), values.len().max(1))
}

pub(super) fn dedupe_tokens(tokens: Vec<String>, limit: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for token in tokens {
        let normalized = token.trim().to_ascii_lowercase();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        out.push(normalized);
        if out.len() >= limit {
            break;
        }
    }
    out
}

pub(super) fn normalize_repo_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized.trim_start_matches('/').to_string()
}

fn semantic_identifier_regex() -> &'static Regex {
    static IDENTIFIER_REGEX: OnceLock<Regex> = OnceLock::new();
    IDENTIFIER_REGEX.get_or_init(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap())
}

fn split_camel_case_word(input: &str) -> Vec<String> {
    if input.is_empty() {
        return Vec::new();
    }

    let chars = input.chars().collect::<Vec<_>>();
    let mut pieces = Vec::new();
    let mut current = String::new();

    for (idx, ch) in chars.iter().enumerate() {
        if !current.is_empty() {
            let prev = chars[idx - 1];
            let next = chars.get(idx + 1).copied().unwrap_or('\0');
            let boundary = (prev.is_ascii_lowercase() && ch.is_ascii_uppercase())
                || (prev.is_ascii_alphabetic() && ch.is_ascii_digit())
                || (prev.is_ascii_digit() && ch.is_ascii_alphabetic())
                || (prev.is_ascii_uppercase()
                    && ch.is_ascii_uppercase()
                    && next.is_ascii_lowercase());
            if boundary {
                pieces.push(current.clone());
                current.clear();
            }
        }
        current.push(*ch);
    }

    if !current.is_empty() {
        pieces.push(current);
    }

    pieces
}
