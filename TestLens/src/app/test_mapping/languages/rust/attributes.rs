use std::collections::HashSet;

use tree_sitter::Node;

use crate::app::test_mapping::model::{
    ReferenceCandidate, ScenarioDiscoverySource,
};

use super::scenarios::RustScenarioSeed;

pub(crate) fn rust_attribute_name(raw_attribute: &str) -> Option<String> {
    let compact: String = raw_attribute.chars().filter(|c| !c.is_whitespace()).collect();
    let stripped = compact.strip_prefix("#[")?.trim_end_matches(']');
    let name = stripped.split_once('(').map(|(name, _)| name).unwrap_or(stripped);
    name.rsplit("::")
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

pub(crate) fn extract_rust_attribute_args(raw_attribute: &str) -> Option<&str> {
    let compact: String = raw_attribute.chars().filter(|c| !c.is_whitespace()).collect();
    let open = compact.find('(')?;
    let close = compact.rfind(')')?;
    (close > open).then_some(
        &raw_attribute[raw_attribute.find('(')? + 1..raw_attribute.rfind(')')?],
    )
}

pub(crate) fn extract_rust_apply_template_name(raw_attributes: &[String]) -> Option<String> {
    raw_attributes.iter().find_map(|attribute| {
        (rust_attribute_name(attribute).as_deref() == Some("apply")).then(|| {
            extract_rust_attribute_args(attribute)
                .map(split_top_level_arguments)
                .and_then(|parts| parts.into_iter().next())
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_string)
        })?
    })
}

