use std::collections::HashSet;

use anyhow::{Context, Result};
use tree_sitter::Node;

use super::canonical::{JAVA_CANONICAL_MAPPINGS, JAVA_SUPPORTED_LANGUAGE_KINDS};
use crate::host::language_adapter::{
    JavaKind, LanguageArtefact, LanguageKind, is_supported_language_kind,
    normalize_artefact_signature, resolve_canonical_kind,
};

struct JavaArtefactDescriptor {
    language_kind: LanguageKind,
    name: String,
    symbol_fqn: String,
    parent_symbol_fqn: Option<String>,
    signature: String,
    modifiers: Vec<String>,
    docstring: Option<String>,
}

pub(crate) fn extract_java_artefacts(content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter java language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let mut out = Vec::new();
    let mut seen: HashSet<(LanguageKind, String, i32)> = HashSet::new();
    collect_java_nodes_recursive(root, content, path, &mut out, &mut seen, None);
    out.sort_by_key(|artefact| {
        (
            artefact.start_line,
            artefact.end_line,
            artefact.canonical_kind.clone(),
            artefact.name.clone(),
        )
    });
    Ok(out)
}

pub(crate) fn extract_java_file_docstring(content: &str) -> Option<String> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
    if parser.set_language(&lang).is_ok()
        && let Some(tree) = parser.parse(content, None)
    {
        let root = tree.root_node();
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            if matches!(
                child.kind(),
                "package_declaration"
                    | "class_declaration"
                    | "interface_declaration"
                    | "enum_declaration"
            ) && let Some(docstring) = extract_java_docstring(child, content)
            {
                return Some(docstring);
            }
        }
    }

    let lines: Vec<&str> = content.lines().collect();
    let mut idx = 0usize;
    while idx < lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed.is_empty() {
            idx += 1;
            continue;
        }
        if trimmed.starts_with("/**") {
            let start = idx;
            idx += 1;
            while idx < lines.len() && !lines[idx - 1].contains("*/") {
                idx += 1;
            }
            return Some(normalize_java_doc_block(&lines[start..idx]))
                .filter(|value| !value.is_empty());
        }
        break;
    }
    None
}

