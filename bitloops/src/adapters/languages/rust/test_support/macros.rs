use std::collections::HashSet;

use tree_sitter::Node;

use crate::host::language_adapter::{
    DiscoveredTestScenario, ReferenceCandidate, ScenarioDiscoverySource,
};

use super::attributes::find_matching_delimiter;
use super::scenarios::{RustDiscoveredScenario, rust_suite_for_node};

pub(crate) fn collect_rust_macro_generated_scenarios(
    root: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> Vec<RustDiscoveredScenario> {
    let test_macro_names = collect_rust_test_generating_macro_names(root, source);
    if test_macro_names.is_empty() {
        return Vec::new();
    }

    let mut scenarios = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "macro_invocation"
            && let Ok(raw_invocation) = node.utf8_text(source)
            && let Some(macro_name) = extract_rust_macro_invocation_name(raw_invocation)
            && test_macro_names.contains(macro_name)
            && let Some(body) = extract_rust_macro_invocation_body(raw_invocation)
            && let Some(scenario_name) = extract_first_identifier_token(body)
        {
            let (suite_name, suite_start_line, suite_end_line) =
                rust_suite_for_node(node, source, relative_path);
            scenarios.push(RustDiscoveredScenario {
                suite_name,
                suite_start_line,
                suite_end_line,
                scenario: DiscoveredTestScenario {
                    name: scenario_name,
                    start_line: node.start_position().row as i64 + 1,
                    end_line: node.end_position().row as i64 + 1,
                    reference_candidates: extract_callable_symbols_from_rust_text(body)
                        .into_iter()
                        .map(ReferenceCandidate::SymbolName)
                        .collect(),
                    discovery_source: ScenarioDiscoverySource::MacroGenerated,
                },
            });
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    scenarios
}

pub(crate) fn collect_rust_proptest_scenarios(
    root: Node<'_>,
    source: &str,
    relative_path: &str,
) -> Vec<RustDiscoveredScenario> {
    let mut scenarios = Vec::new();
    let mut stack = vec![root];
    let source_bytes = source.as_bytes();

    while let Some(node) = stack.pop() {
        if node.kind() == "macro_invocation"
            && let Ok(raw_invocation) = node.utf8_text(source_bytes)
            && let Some(macro_name) = extract_rust_macro_invocation_name(raw_invocation)
            && macro_name == "proptest"
            && let Some(body) = extract_rust_macro_invocation_body(raw_invocation)
        {
            let (suite_name, suite_start_line, suite_end_line) =
                rust_suite_for_node(node, source_bytes, relative_path);
            let invocation_start_line = node.start_position().row as i64 + 1;
            for proptest_case in extract_proptest_cases(body) {
                scenarios.push(RustDiscoveredScenario {
                    suite_name: suite_name.clone(),
                    suite_start_line,
                    suite_end_line,
                    scenario: DiscoveredTestScenario {
                        name: proptest_case.name,
                        start_line: invocation_start_line + proptest_case.start_line_offset,
                        end_line: invocation_start_line + proptest_case.end_line_offset,
                        reference_candidates: extract_callable_symbols_from_rust_text(
                            &proptest_case.body,
                        )
                        .into_iter()
                        .map(ReferenceCandidate::SymbolName)
                        .collect(),
                        discovery_source: ScenarioDiscoverySource::Source,
                    },
                });
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    scenarios
}

pub(crate) fn extract_rust_macro_invocation_body(raw_invocation: &str) -> Option<&str> {
    let bang_index = raw_invocation.find('!')?;
    let remainder = &raw_invocation[bang_index + 1..];
    let open_relative = remainder.find(['(', '{', '['])?;
    let open_index = bang_index + 1 + open_relative;
    let close_index = find_matching_delimiter(raw_invocation, open_index)?;
    Some(raw_invocation[open_index + 1..close_index].trim())
}

fn collect_rust_test_generating_macro_names(root: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "macro_definition"
            && let Ok(raw_definition) = node.utf8_text(source)
            && super::rust_source_contains_test_markers(raw_definition)
            && let Some(name) = extract_rust_macro_definition_name(raw_definition)
        {
            names.insert(name.to_string());
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    names
}

#[derive(Debug, Clone)]
struct ProptestCase {
    name: String,
    body: String,
    start_line_offset: i64,
    end_line_offset: i64,
}

fn extract_proptest_cases(body: &str) -> Vec<ProptestCase> {
    let mut cases = Vec::new();
    let mut search_start = 0usize;

    while let Some(relative_idx) = body[search_start..].find("fn ") {
        let fn_index = search_start + relative_idx;
        let after_fn = fn_index + 3;
        let name_tail = &body[after_fn..];
        let name_len = name_tail
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .unwrap_or(name_tail.len());
        if name_len == 0 {
            search_start = after_fn;
            continue;
        }

        let name = name_tail[..name_len].to_string();
        let open_brace = match body[after_fn + name_len..].find('{') {
            Some(idx) => after_fn + name_len + idx,
            None => break,
        };
        let Some(close_brace) = find_matching_delimiter(body, open_brace) else {
            break;
        };
        let body_text = body[open_brace + 1..close_brace].trim().to_string();
        let start_line_offset = body[..fn_index].lines().count() as i64;
        let end_line_offset = body[..close_brace].lines().count() as i64;
        cases.push(ProptestCase {
            name,
            body: body_text,
            start_line_offset,
            end_line_offset,
        });
        search_start = close_brace + 1;
    }

    cases
}

fn extract_rust_macro_definition_name(raw_definition: &str) -> Option<&str> {
    let (_, remainder) = raw_definition.split_once("macro_rules!")?;
    let trimmed = remainder.trim_start();
    let end = trimmed
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .unwrap_or(trimmed.len());
    (end > 0).then_some(&trimmed[..end])
}

fn extract_rust_macro_invocation_name(raw_invocation: &str) -> Option<&str> {
    let bang_index = raw_invocation.find('!')?;
    let prefix = raw_invocation[..bang_index].trim();
    let name = prefix.rsplit("::").next().unwrap_or(prefix).trim();
    (!name.is_empty()).then_some(name)
}

fn extract_first_identifier_token(raw: &str) -> Option<String> {
    let mut chars = raw.char_indices().peekable();

    while let Some((start, ch)) = chars.next() {
        if !is_rust_identifier_start(ch) {
            continue;
        }

        let mut end = start + ch.len_utf8();
        while let Some((idx, next)) = chars.peek().copied() {
            if !is_rust_identifier_continue(next) {
                break;
            }
            end = idx + next.len_utf8();
            chars.next();
        }

        return Some(raw[start..end].to_string());
    }

    None
}

fn extract_callable_symbols_from_rust_text(raw: &str) -> HashSet<String> {
    let mut symbols = HashSet::new();
    let chars: Vec<char> = raw.chars().collect();

    for (idx, ch) in chars.iter().enumerate() {
        if *ch != '(' {
            continue;
        }

        let mut start = idx;
        while start > 0 {
            let previous = chars[start - 1];
            if previous.is_ascii_alphanumeric() || matches!(previous, '_' | ':' | '.') {
                start -= 1;
            } else {
                break;
            }
        }
        if start == idx {
            continue;
        }

        let token: String = chars[start..idx].iter().collect();
        let token = token.trim_matches(':').trim_matches('.');
        if token.is_empty() {
            continue;
        }

        let simple = token
            .rsplit("::")
            .next()
            .unwrap_or(token)
            .rsplit('.')
            .next()
            .unwrap_or(token);
        if simple.is_empty()
            || is_rust_non_callable_token(simple)
            || !simple.chars().next().is_some_and(is_rust_identifier_start)
            || !simple.chars().all(is_rust_identifier_continue)
        {
            continue;
        }

        symbols.insert(simple.to_string());
    }

    symbols
}

fn is_rust_identifier_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_rust_identifier_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_rust_non_callable_token(token: &str) -> bool {
    matches!(
        token,
        "if" | "for" | "while" | "match" | "loop" | "return" | "type_property_test"
    )
}
