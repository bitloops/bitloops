use std::collections::HashSet;

use anyhow::{Context, Result};

use super::canonical::{PYTHON_CANONICAL_MAPPINGS, PYTHON_SUPPORTED_LANGUAGE_KINDS};
use crate::host::language_adapter::{
    LanguageArtefact, is_supported_language_kind, resolve_canonical_kind,
};

pub(crate) fn extract_python_artefacts(content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter python language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String, i32)> = HashSet::new();
    collect_python_nodes_recursive(root, content, path, &mut out, &mut seen, &[]);
    out.sort_by_key(|i| {
        (
            i.start_line,
            i.end_line,
            i.canonical_kind.clone(),
            i.name.clone(),
        )
    });
    Ok(out)
}

pub(crate) fn extract_python_file_docstring(content: &str) -> Option<String> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    parser.set_language(&lang).ok()?;
    let tree = parser.parse(content, None)?;
    extract_docstring_from_body(tree.root_node(), content)
}

fn collect_python_nodes_recursive(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(String, String, i32)>,
    pending_modifiers: &[String],
) {
    match node.kind() {
        "decorated_definition" => {
            if let Some(definition) = node.child_by_field_name("definition") {
                let mut modifiers = extract_python_decorators(node, content);
                modifiers.extend_from_slice(pending_modifiers);
                collect_python_nodes_recursive(definition, content, path, out, seen, &modifiers);
            }
            return;
        }
        "class_definition" => {
            push_python_class_artefact(node, content, path, out, seen, pending_modifiers)
        }
        "function_definition" => {
            push_python_function_artefact(node, content, path, out, seen, pending_modifiers)
        }
        "import_statement" | "import_from_statement" | "future_import_statement" => {
            push_python_import_artefact(node, content, path, out, seen)
        }
        "assignment" => push_python_assignment_artefacts(node, content, path, out, seen),
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_python_nodes_recursive(child, content, path, out, seen, &[]);
    }
}

fn push_python_class_artefact(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(String, String, i32)>,
    pending_modifiers: &[String],
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Ok(name) = name_node.utf8_text(content.as_bytes()) else {
        return;
    };
    let name = name.trim();
    if name.is_empty() {
        return;
    }

    let parent_symbol_fqn = nearest_enclosing_class_fqn(node, content, path);
    let symbol_fqn = if let Some(parent) = parent_symbol_fqn.as_deref() {
        format!("{parent}::{name}")
    } else {
        format!("{path}::{name}")
    };
    push_python_artefact(
        out,
        seen,
        node,
        content,
        "class_definition",
        name.to_string(),
        symbol_fqn,
        parent_symbol_fqn,
        false,
        merge_python_modifiers(node, content, pending_modifiers),
        node.child_by_field_name("body")
            .and_then(|body| extract_docstring_from_body(body, content)),
    );
}

fn push_python_function_artefact(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(String, String, i32)>,
    pending_modifiers: &[String],
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Ok(name) = name_node.utf8_text(content.as_bytes()) else {
        return;
    };
    let name = name.trim();
    if name.is_empty() {
        return;
    }

    let Some(function_owner) = python_function_owner(node, content, path) else {
        return;
    };
    let (symbol_fqn, parent_symbol_fqn, inside_class) = match function_owner {
        PythonFunctionOwner::TopLevel => (format!("{path}::{name}"), None, false),
        PythonFunctionOwner::Class(class_fqn) => {
            (format!("{class_fqn}::{name}"), Some(class_fqn), true)
        }
        PythonFunctionOwner::NestedFunction => return,
    };

    push_python_artefact(
        out,
        seen,
        node,
        content,
        "function_definition",
        name.to_string(),
        symbol_fqn,
        parent_symbol_fqn,
        inside_class,
        merge_python_modifiers(node, content, pending_modifiers),
        node.child_by_field_name("body")
            .and_then(|body| extract_docstring_from_body(body, content)),
    );
}

fn push_python_import_artefact(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(String, String, i32)>,
) {
    let start_line = node.start_position().row as i32 + 1;
    let name = format!("import@{start_line}");
    push_python_artefact(
        out,
        seen,
        node,
        content,
        node.kind(),
        name.clone(),
        format!("{path}::import::{name}"),
        None,
        false,
        Vec::new(),
        None,
    );
}

fn push_python_assignment_artefacts(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(String, String, i32)>,
) {
    if !is_python_module_scope(node) {
        return;
    }

    let Some(left) = node.child_by_field_name("left") else {
        return;
    };
    if left.kind() == "identifier" {
        if let Ok(name) = left.utf8_text(content.as_bytes()) {
            let name = name.trim();
            if !name.is_empty() {
                push_python_artefact(
                    out,
                    seen,
                    node,
                    content,
                    "assignment",
                    name.to_string(),
                    format!("{path}::{name}"),
                    None,
                    false,
                    Vec::new(),
                    None,
                );
            }
        }
        return;
    }
    let mut cursor = left.walk();
    for child in left.named_children(&mut cursor) {
        if child.kind() != "identifier" {
            continue;
        }
        let Ok(name) = child.utf8_text(content.as_bytes()) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        push_python_artefact(
            out,
            seen,
            node,
            content,
            "assignment",
            name.to_string(),
            format!("{path}::{name}"),
            None,
            false,
            Vec::new(),
            None,
        );
    }
}