fn collect_java_nodes_recursive(
    node: Node<'_>,
    content: &str,
    path: &str,
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(LanguageKind, String, i32)>,
    current_type_fqn: Option<String>,
) {
    let mut next_type_fqn = current_type_fqn.clone();

    match node.kind() {
        "package_declaration" => {
            if let Some(name) = first_name_like_child_text(node, content) {
                push_java_artefact(
                    out,
                    seen,
                    node,
                    JavaArtefactDescriptor {
                        language_kind: LanguageKind::java(JavaKind::Package),
                        name: name.clone(),
                        symbol_fqn: format!("{path}::{name}"),
                        parent_symbol_fqn: None,
                        signature: node_signature(node, content),
                        modifiers: extract_java_modifiers(node, content),
                        docstring: extract_java_docstring(node, content),
                    },
                );
            }
        }
        "import_declaration" => {
            let start_line = node.start_position().row as i32 + 1;
            let name = format!("import@{start_line}");
            let mut modifiers = extract_java_modifiers(node, content);
            if let Some(import_ref) = import_reference(node, content) {
                modifiers.push(import_ref);
            }
            push_java_artefact(
                out,
                seen,
                node,
                JavaArtefactDescriptor {
                    language_kind: LanguageKind::java(JavaKind::Import),
                    name: name.clone(),
                    symbol_fqn: format!("{path}::import::{name}"),
                    parent_symbol_fqn: None,
                    signature: node_signature(node, content),
                    modifiers,
                    docstring: extract_java_docstring(node, content),
                },
            );
        }
        "class_declaration" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|name_node| trimmed_node_text(name_node, content))
            {
                let symbol_fqn = current_type_fqn
                    .as_ref()
                    .map(|parent| format!("{parent}::{name}"))
                    .unwrap_or_else(|| format!("{path}::{name}"));
                push_java_artefact(
                    out,
                    seen,
                    node,
                    JavaArtefactDescriptor {
                        language_kind: LanguageKind::java(JavaKind::Class),
                        name: name.clone(),
                        symbol_fqn: symbol_fqn.clone(),
                        parent_symbol_fqn: current_type_fqn.clone(),
                        signature: node_signature(node, content),
                        modifiers: extract_java_modifiers(node, content),
                        docstring: extract_java_docstring(node, content),
                    },
                );
                next_type_fqn = Some(symbol_fqn);
            }
        }
        "interface_declaration" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|name_node| trimmed_node_text(name_node, content))
            {
                let symbol_fqn = current_type_fqn
                    .as_ref()
                    .map(|parent| format!("{parent}::{name}"))
                    .unwrap_or_else(|| format!("{path}::{name}"));
                push_java_artefact(
                    out,
                    seen,
                    node,
                    JavaArtefactDescriptor {
                        language_kind: LanguageKind::java(JavaKind::Interface),
                        name: name.clone(),
                        symbol_fqn: symbol_fqn.clone(),
                        parent_symbol_fqn: current_type_fqn.clone(),
                        signature: node_signature(node, content),
                        modifiers: extract_java_modifiers(node, content),
                        docstring: extract_java_docstring(node, content),
                    },
                );
                next_type_fqn = Some(symbol_fqn);
            }
        }
        "enum_declaration" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|name_node| trimmed_node_text(name_node, content))
            {
                let symbol_fqn = current_type_fqn
                    .as_ref()
                    .map(|parent| format!("{parent}::{name}"))
                    .unwrap_or_else(|| format!("{path}::{name}"));
                push_java_artefact(
                    out,
                    seen,
                    node,
                    JavaArtefactDescriptor {
                        language_kind: LanguageKind::java(JavaKind::Enum),
                        name: name.clone(),
                        symbol_fqn: symbol_fqn.clone(),
                        parent_symbol_fqn: current_type_fqn.clone(),
                        signature: node_signature(node, content),
                        modifiers: extract_java_modifiers(node, content),
                        docstring: extract_java_docstring(node, content),
                    },
                );
                next_type_fqn = Some(symbol_fqn);
            }
        }
        "constructor_declaration" => {
            let Some(parent_symbol_fqn) = current_type_fqn.clone() else {
                return;
            };
            push_java_artefact(
                out,
                seen,
                node,
                JavaArtefactDescriptor {
                    language_kind: LanguageKind::java(JavaKind::Constructor),
                    name: "<init>".to_string(),
                    symbol_fqn: format!("{parent_symbol_fqn}::<init>"),
                    parent_symbol_fqn: Some(parent_symbol_fqn),
                    signature: node_signature(node, content),
                    modifiers: extract_java_modifiers(node, content),
                    docstring: extract_java_docstring(node, content),
                },
            );
        }
        "method_declaration" => {
            let Some(parent_symbol_fqn) = current_type_fqn.clone() else {
                return;
            };
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|name_node| trimmed_node_text(name_node, content))
            {
                push_java_artefact(
                    out,
                    seen,
                    node,
                    JavaArtefactDescriptor {
                        language_kind: LanguageKind::java(JavaKind::Method),
                        name: name.clone(),
                        symbol_fqn: format!("{parent_symbol_fqn}::{name}"),
                        parent_symbol_fqn: Some(parent_symbol_fqn),
                        signature: node_signature(node, content),
                        modifiers: extract_java_modifiers(node, content),
                        docstring: extract_java_docstring(node, content),
                    },
                );
            }
        }
        "field_declaration" => {
            let Some(parent_symbol_fqn) = current_type_fqn.clone() else {
                return;
            };
            let signature = node_signature(node, content);
            let modifiers = extract_java_modifiers(node, content);
            let docstring = extract_java_docstring(node, content);
            let mut cursor = node.walk();
            for declarator in node.children_by_field_name("declarator", &mut cursor) {
                let Some(name_node) = declarator.child_by_field_name("name") else {
                    continue;
                };
                let Some(name) = trimmed_node_text(name_node, content) else {
                    continue;
                };
                push_java_artefact(
                    out,
                    seen,
                    declarator,
                    JavaArtefactDescriptor {
                        language_kind: LanguageKind::java(JavaKind::Field),
                        name: name.clone(),
                        symbol_fqn: format!("{parent_symbol_fqn}::{name}"),
                        parent_symbol_fqn: Some(parent_symbol_fqn.clone()),
                        signature: signature.clone(),
                        modifiers: modifiers.clone(),
                        docstring: docstring.clone(),
                    },
                );
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_java_nodes_recursive(child, content, path, out, seen, next_type_fqn.clone());
    }
}

