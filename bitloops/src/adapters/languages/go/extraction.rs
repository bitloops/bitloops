use std::collections::HashSet;

use anyhow::{Context, Result};
use tree_sitter::Node;

use super::canonical::{GO_CANONICAL_MAPPINGS, GO_SUPPORTED_LANGUAGE_KINDS};
use crate::host::language_adapter::{
    GoKind, LanguageArtefact, LanguageKind, is_supported_language_kind, resolve_canonical_kind,
};

struct GoArtefactDescriptor {
    language_kind: LanguageKind,
    name: String,
    symbol_fqn: String,
    parent_symbol_fqn: Option<String>,
    modifiers: Vec<String>,
}

pub(crate) fn extract_go_artefacts(content: &str, path: &str) -> Result<Vec<LanguageArtefact>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter go language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let mut out = Vec::new();
    let mut seen: HashSet<(LanguageKind, String, i32)> = HashSet::new();
    collect_go_nodes_recursive(root, content, path, &mut out, &mut seen);
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

pub(crate) fn package_name_from_root(root: Node<'_>, content: &str) -> Option<String> {
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .find(|child| child.kind() == "package_clause")
        .and_then(|node| node.named_child(0))
        .and_then(|node| trimmed_node_text(node, content))
}

pub(crate) fn trimmed_node_text(node: Node<'_>, content: &str) -> Option<String> {
    node.utf8_text(content.as_bytes())
        .ok()
        .map(str::trim)
        .map(str::to_string)
        .filter(|text| !text.is_empty())
}

pub(crate) fn type_name_from_node(node: Node<'_>, content: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" | "identifier" | "field_identifier" | "package_identifier" => {
            trimmed_node_text(node, content)
        }
        "qualified_type" => node
            .child_by_field_name("name")
            .and_then(|name| trimmed_node_text(name, content)),
        "generic_type" | "type_instantiation_expression" => node
            .child_by_field_name("type")
            .and_then(|inner| type_name_from_node(inner, content)),
        "pointer_type" | "slice_type" | "parenthesized_type" | "negated_type" => node
            .named_child(0)
            .and_then(|inner| type_name_from_node(inner, content)),
        "array_type" => node
            .child_by_field_name("element")
            .and_then(|inner| type_name_from_node(inner, content)),
        "map_type" | "channel_type" => node
            .child_by_field_name("value")
            .and_then(|inner| type_name_from_node(inner, content)),
        "parameter_declaration" | "variadic_parameter_declaration" => node
            .child_by_field_name("type")
            .and_then(|inner| type_name_from_node(inner, content)),
        _ => trimmed_node_text(node, content)
            .map(|text| {
                text.trim_start_matches('*')
                    .split(['[', ']', '{', '}', '(', ')', ',', ' '])
                    .find(|part| !part.is_empty())
                    .unwrap_or("")
                    .to_string()
            })
            .filter(|text| !text.is_empty()),
    }
}

