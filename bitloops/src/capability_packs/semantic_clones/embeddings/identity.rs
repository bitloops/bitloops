use std::sync::OnceLock;

use regex::Regex;

use super::types::SymbolEmbeddingInput;

pub(super) fn normalize_identity_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized.trim_start_matches('/').to_string()
}

pub(super) fn normalize_identity_terms(input: &str) -> String {
    split_identity_tokens(input).join(" ")
}

pub(super) fn normalize_identity_path_terms(path: &str) -> String {
    let mut tokens = split_identity_tokens(path);
    strip_trailing_identity_path_suffix(&mut tokens);
    tokens.join(" ")
}

pub(super) fn identity_container_raw(input: &SymbolEmbeddingInput) -> String {
    let normalized_path = normalize_identity_path(&input.path);
    let mut segments = input
        .symbol_fqn
        .trim()
        .replace('\\', "/")
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if segments
        .first()
        .is_some_and(|segment| normalize_identity_path(segment) == normalized_path)
    {
        segments.remove(0);
    }
    if !segments.is_empty() {
        segments.pop();
    }
    segments.join("::")
}

fn split_identity_tokens(input: &str) -> Vec<String> {
    let regex = identity_identifier_regex();
    let mut out = Vec::new();
    for capture in regex.find_iter(input) {
        let raw = capture.as_str();
        for chunk in raw.split('_') {
            for piece in split_identity_camel_case_word(chunk) {
                let lowered = piece.to_ascii_lowercase();
                if lowered.is_empty() {
                    continue;
                }
                out.push(lowered);
            }
        }
    }
    out
}

fn strip_trailing_identity_path_suffix(tokens: &mut Vec<String>) {
    let should_strip = tokens.last().is_some_and(|token| {
        let len = token.len();
        (1..=4).contains(&len) && token.chars().all(|ch| ch.is_ascii_lowercase())
    });
    if should_strip {
        tokens.pop();
    }
}

fn identity_identifier_regex() -> &'static Regex {
    static IDENTIFIER_REGEX: OnceLock<Regex> = OnceLock::new();
    IDENTIFIER_REGEX.get_or_init(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").expect("valid regex"))
}

fn split_identity_camel_case_word(input: &str) -> Vec<String> {
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