fn push_java_artefact(
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(LanguageKind, String, i32)>,
    node: Node<'_>,
    descriptor: JavaArtefactDescriptor,
) {
    let JavaArtefactDescriptor {
        language_kind,
        name,
        symbol_fqn,
        parent_symbol_fqn,
        signature,
        modifiers,
        docstring,
    } = descriptor;

    if name.is_empty() || !is_supported_language_kind(JAVA_SUPPORTED_LANGUAGE_KINDS, language_kind)
    {
        return;
    }

    let start_line = node.start_position().row as i32 + 1;
    if !seen.insert((language_kind, name.clone(), start_line)) {
        return;
    }

    out.push(LanguageArtefact {
        canonical_kind: resolve_canonical_kind(JAVA_CANONICAL_MAPPINGS, language_kind, false)
            .map(|projection| projection.as_str().to_string()),
        language_kind,
        name,
        symbol_fqn,
        parent_symbol_fqn,
        start_line,
        end_line: node.end_position().row as i32 + 1,
        start_byte: node.start_byte() as i32,
        end_byte: node.end_byte() as i32,
        signature,
        modifiers,
        docstring,
    });
}

fn first_name_like_child_text(node: Node<'_>, content: &str) -> Option<String> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| matches!(child.kind(), "identifier" | "scoped_identifier"))
        .and_then(|child| trimmed_node_text(child, content))
}