fn push_python_artefact(
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(String, String, i32)>,
    node: tree_sitter::Node,
    content: &str,
    language_kind: &str,
    name: String,
    symbol_fqn: String,
    parent_symbol_fqn: Option<String>,
    inside_parent: bool,
    modifiers: Vec<String>,
    docstring: Option<String>,
) {
    if name.is_empty()
        || !is_supported_language_kind(PYTHON_SUPPORTED_LANGUAGE_KINDS, language_kind)
    {
        return;
    }

    let start_line = node.start_position().row as i32 + 1;
    if !seen.insert((language_kind.to_string(), name.clone(), start_line)) {
        return;
    }

    let signature = node
        .utf8_text(content.as_bytes())
        .ok()
        .and_then(|text| text.lines().next())
        .unwrap_or("")
        .trim()
        .to_string();

    out.push(LanguageArtefact {
        canonical_kind: resolve_canonical_kind(
            PYTHON_CANONICAL_MAPPINGS,
            language_kind,
            inside_parent,
        )
        .map(|projection| projection.as_str().to_string()),
        language_kind: language_kind.to_string(),
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

fn python_function_owner(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
) -> Option<PythonFunctionOwner> {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_definition" => {
                return Some(PythonFunctionOwner::Class(python_class_symbol_fqn(
                    parent, content, path,
                )));
            }
            "function_definition" => return Some(PythonFunctionOwner::NestedFunction),
            "module" => return Some(PythonFunctionOwner::TopLevel),
            _ => current = parent.parent(),
        }
    }
    None
}

fn nearest_enclosing_class_fqn(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "class_definition" => return Some(python_class_symbol_fqn(parent, content, path)),
            "function_definition" | "module" => return None,
            _ => current = parent.parent(),
        }
    }
    None
}

fn python_class_symbol_fqn(node: tree_sitter::Node, content: &str, path: &str) -> String {
    let Some(name_node) = node.child_by_field_name("name") else {
        return path.to_string();
    };
    let Ok(name) = name_node.utf8_text(content.as_bytes()) else {
        return path.to_string();
    };
    let name = name.trim();
    if let Some(parent) = nearest_enclosing_class_fqn(node, content, path) {
        format!("{parent}::{name}")
    } else {
        format!("{path}::{name}")
    }
}

fn merge_python_modifiers(
    node: tree_sitter::Node,
    content: &str,
    pending_modifiers: &[String],
) -> Vec<String> {
    let mut modifiers = Vec::new();
    if let Ok(signature) = node.utf8_text(content.as_bytes())
        && signature.trim_start().starts_with("async ")
    {
        modifiers.push("async".to_string());
    }
    for modifier in pending_modifiers {
        if !modifiers.contains(modifier) {
            modifiers.push(modifier.clone());
        }
    }
    modifiers
}

fn extract_python_decorators(node: tree_sitter::Node, content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "decorator" {
            continue;
        }
        let Some(expression) = child.named_child(0) else {
            continue;
        };
        let Ok(value) = expression.utf8_text(content.as_bytes()) else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        out.push(value.to_string());
    }
    out
}

fn is_python_module_scope(node: tree_sitter::Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "module" => return true,
            "function_definition" | "class_definition" => return false,
            _ => current = parent.parent(),
        }
    }
    false
}

pub(crate) fn extract_docstring_from_body(
    node: tree_sitter::Node,
    content: &str,
) -> Option<String> {
    let mut cursor = node.walk();
    let first_child = node.named_children(&mut cursor).next()?;
    expression_statement_string_value(first_child, content)
}

fn expression_statement_string_value(node: tree_sitter::Node, content: &str) -> Option<String> {
    if node.kind() != "expression_statement" {
        return None;
    }
    let mut cursor = node.walk();
    let expression = node.named_children(&mut cursor).next()?;
    match expression.kind() {
        "string" | "concatenated_string" => {
            let text = expression.utf8_text(content.as_bytes()).ok()?;
            normalize_python_string_literal(text)
        }
        _ => None,
    }
}

fn normalize_python_string_literal(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    let prefixes = ["r", "u", "b", "f", "rb", "br", "fr", "rf"];
    let mut body = trimmed;
    for prefix in prefixes {
        if !lower.starts_with(prefix) {
            continue;
        }
        let suffix = &trimmed[prefix.len()..];
        if suffix.starts_with("\"\"\"")
            || suffix.starts_with("'''")
            || suffix.starts_with('"')
            || suffix.starts_with('\'')
        {
            body = &trimmed[prefix.len()..];
            break;
        }
    }

    if body.starts_with("\"\"\"") && body.ends_with("\"\"\"") && body.len() >= 6 {
        return Some(body[3..body.len() - 3].to_string());
    }
    if body.starts_with("'''") && body.ends_with("'''") && body.len() >= 6 {
        return Some(body[3..body.len() - 3].to_string());
    }
    if body.starts_with('"') && body.ends_with('"') && body.len() >= 2 {
        return Some(body[1..body.len() - 1].to_string());
    }
    if body.starts_with('\'') && body.ends_with('\'') && body.len() >= 2 {
        return Some(body[1..body.len() - 1].to_string());
    }

    None
}

enum PythonFunctionOwner {
    TopLevel,
    Class(String),
    NestedFunction,
}