pub(crate) fn split_top_level_arguments(raw: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0i32;
    let mut brace_depth = 0i32;
    let mut bracket_depth = 0i32;
    let mut in_string: Option<char> = None;
    let mut escaped = false;

    for (idx, ch) in raw.char_indices() {
        if let Some(quote) = in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                in_string = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => in_string = Some(ch),
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            '{' => brace_depth += 1,
            '}' => brace_depth -= 1,
            '[' => bracket_depth += 1,
            ']' => bracket_depth -= 1,
            ',' if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 => {
                let part = raw[start..idx].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = idx + 1;
            }
            _ => {}
        }
    }

    let tail = raw[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

pub(crate) fn display_rstest_argument(value: &str) -> String {
    value.trim().to_string()
}

pub(crate) fn summarize_rstest_values(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| display_rstest_argument(value))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn extract_leading_rust_attributes(parameter: &str) -> Vec<&str> {
    let mut attributes = Vec::new();
    let mut offset = 0usize;
    let bytes = parameter.as_bytes();

    while offset < parameter.len() {
        while offset < parameter.len() && bytes[offset].is_ascii_whitespace() {
            offset += 1;
        }
        if offset + 2 > parameter.len() || &parameter[offset..offset + 2] != "#[" {
            break;
        }

        let end = find_matching_delimiter(parameter, offset + 1)
            .map(|idx| idx + 1)
            .unwrap_or(parameter.len());
        attributes.push(parameter[offset..end].trim());
        offset = end;
    }

    attributes
}

pub(crate) fn extract_rust_parameter_name(parameter: &str) -> Option<String> {
    let without_attributes = {
        let mut offset = 0usize;
        let bytes = parameter.as_bytes();
        while offset < parameter.len() {
            while offset < parameter.len() && bytes[offset].is_ascii_whitespace() {
                offset += 1;
            }
            if offset + 2 > parameter.len() || &parameter[offset..offset + 2] != "#[" {
                break;
            }
            let end = find_matching_delimiter(parameter, offset + 1)? + 1;
            offset = end;
        }
        parameter[offset..].trim()
    };

    let name = without_attributes
        .split(':')
        .next()
        .unwrap_or(without_attributes)
        .trim()
        .trim_start_matches("mut ")
        .trim_start_matches('&')
        .trim();

    let name = name
        .split_whitespace()
        .last()
        .unwrap_or(name)
        .trim()
        .trim_matches('_');

    (!name.is_empty()).then_some(name.to_string())
}

pub(crate) fn rust_attribute_is_test(attribute_node: Node<'_>, source: &[u8]) -> bool {
    let Ok(raw) = attribute_node.utf8_text(source) else {
        return false;
    };

    let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.starts_with("#[cfg(") {
        return false;
    }

    compact == "#[test]"
        || compact.starts_with("#[test(")
        || compact.contains("::test]")
        || compact.contains("::test(")
        || compact == "#[rstest]"
        || compact.starts_with("#[rstest(")
        || compact.contains("::rstest]")
        || compact.contains("::rstest(")
        || compact == "#[wasm_bindgen_test]"
        || compact.starts_with("#[wasm_bindgen_test(")
        || compact.contains("::wasm_bindgen_test]")
        || compact.contains("::wasm_bindgen_test(")
        || compact == "#[quickcheck]"
        || compact.starts_with("#[quickcheck(")
        || compact.contains("::quickcheck]")
        || compact.contains("::quickcheck(")
}

pub(crate) fn rust_attribute_is_parameterized_test(
    attribute_node: Node<'_>,
    source: &[u8],
) -> bool {
    let Ok(raw) = attribute_node.utf8_text(source) else {
        return false;
    };

    let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    compact.starts_with("#[test_case(")
        || compact.contains("::test_case(")
        || compact.starts_with("#[case(")
        || compact.contains("::case(")
}

pub(crate) fn rust_function_attributes(function_node: Node<'_>) -> Vec<Node<'_>> {
    let mut attributes = Vec::new();
    let mut seen_ranges = HashSet::new();

    let mut cursor = function_node.walk();
    for child in function_node.children(&mut cursor) {
        if child.kind() != "attribute_item" {
            continue;
        }

        let key = (child.start_byte(), child.end_byte());
        if seen_ranges.insert(key) {
            attributes.push(child);
        }
    }

    let mut sibling = function_node.prev_named_sibling();
    while let Some(node) = sibling {
        if node.kind() != "attribute_item" {
            break;
        }

        let key = (node.start_byte(), node.end_byte());
        if seen_ranges.insert(key) {
            attributes.push(node);
        }

        sibling = node.prev_named_sibling();
    }

    attributes.sort_by_key(Node::start_byte);
    attributes
}

pub(crate) fn build_rust_parameterized_test_case(
    function_name: &str,
    attribute_node: Node<'_>,
    source: &[u8],
    function_end_line: i64,
) -> RustScenarioSeed {
    let raw = attribute_node.utf8_text(source).unwrap_or_default();
    let rule_variant = extract_rule_variant_from_rust_test_case(raw);
    let fixture_path = extract_fixture_path_from_rust_test_case(raw);

    let mut name_parts = Vec::new();
    if let Some(rule_variant) = rule_variant.as_deref() {
        name_parts.push(rule_variant.to_string());
    }
    if let Some(fixture_path) = fixture_path.as_deref() {
        name_parts.push(fixture_path.to_string());
    }

    let name = if name_parts.is_empty() {
        function_name.to_string()
    } else {
        format!("{function_name}[{}]", name_parts.join(", "))
    };

    let mut reference_candidates = Vec::new();
    if let Some(rule_variant) = rule_variant {
        reference_candidates.push(ReferenceCandidate::ScopedSymbol(rule_variant));
    }

    RustScenarioSeed {
        name,
        start_line: attribute_node.start_position().row as i64 + 1,
        end_line: function_end_line,
        reference_candidates,
        discovery_source: ScenarioDiscoverySource::Source,
    }
}

pub(crate) fn find_matching_delimiter(raw: &str, open_index: usize) -> Option<usize> {
    let open_delimiter = raw[open_index..].chars().next()?;
    let close_delimiter = match open_delimiter {
        '(' => ')',
        '{' => '}',
        '[' => ']',
        _ => return None,
    };

    let mut depth = 0i32;
    for (idx, ch) in raw.char_indices().skip_while(|(idx, _)| *idx < open_index) {
        if ch == open_delimiter {
            depth += 1;
        } else if ch == close_delimiter {
            depth -= 1;
            if depth == 0 {
                return Some(idx);
            }
        }
    }

    None
}

fn extract_rule_variant_from_rust_test_case(raw_attribute: &str) -> Option<String> {
    rust_scoped_tokens(raw_attribute).into_iter().find_map(|token| {
        token
            .strip_prefix("Rule::")
            .and_then(|value| value.rsplit("::").next())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn extract_fixture_path_from_rust_test_case(raw_attribute: &str) -> Option<String> {
    extract_string_literals(raw_attribute).into_iter().find(|literal| {
        literal.ends_with(".py") || literal.ends_with(".pyi") || literal.ends_with(".ipynb")
    })
}

fn rust_scoped_tokens(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == ':' {
            current.push(ch);
            continue;
        }

        if current.contains("::") {
            tokens.push(current.clone());
        }
        current.clear();
    }

    if current.contains("::") {
        tokens.push(current);
    }

    tokens
}

fn extract_string_literals(raw: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '"' && ch != '\'' {
            continue;
        }

        let quote = ch;
        let mut literal = String::new();
        let mut escaped = false;

        for next in chars.by_ref() {
            if escaped {
                literal.push(next);
                escaped = false;
                continue;
            }

            if next == '\\' {
                escaped = true;
                continue;
            }

            if next == quote {
                literals.push(literal);
                break;
            }

            literal.push(next);
        }
    }

    literals
}