pub(crate) fn trimmed_node_text(node: Node<'_>, content: &str) -> Option<String> {
    node.utf8_text(content.as_bytes())
        .ok()
        .map(str::trim)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

fn node_signature(node: Node<'_>, content: &str) -> String {
    normalize_artefact_signature(
        node.utf8_text(content.as_bytes())
            .ok()
            .and_then(|text| text.lines().next())
            .unwrap_or(""),
    )
}

fn import_reference(node: Node<'_>, content: &str) -> Option<String> {
    let raw = node.utf8_text(content.as_bytes()).ok()?.trim().to_string();
    let mut trimmed = raw.trim_end_matches(';').trim().to_string();
    if let Some(rest) = trimmed.strip_prefix("import") {
        trimmed = rest.trim().to_string();
    }
    Some(trimmed)
}

fn extract_java_modifiers(node: Node<'_>, content: &str) -> Vec<String> {
    let mut modifiers = Vec::new();
    let name_start = node
        .child_by_field_name("name")
        .map(|child| child.start_byte())
        .or_else(|| {
            node.child_by_field_name("type")
                .map(|child| child.start_byte())
        })
        .unwrap_or_else(|| node.end_byte());
    let prefix = content
        .get(node.start_byte()..name_start)
        .unwrap_or_default()
        .trim();

    for keyword in [
        "public",
        "protected",
        "private",
        "static",
        "final",
        "abstract",
        "native",
        "synchronized",
        "transient",
        "volatile",
        "sealed",
        "non-sealed",
        "strictfp",
    ] {
        if prefix.contains(keyword) {
            modifiers.push(keyword.to_string());
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if !matches!(
            child.kind(),
            "annotation" | "marker_annotation" | "modifiers"
        ) {
            continue;
        }
        if child.kind() == "modifiers" {
            let mut inner_cursor = child.walk();
            for inner in child.named_children(&mut inner_cursor) {
                if let Some(name_node) = inner.child_by_field_name("name")
                    && let Some(name) = trimmed_node_text(name_node, content)
                {
                    push_modifier(&mut modifiers, &name);
                }
            }
            continue;
        }
        if let Some(name_node) = child.child_by_field_name("name")
            && let Some(name) = trimmed_node_text(name_node, content)
        {
            push_modifier(&mut modifiers, &name);
        }
    }

    modifiers
}

fn push_modifier(modifiers: &mut Vec<String>, modifier: &str) {
    let normalized = modifier.trim().to_string();
    if normalized.is_empty() || modifiers.iter().any(|existing| existing == &normalized) {
        return;
    }
    modifiers.push(normalized);
}

fn extract_java_docstring(node: Node<'_>, content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let anchor_line = node.start_position().row as i32 + 1;
    if anchor_line <= 1 {
        return None;
    }

    let mut blocks = Vec::new();
    let mut line_idx = anchor_line - 2;
    while line_idx >= 0 {
        let trimmed = lines[line_idx as usize].trim();
        if trimmed.is_empty() {
            break;
        }
        if trimmed.contains("*/") || trimmed.starts_with('*') {
            let mut start = line_idx;
            while start >= 0 && !lines[start as usize].contains("/**") {
                start -= 1;
            }
            if start < 0 {
                break;
            }
            blocks.push(normalize_java_doc_block(
                &lines[start as usize..=line_idx as usize],
            ));
            line_idx = start - 1;
            continue;
        }
        break;
    }

    if blocks.is_empty() {
        None
    } else {
        blocks.reverse();
        Some(blocks.join("\n\n")).filter(|value| !value.is_empty())
    }
}

fn normalize_java_doc_block(lines: &[&str]) -> String {
    let mut normalized = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let mut text = line.trim().to_string();
        if index == 0 {
            text = text.trim_start_matches("/**").trim().to_string();
        }
        if index + 1 == lines.len() {
            text = text
                .trim_end_matches('/')
                .trim_end_matches('*')
                .trim()
                .to_string();
        }
        text = text.trim_start_matches('*').trim().to_string();
        normalized.push(text);
    }
    normalized.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{extract_java_artefacts, extract_java_file_docstring};
    use crate::host::language_adapter::{JavaKind, LanguageKind};

    #[test]
    fn extract_java_artefacts_collects_package_imports_types_methods_fields_and_constructors() {
        let content = r#"package com.acme;

import java.util.List;

class Base {}
interface Runner {}

/**
 * Greeter docs
 */
class Greeter extends Base implements Runner {
    private int count;

    Greeter() {}

    void greet(List<String> names) {}
}
"#;

        let artefacts = extract_java_artefacts(content, "src/com/acme/Greeter.java").unwrap();

        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::java(JavaKind::Package)
                && artefact.name == "com.acme"
                && artefact.canonical_kind.as_deref() == Some("module")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::java(JavaKind::Import)
                && artefact.canonical_kind.as_deref() == Some("import")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::java(JavaKind::Class)
                && artefact.name == "Greeter"
                && artefact.canonical_kind.as_deref() == Some("type")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::java(JavaKind::Constructor)
                && artefact.name == "<init>"
                && artefact.parent_symbol_fqn.as_deref()
                    == Some("src/com/acme/Greeter.java::Greeter")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::java(JavaKind::Method)
                && artefact.name == "greet"
                && artefact.canonical_kind.as_deref() == Some("method")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::java(JavaKind::Field)
                && artefact.name == "count"
                && artefact.canonical_kind.as_deref() == Some("variable")
        }));
    }

    #[test]
    fn extract_java_artefacts_tracks_nested_type_ownership() {
        let content = r#"package com.acme;

class Outer {
    class Inner {
        void run() {}
    }
}
"#;

        let artefacts = extract_java_artefacts(content, "src/com/acme/Outer.java").unwrap();

        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::java(JavaKind::Class)
                && artefact.name == "Inner"
                && artefact.parent_symbol_fqn.as_deref() == Some("src/com/acme/Outer.java::Outer")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::java(JavaKind::Method)
                && artefact.name == "run"
                && artefact.parent_symbol_fqn.as_deref()
                    == Some("src/com/acme/Outer.java::Outer::Inner")
        }));
    }

    #[test]
    fn extract_java_file_docstring_prefers_top_level_java_declaration_docs() {
        let content = r#"package com.acme;

import java.util.List;

/**
 * Greeter docs
 */
class Greeter {
    void greet(List<String> names) {}
}
"#;

        assert_eq!(
            extract_java_file_docstring(content).as_deref(),
            Some("Greeter docs")
        );
    }
}
