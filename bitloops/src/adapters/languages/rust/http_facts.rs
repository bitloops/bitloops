use serde_json::json;

use crate::host::language_adapter::{
    LanguageHttpFact, LanguageHttpFactArtefact, LanguageHttpFactEvidence, LanguageHttpFactFile,
};

const HTTP_PRIMITIVE_LOSSY_TRANSFORM: &str = "LossyTransform";
const HTTP_ROLE_BODY_REPLACEMENT: &str = "http.response.body_replacement";
const HTTP_ROLE_BODY_STRIPPING: &str = "http.response.body_stripping";

pub(crate) fn extract_rust_http_facts(
    file: &LanguageHttpFactFile,
    content: &str,
    artefacts: &[LanguageHttpFactArtefact],
) -> Vec<LanguageHttpFact> {
    let mut facts = Vec::new();
    for artefact in artefacts.iter().filter(|artefact| is_callable(artefact)) {
        let Some((artefact_start, body)) =
            content_slice(content, artefact.start_byte, artefact.end_byte)
        else {
            continue;
        };
        let display_symbol_fqn = display_symbol_fqn(content, file, artefact);
        for candidate in body_replacement_candidates(content, artefact_start, body, artefact) {
            let mut roles = vec![HTTP_ROLE_BODY_REPLACEMENT.to_string()];
            if candidate.body_stripping {
                roles.push(HTTP_ROLE_BODY_STRIPPING.to_string());
            }
            let mut terms = vec![
                "HTTP".to_string(),
                "response body".to_string(),
                "body replacement".to_string(),
                "lossy transform".to_string(),
            ];
            if candidate.body_stripping {
                terms.push("body stripping".to_string());
            }
            if candidate.guard_condition.is_some() {
                terms.push("HEAD".to_string());
            }

            facts.push(LanguageHttpFact {
                stable_key: format!(
                    "rust.http.lossy_body_transform:{}:{}:{}",
                    file.path, display_symbol_fqn, candidate.start_line
                ),
                primitive_type: HTTP_PRIMITIVE_LOSSY_TRANSFORM.to_string(),
                subject: format!(
                    "Rust callable `{display_symbol_fqn}` replaces an HTTP response body before response serialisation"
                ),
                roles,
                terms,
                properties: json!({
                    "sourceCategory": "language_ecosystem_http_fact",
                    "language": "rust",
                    "operationPattern": candidate.operation_pattern,
                    "receiver": candidate.receiver,
                    "guardCondition": candidate.guard_condition,
                    "replacementKind": candidate.replacement_kind,
                    "closureParameter": candidate.closure_parameter,
                    "originalBodyIgnored": candidate.original_body_ignored,
                    "destroyedSignals": ["body_exact_size_signal", "body_size_hint"],
                    "detectedSignals": candidate.detected_signals,
                    "confidenceSource": candidate.confidence_source,
                    "parserVersion": file.parser_version,
                    "extractorVersion": file.extractor_version,
                    "symbolFqn": artefact.symbol_fqn,
                    "displaySymbolFqn": display_symbol_fqn,
                }),
                confidence_level: candidate.confidence_level,
                confidence_score: candidate.confidence_score,
                evidence: vec![LanguageHttpFactEvidence {
                    path: file.path.clone(),
                    artefact_id: Some(artefact.artefact_id.clone()),
                    symbol_id: Some(artefact.symbol_id.clone()),
                    content_id: file.content_id.clone(),
                    start_line: Some(candidate.start_line),
                    end_line: Some(candidate.end_line),
                    start_byte: Some(candidate.start_byte),
                    end_byte: Some(candidate.end_byte),
                    properties: json!({
                        "symbolFqn": artefact.symbol_fqn,
                        "displaySymbolFqn": display_symbol_fqn,
                        "signature": artefact.signature,
                        "operationPattern": candidate.operation_pattern,
                        "receiver": candidate.receiver,
                    }),
                }],
            });
        }
    }
    facts
}