fn collect_go_nodes_recursive(
    node: Node<'_>,
    content: &str,
    path: &str,
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(LanguageKind, String, i32)>,
) {
    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Some(name) = trimmed_node_text(name_node, content)
            {
                push_go_artefact(
                    out,
                    seen,
                    node,
                    content,
                    GoArtefactDescriptor {
                        language_kind: LanguageKind::go(GoKind::FunctionDeclaration),
                        name: name.clone(),
                        symbol_fqn: format!("{path}::{name}"),
                        parent_symbol_fqn: None,
                        modifiers: function_modifiers(node, content),
                    },
                );
            }
        }
        "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Some(name) = trimmed_node_text(name_node, content)
                && let Some(receiver) = node.child_by_field_name("receiver")
                && let Some(receiver_type_name) = receiver_type_name(receiver, content)
            {
                let parent_symbol_fqn = format!("{path}::{receiver_type_name}");
                push_go_artefact(
                    out,
                    seen,
                    node,
                    content,
                    GoArtefactDescriptor {
                        language_kind: LanguageKind::go(GoKind::MethodDeclaration),
                        name: name.clone(),
                        symbol_fqn: format!("{parent_symbol_fqn}::{name}"),
                        parent_symbol_fqn: Some(parent_symbol_fqn),
                        modifiers: function_modifiers(node, content),
                    },
                );
            }
        }
        "type_spec" | "type_alias" => {
            if let Some(type_name_node) = node.child_by_field_name("name")
                && let Some(name) = trimmed_node_text(type_name_node, content)
            {
                let type_node = node.child_by_field_name("type");
                let language_kind = type_node
                    .and_then(|inner| match inner.kind() {
                        "struct_type" => Some(LanguageKind::go(GoKind::StructType)),
                        "interface_type" => Some(LanguageKind::go(GoKind::InterfaceType)),
                        _ => None,
                    })
                    .unwrap_or_else(|| {
                        LanguageKind::go(
                            GoKind::from_tree_sitter_kind(node.kind())
                                .expect("validated go type node kind"),
                        )
                    });
                let modifiers = type_modifiers(node, type_node, content);
                push_go_artefact(
                    out,
                    seen,
                    node,
                    content,
                    GoArtefactDescriptor {
                        language_kind,
                        name: name.clone(),
                        symbol_fqn: format!("{path}::{name}"),
                        parent_symbol_fqn: None,
                        modifiers,
                    },
                );
            }
        }
        "import_spec" => {
            if let Some(path_node) = node.child_by_field_name("path")
                && let Some(import_path) = trimmed_node_text(path_node, content)
            {
                let import_name = node
                    .child_by_field_name("name")
                    .and_then(|name_node| trimmed_node_text(name_node, content))
                    .unwrap_or_else(|| import_path_stem(&import_path));
                let line_no = node.start_position().row as i32 + 1;
                push_go_artefact(
                    out,
                    seen,
                    node,
                    content,
                    GoArtefactDescriptor {
                        language_kind: LanguageKind::go(GoKind::ImportSpec),
                        name: import_name.clone(),
                        symbol_fqn: format!("{path}::import::{import_name}@{line_no}"),
                        parent_symbol_fqn: None,
                        modifiers: vec![strip_string_literal_delimiters(&import_path)],
                    },
                );
            }
        }
        "const_spec" | "var_spec" if is_module_scope(node) => {
            let language_kind = LanguageKind::go(
                GoKind::from_tree_sitter_kind(node.kind()).expect("validated go value kind"),
            );
            let mut cursor = node.walk();
            for name_node in node.children_by_field_name("name", &mut cursor) {
                if let Some(name) = trimmed_node_text(name_node, content) {
                    push_go_artefact(
                        out,
                        seen,
                        node,
                        content,
                        GoArtefactDescriptor {
                            language_kind,
                            name: name.clone(),
                            symbol_fqn: format!("{path}::{name}"),
                            parent_symbol_fqn: None,
                            modifiers: Vec::new(),
                        },
                    );
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_go_nodes_recursive(child, content, path, out, seen);
    }
}

fn push_go_artefact(
    out: &mut Vec<LanguageArtefact>,
    seen: &mut HashSet<(LanguageKind, String, i32)>,
    node: Node<'_>,
    content: &str,
    descriptor: GoArtefactDescriptor,
) {
    let GoArtefactDescriptor {
        language_kind,
        name,
        symbol_fqn,
        parent_symbol_fqn,
        modifiers,
    } = descriptor;

    if name.is_empty() || !is_supported_language_kind(GO_SUPPORTED_LANGUAGE_KINDS, language_kind) {
        return;
    }

    let start_line = node.start_position().row as i32 + 1;
    if !seen.insert((language_kind, name.clone(), start_line)) {
        return;
    }

    let signature = trimmed_node_text(node, content)
        .and_then(|text| text.lines().next().map(str::trim).map(str::to_string))
        .unwrap_or_default();

    out.push(LanguageArtefact {
        canonical_kind: resolve_canonical_kind(GO_CANONICAL_MAPPINGS, language_kind, false)
            .map(|kind| kind.as_str().to_string()),
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
        docstring: None,
    });
}

fn is_module_scope(node: Node<'_>) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(
            parent.kind(),
            "function_declaration" | "method_declaration" | "func_literal"
        ) {
            return false;
        }
        current = parent.parent();
    }
    true
}

