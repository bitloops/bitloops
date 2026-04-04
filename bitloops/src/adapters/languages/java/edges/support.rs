use std::collections::HashMap;

use tree_sitter::Node;

use super::super::extraction::trimmed_node_text;
use crate::host::language_adapter::LanguageArtefact;

pub(super) fn smallest_enclosing_callable(
    line_no: i32,
    callables: &[LanguageArtefact],
) -> Option<LanguageArtefact> {
    callables
        .iter()
        .filter(|artefact| artefact.start_line <= line_no && artefact.end_line >= line_no)
        .min_by_key(|artefact| artefact.end_line - artefact.start_line)
        .cloned()
}

pub(super) fn smallest_enclosing_symbol(
    line_no: i32,
    path: &str,
    type_targets: &HashMap<String, String>,
    callables: &[LanguageArtefact],
    field_targets_by_parent_and_name: &HashMap<(String, String), String>,
) -> Option<String> {
    if let Some(callable) = smallest_enclosing_callable(line_no, callables) {
        return Some(callable.symbol_fqn);
    }
    field_targets_by_parent_and_name
        .values()
        .find(|symbol_fqn| symbol_fqn.starts_with(path))
        .cloned()
        .or_else(|| {
            type_targets
                .values()
                .find(|symbol_fqn| symbol_fqn.starts_with(path))
                .cloned()
        })
        .or_else(|| Some(path.to_string()))
}

pub(super) fn java_type_name_from_node(node: Node<'_>, content: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" | "identifier" | "scoped_identifier" | "scoped_type_identifier" => {
            trimmed_node_text(node, content)
                .map(|text| text.rsplit('.').next().unwrap_or(text.as_str()).to_string())
        }
        "generic_type" => node
            .child_by_field_name("type")
            .and_then(|inner| java_type_name_from_node(inner, content))
            .or_else(|| {
                trimmed_node_text(node, content).and_then(|text| java_type_name_from_text(&text))
            }),
        "array_type" | "annotated_type" => node
            .child_by_field_name("element")
            .or_else(|| node.named_child(0))
            .and_then(|inner| java_type_name_from_node(inner, content)),
        _ => trimmed_node_text(node, content).and_then(|text| java_type_name_from_text(&text)),
    }
}

pub(super) fn java_type_name_from_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_generics = trimmed.split('<').next().unwrap_or(trimmed).trim();
    let without_arrays = without_generics.trim_end_matches("[]").trim();
    let candidate = without_arrays
        .rsplit('.')
        .next()
        .unwrap_or(without_arrays)
        .trim();
    let candidate = candidate.trim_matches(|ch: char| matches!(ch, '@' | '?' | '&' | ' '));
    let first = candidate.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    Some(candidate.to_string())
}