#[derive(Debug, Clone, PartialEq)]
struct BodyReplacementCandidate {
    operation_pattern: String,
    receiver: Option<String>,
    guard_condition: Option<String>,
    replacement_kind: String,
    closure_parameter: Option<String>,
    original_body_ignored: bool,
    body_stripping: bool,
    detected_signals: Vec<String>,
    confidence_source: String,
    confidence_level: String,
    confidence_score: f64,
    start_line: i32,
    end_line: i32,
    start_byte: i32,
    end_byte: i32,
}

fn is_callable(artefact: &LanguageHttpFactArtefact) -> bool {
    matches!(
        artefact.canonical_kind.as_deref(),
        Some("function" | "method" | "callable")
    ) || matches!(
        artefact.language_kind.as_str(),
        "function_item" | "method_declaration"
    )
}

fn content_slice(content: &str, start_byte: i32, end_byte: i32) -> Option<(usize, &str)> {
    let start = usize::try_from(start_byte).ok()?;
    let end = usize::try_from(end_byte).ok()?;
    if start > end
        || end > content.len()
        || !content.is_char_boundary(start)
        || !content.is_char_boundary(end)
    {
        return None;
    }
    content.get(start..end).map(|slice| (start, slice))
}

fn body_replacement_candidates(
    content: &str,
    artefact_start: usize,
    body: &str,
    artefact: &LanguageHttpFactArtefact,
) -> Vec<BodyReplacementCandidate> {
    let mut candidates = Vec::new();
    for (pattern, operation_pattern) in [
        (".map(", "response_map_method"),
        ("Response::map(", "response_map_associated"),
    ] {
        for pattern_start in find_all(body, pattern) {
            let open_paren = pattern_start + pattern.len() - 1;
            let Some(close_paren) = find_matching_delimiter(body, open_paren, '(', ')') else {
                continue;
            };
            let expression_start = expression_start(body, pattern_start);
            let expression_end = close_paren + 1;
            let Some(call_text) = body.get(pattern_start..expression_end) else {
                continue;
            };
            let Some(closure) = extract_first_closure(call_text) else {
                continue;
            };
            let replacement_kind = replacement_kind(closure.parameter, closure.body);
            let guard_condition = guard_condition(body, artefact);
            let receiver = if operation_pattern == "response_map_method" {
                map_receiver(body, pattern_start)
            } else {
                Some("Response".to_string())
            };
            if !response_map_context(
                operation_pattern,
                receiver.as_deref(),
                body,
                artefact,
                &guard_condition,
            ) {
                continue;
            }
            let original_body_ignored = closure_parameter_ignored(closure.parameter, closure.body);
            if replacement_kind == "body_preserving" {
                continue;
            }
            if !original_body_ignored && replacement_kind != "empty_body" {
                continue;
            }
            let body_stripping = replacement_kind == "empty_body"
                || guard_condition_indicates_stripping(&guard_condition);
            let mut detected_signals = vec![operation_pattern.to_string()];
            if original_body_ignored {
                detected_signals.push("closure_discards_original_body".to_string());
            }
            if body_stripping {
                detected_signals.push("body_stripping".to_string());
            }
            detected_signals.push(replacement_kind.to_string());
            if let Some(guard) = &guard_condition {
                detected_signals.push(format!("guard:{guard}"));
            }

            let global_start = artefact_start + expression_start;
            let global_end = artefact_start + expression_end;
            candidates.push(BodyReplacementCandidate {
                operation_pattern: operation_pattern.to_string(),
                receiver,
                guard_condition,
                replacement_kind: replacement_kind.to_string(),
                closure_parameter: Some(closure.parameter.to_string()),
                original_body_ignored,
                body_stripping,
                detected_signals,
                confidence_source:
                    "rust_language_adapter.static_pattern.response_body_map_replacement"
                        .to_string(),
                confidence_level: "HIGH".to_string(),
                confidence_score: if replacement_kind == "empty_body" {
                    0.9
                } else {
                    0.82
                },
                start_line: line_for_byte(content, global_start),
                end_line: line_for_byte(content, global_end),
                start_byte: i32::try_from(global_start).unwrap_or(artefact.start_byte),
                end_byte: i32::try_from(global_end).unwrap_or(artefact.end_byte),
            });
        }
    }
    candidates
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClosureParts<'a> {
    parameter: &'a str,
    body: &'a str,
}

