use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

use crate::host::devql::EDGE_KIND_EXPORTS;

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

pub(crate) fn build_dependency_context_signal(edge_kind: &str, target_ref: &str) -> Option<String> {
    let edge_kind = edge_kind.trim().to_ascii_lowercase();
    // Export edges describe module surface, not what the symbol itself depends on.
    if edge_kind.is_empty() || edge_kind == EDGE_KIND_EXPORTS {
        return None;
    }

    let target = compact_dependency_target(target_ref);
    if target.is_empty() {
        return None;
    }

    Some(format!("{edge_kind}:{target}"))
}

pub(crate) fn render_dependency_context(signals: &[String]) -> String {
    signals
        .iter()
        .map(|signal| signal.replace('_', " "))
        .collect::<Vec<_>>()
        .join(", ")
}

fn compact_dependency_target(target_ref: &str) -> String {
    let trimmed = target_ref.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let namespace_segments = trimmed
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if namespace_segments.len() >= 2 {
        let compact = namespace_segments[namespace_segments.len() - 2..]
            .iter()
            .map(|segment| {
                compact_dependency_segment(segment, segment.contains('/') || segment.contains('\\'))
            })
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        if !compact.is_empty() {
            return compact.join("::");
        }
    }

    compact_dependency_segment(trimmed, trimmed.contains('/') || trimmed.contains('\\'))
}

fn compact_dependency_segment(segment: &str, path_like: bool) -> String {
    let segment = segment.rsplit(['/', '\\']).next().unwrap_or(segment).trim();
    if segment.is_empty() {
        return String::new();
    }

    let mut tokens = split_identifier_tokens(segment);
    if path_like {
        strip_trailing_path_suffix_token(&mut tokens);
    }
    if tokens.is_empty() {
        return segment.to_ascii_lowercase();
    }
    tokens.join("_")
}

fn strip_trailing_path_suffix_token(tokens: &mut Vec<String>) {
    let should_strip = tokens.last().is_some_and(|token| {
        let len = token.len();
        (1..=4).contains(&len) && token.chars().all(|ch| ch.is_ascii_lowercase())
    });
    if should_strip {
        tokens.pop();
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_context_signal_compacts_path_like_targets() {
        let signal = build_dependency_context_signal(
            "calls",
            "src/bounded-contexts/code.ts::ChangePathOfCodeFileCommandHandler::execute",
        )
        .expect("dependency signal");

        assert_eq!(
            signal,
            "calls:change_path_of_code_file_command_handler::execute"
        );
    }

    #[test]
    fn dependency_context_signal_ignores_exports() {
        assert!(build_dependency_context_signal(EDGE_KIND_EXPORTS, "src/app.ts::foo").is_none());
    }

    #[test]
    fn dependency_context_signal_keeps_non_path_dotted_segments() {
        let signal = build_dependency_context_signal("references", "Domain.Event::payload")
            .expect("dependency signal");

        assert_eq!(signal, "references:domain_event::payload");
    }
}