fn receiver_type_name(receiver: Node<'_>, content: &str) -> Option<String> {
    let receiver_decl = receiver.named_child(0)?;
    let receiver_type = receiver_decl.child_by_field_name("type")?;
    type_name_from_node(receiver_type, content)
}

fn function_modifiers(node: Node<'_>, content: &str) -> Vec<String> {
    let mut modifiers = Vec::new();
    if node.child_by_field_name("type_parameters").is_some() {
        modifiers.push("generic".to_string());
    }
    if node.child_by_field_name("result").is_some()
        && let Some(result) = node.child_by_field_name("result")
        && let Some(result_text) = trimmed_node_text(result, content)
        && !result_text.is_empty()
    {
        modifiers.push("returns".to_string());
    }
    modifiers
}

fn type_modifiers(node: Node<'_>, type_node: Option<Node<'_>>, content: &str) -> Vec<String> {
    let mut modifiers = Vec::new();
    if node.kind() == "type_alias" {
        modifiers.push("alias".to_string());
    }
    if let Some(inner) = type_node {
        match inner.kind() {
            "struct_type" => modifiers.push("struct".to_string()),
            "interface_type" => modifiers.push("interface".to_string()),
            _ => {
                if let Some(name) = type_name_from_node(inner, content) {
                    modifiers.push(name);
                }
            }
        }
    }
    if node.child_by_field_name("type_parameters").is_some() {
        modifiers.push("generic".to_string());
    }
    modifiers
}

pub(crate) fn strip_string_literal_delimiters(text: &str) -> String {
    text.trim().trim_matches(['"', '`']).to_string()
}

pub(crate) fn import_path_stem(import_path: &str) -> String {
    let trimmed = strip_string_literal_delimiters(import_path);
    trimmed
        .rsplit('/')
        .next()
        .unwrap_or(trimmed.as_str())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::extract_go_artefacts;
    use crate::host::language_adapter::{GoKind, LanguageKind};

    #[test]
    fn extract_go_artefacts_collects_functions_methods_types_imports_and_values() {
        let content = r#"package service

import (
    "context"
    alias "net/http"
)

const DefaultPort = 8080
var DefaultHost = "localhost"

type Handler struct{}
type Runner interface {
    Run(context.Context) error
}

func NewHandler() *Handler {
    return &Handler{}
}

func (h *Handler) ServeHTTP() {}
"#;

        let artefacts = extract_go_artefacts(content, "service/handler.go").unwrap();

        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::go(GoKind::FunctionDeclaration)
                && artefact.name == "NewHandler"
                && artefact.canonical_kind.as_deref() == Some("function")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::go(GoKind::MethodDeclaration)
                && artefact.name == "ServeHTTP"
                && artefact.parent_symbol_fqn.as_deref() == Some("service/handler.go::Handler")
                && artefact.canonical_kind.as_deref() == Some("method")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::go(GoKind::StructType)
                && artefact.name == "Handler"
                && artefact.canonical_kind.as_deref() == Some("type")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::go(GoKind::InterfaceType)
                && artefact.name == "Runner"
                && artefact.canonical_kind.as_deref() == Some("interface")
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::go(GoKind::ImportSpec)
                && artefact.name == "alias"
        }));
        assert!(artefacts.iter().any(|artefact| {
            artefact.language_kind == LanguageKind::go(GoKind::ConstSpec)
                && artefact.name == "DefaultPort"
        }));
    }
}