fn extract_first_closure(call_text: &str) -> Option<ClosureParts<'_>> {
    let first_pipe = call_text.find('|')?;
    let tail = call_text.get(first_pipe + 1..)?;
    let second_relative = tail.find('|')?;
    let second_pipe = first_pipe + 1 + second_relative;
    let parameter = call_text.get(first_pipe + 1..second_pipe)?.trim();
    let body = call_text.get(second_pipe + 1..)?.trim();
    Some(ClosureParts { parameter, body })
}

fn replacement_kind(parameter: &str, replacement_body: &str) -> &'static str {
    if closure_body_preserves_parameter(parameter, replacement_body) {
        return "body_preserving";
    }

    let lower = replacement_body.to_ascii_lowercase();
    if replacement_body.contains("Empty::new(")
        || replacement_body.contains("Body::empty(")
        || lower.contains("empty_body")
        || lower.contains("emptybody")
    {
        "empty_body"
    } else {
        "unrelated_body"
    }
}

fn closure_parameter_ignored(parameter: &str, replacement_body: &str) -> bool {
    let Some(parameter) = closure_parameter_name(parameter) else {
        return false;
    };
    if parameter == "_" || parameter.starts_with('_') {
        return true;
    }
    !contains_identifier(replacement_body, &parameter)
}

fn closure_body_preserves_parameter(parameter: &str, replacement_body: &str) -> bool {
    let Some(parameter) = closure_parameter_name(parameter) else {
        return false;
    };
    normalise_closure_expression(replacement_body) == parameter
}

fn closure_parameter_name(parameter: &str) -> Option<String> {
    let mut parameter = parameter.trim();
    if parameter.contains(',') {
        return None;
    }
    parameter = parameter.strip_prefix('&').unwrap_or(parameter).trim();
    parameter = parameter.strip_prefix("mut ").unwrap_or(parameter).trim();
    parameter = parameter.strip_prefix('&').unwrap_or(parameter).trim();
    parameter = parameter.strip_prefix("mut ").unwrap_or(parameter).trim();
    let parameter = parameter
        .split_once(':')
        .map(|(name, _)| name)
        .unwrap_or(parameter)
        .trim();
    (!parameter.is_empty() && parameter.chars().all(is_identifier_char))
        .then(|| parameter.to_string())
}

fn normalise_closure_expression(expression: &str) -> String {
    let mut expression = expression.trim();
    while let Some(stripped) = expression
        .strip_suffix(')')
        .or_else(|| expression.strip_suffix(';'))
        .or_else(|| expression.strip_suffix(','))
    {
        expression = stripped.trim_end();
    }
    if let Some(inner) = expression
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
    {
        expression = inner.trim();
        while let Some(stripped) = expression.strip_suffix(';') {
            expression = stripped.trim_end();
        }
    }
    expression.trim().to_string()
}

fn map_receiver(body: &str, pattern_start: usize) -> Option<String> {
    let prefix = body.get(..pattern_start)?.trim_end();
    let receiver_start = prefix
        .rfind(|ch: char| !is_identifier_char(ch))
        .map(|index| index + 1)
        .unwrap_or(0);
    let receiver = prefix.get(receiver_start..)?.trim();
    (!receiver.is_empty()).then(|| receiver.to_string())
}

fn response_map_context(
    operation_pattern: &str,
    receiver: Option<&str>,
    body: &str,
    artefact: &LanguageHttpFactArtefact,
    guard_condition: &Option<String>,
) -> bool {
    if operation_pattern == "response_map_associated" {
        return true;
    }
    if receiver.is_some_and(receiver_indicates_response) {
        return true;
    }
    if guard_condition_indicates_stripping(guard_condition) {
        return true;
    }
    let signature = artefact.signature.as_deref().unwrap_or_default();
    signature.contains("Response<")
        || signature.contains("http::Response")
        || body.contains("Response<")
        || body.contains("http::Response")
}

fn receiver_indicates_response(receiver: &str) -> bool {
    matches!(
        receiver.to_ascii_lowercase().as_str(),
        "res" | "resp" | "response" | "rsp"
    )
}

fn guard_condition(body: &str, artefact: &LanguageHttpFactArtefact) -> Option<String> {
    let lower_body = body.to_ascii_lowercase();
    let lower_symbol = artefact.symbol_fqn.to_ascii_lowercase();
    if body.contains("Method::HEAD") || lower_body.contains("method::head") {
        Some("Method::HEAD".to_string())
    } else if lower_body.contains("strip_body") || lower_symbol.contains("strip_body") {
        Some("strip_body".to_string())
    } else {
        None
    }
}

fn guard_condition_indicates_stripping(guard_condition: &Option<String>) -> bool {
    matches!(
        guard_condition.as_deref(),
        Some("Method::HEAD" | "strip_body")
    )
}

fn find_all(haystack: &str, needle: &str) -> Vec<usize> {
    let mut matches = Vec::new();
    let mut offset = 0;
    while let Some(relative) = haystack[offset..].find(needle) {
        let absolute = offset + relative;
        matches.push(absolute);
        offset = absolute + needle.len();
    }
    matches
}

fn expression_start(text: &str, pattern_start: usize) -> usize {
    text[..pattern_start]
        .rfind(['\n', ';', '{', '}'])
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn find_matching_delimiter(
    text: &str,
    open_index: usize,
    open: char,
    close: char,
) -> Option<usize> {
    let mut depth = 0_i32;
    for (index, ch) in text
        .char_indices()
        .skip_while(|(index, _)| *index < open_index)
    {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn line_for_byte(content: &str, byte: usize) -> i32 {
    let safe_byte = byte.min(content.len());
    let prefix = content.get(..safe_byte).unwrap_or_default();
    i32::try_from(prefix.bytes().filter(|byte| *byte == b'\n').count() + 1).unwrap_or(1)
}

fn contains_identifier(text: &str, identifier: &str) -> bool {
    text.match_indices(identifier).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let after = text[start + identifier.len()..].chars().next();
        !before.is_some_and(is_identifier_char) && !after.is_some_and(is_identifier_char)
    })
}

fn is_identifier_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn display_symbol_fqn(
    content: &str,
    file: &LanguageHttpFactFile,
    artefact: &LanguageHttpFactArtefact,
) -> String {
    if !artefact.symbol_fqn.contains("impl@") {
        return artefact.symbol_fqn.clone();
    }
    let Some(method_name) = rust_callable_name(artefact) else {
        return artefact.symbol_fqn.clone();
    };
    let Some(owner) = enclosing_impl_owner(content, artefact.start_byte) else {
        return artefact.symbol_fqn.clone();
    };
    format!("{}::{owner}::{method_name}", file.path)
}

fn rust_callable_name(artefact: &LanguageHttpFactArtefact) -> Option<String> {
    if let Some(signature) = artefact.signature.as_deref()
        && let Some(after_fn) = signature.trim().strip_prefix("fn ")
    {
        let name = after_fn
            .split(|ch: char| !(ch == '_' || ch.is_ascii_alphanumeric()))
            .next()
            .unwrap_or_default()
            .trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    artefact
        .symbol_fqn
        .rsplit("::")
        .next()
        .map(str::to_string)
        .filter(|name| !name.is_empty())
}

fn enclosing_impl_owner(content: &str, artefact_start_byte: i32) -> Option<String> {
    let artefact_start = usize::try_from(artefact_start_byte).ok()?;
    let prefix = content.get(..artefact_start)?;
    let mut search_end = prefix.len();
    let impl_start = loop {
        let Some(index) = prefix.get(..search_end)?.rfind("impl") else {
            break None;
        };
        let before = prefix[..index].chars().next_back();
        let after = prefix[index + "impl".len()..].chars().next();
        if !before.is_some_and(is_identifier_char)
            && after.is_some_and(|ch| ch.is_whitespace() || ch == '<')
        {
            break Some(index);
        }
        search_end = index;
    }?;
    let header_tail = content.get(impl_start..)?;
    let open_brace = header_tail.find('{')?;
    let header = header_tail.get(..open_brace)?.trim();
    let owner = if let Some((_, target)) = header.rsplit_once(" for ") {
        target
    } else {
        header.trim_start_matches("impl").trim()
    };
    let owner = owner
        .split(['<', '{', ' ', '\n', '\t'])
        .next()
        .unwrap_or_default()
        .rsplit("::")
        .next()
        .unwrap_or_default()
        .trim();
    (!owner.is_empty()).then(|| owner.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::languages::rust::extraction::extract_rust_artefacts;

    #[test]
    fn replacement_kind_classifies_body_preserving_passthrough() {
        assert_eq!(replacement_kind("body", "body)"), "body_preserving");
        assert_eq!(
            replacement_kind("&mut body: B", "{ body })"),
            "body_preserving"
        );
        assert_eq!(replacement_kind("_", "boxed(Empty::new()))"), "empty_body");
        assert_eq!(replacement_kind("body", "boxed(other))"), "unrelated_body");
    }

    #[test]
    fn rust_http_fact_detects_empty_body_replacement_without_ecosystem_names() {
        let content = r#"
use http::Response;

fn strip_body<B>(res: Response<B>) -> Response<()> {
    res.map(|_| boxed(Empty::new()))
}
"#;
        let artefacts = extract_rust_artefacts(content, "src/response.rs").expect("artefacts");
        let http_artefacts = artefacts
            .into_iter()
            .map(|artefact| LanguageHttpFactArtefact {
                symbol_id: format!("symbol:{}", artefact.symbol_fqn),
                artefact_id: format!("artefact:{}", artefact.symbol_fqn),
                symbol_fqn: artefact.symbol_fqn,
                canonical_kind: artefact.canonical_kind,
                language_kind: artefact.language_kind.as_str().to_string(),
                start_line: artefact.start_line,
                end_line: artefact.end_line,
                start_byte: artefact.start_byte,
                end_byte: artefact.end_byte,
                signature: Some(artefact.signature),
            })
            .collect::<Vec<_>>();
        let file = LanguageHttpFactFile {
            repo_id: "repo-1".to_string(),
            path: "src/response.rs".to_string(),
            language: "rust".to_string(),
            content_id: "content-1".to_string(),
            parser_version: "tree-sitter-rust@1".to_string(),
            extractor_version: "rust-language-pack@1".to_string(),
        };

        let facts = extract_rust_http_facts(&file, content, &http_artefacts);

        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].primitive_type, "LossyTransform");
        assert!(
            facts[0]
                .roles
                .contains(&HTTP_ROLE_BODY_REPLACEMENT.to_string())
        );
        assert!(
            facts[0]
                .roles
                .contains(&HTTP_ROLE_BODY_STRIPPING.to_string())
        );
        let fact_debug = format!("{:?}", facts[0]);
        assert!(!fact_debug.contains("axum"));
    }

    #[test]
    fn rust_http_fact_ignores_body_preserving_response_map() {
        let content = r#"
use http::Response;

fn preserve_body<B>(res: Response<B>) -> Response<B> {
    res.map(|body| body)
}
"#;
        let artefacts = extract_rust_artefacts(content, "src/response.rs").expect("artefacts");
        let http_artefacts = artefacts
            .into_iter()
            .map(|artefact| LanguageHttpFactArtefact {
                symbol_id: format!("symbol:{}", artefact.symbol_fqn),
                artefact_id: format!("artefact:{}", artefact.symbol_fqn),
                symbol_fqn: artefact.symbol_fqn,
                canonical_kind: artefact.canonical_kind,
                language_kind: artefact.language_kind.as_str().to_string(),
                start_line: artefact.start_line,
                end_line: artefact.end_line,
                start_byte: artefact.start_byte,
                end_byte: artefact.end_byte,
                signature: Some(artefact.signature),
            })
            .collect::<Vec<_>>();
        let file = LanguageHttpFactFile {
            repo_id: "repo-1".to_string(),
            path: "src/response.rs".to_string(),
            language: "rust".to_string(),
            content_id: "content-1".to_string(),
            parser_version: "tree-sitter-rust@1".to_string(),
            extractor_version: "rust-language-pack@1".to_string(),
        };

        let facts = extract_rust_http_facts(&file, content, &http_artefacts);

        assert!(facts.is_empty());
    }
}
