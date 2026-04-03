use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::host::language_adapter::{DependencyEdge, JavaKind, LanguageArtefact, LanguageKind};

#[path = "edges/calls.rs"]
mod calls;
#[path = "edges/imports.rs"]
mod imports;
#[path = "edges/references.rs"]
mod references;
#[path = "edges/relationships.rs"]
mod relationships;
#[path = "edges/support.rs"]
mod support;
#[cfg(test)]
#[path = "edges/tests.rs"]
mod tests;

use calls::{
    collect_java_explicit_constructor_invocation, collect_java_method_invocation_edge,
    collect_java_object_creation_edge,
};
use imports::{collect_java_import_data, collect_java_import_edge};
use references::{
    collect_java_constructor_type_references, collect_java_field_type_references,
    collect_java_method_type_references,
};
use relationships::{
    collect_java_class_relationships, collect_java_enum_relationships,
    collect_java_interface_relationships,
};

pub(super) struct JavaTraversalCtx<'a> {
    pub(super) content: &'a str,
    pub(super) path: &'a str,
    pub(super) callables: &'a [LanguageArtefact],
    pub(super) type_targets: &'a HashMap<String, String>,
    pub(super) imported_type_refs: &'a HashMap<String, String>,
    pub(super) imported_static_refs: &'a HashMap<String, String>,
    pub(super) method_targets_by_parent_and_name: &'a HashMap<(String, String), String>,
    pub(super) field_targets_by_parent_and_name: &'a HashMap<(String, String), String>,
    pub(super) constructor_targets_by_type: &'a HashMap<String, String>,
    pub(super) out: &'a mut Vec<DependencyEdge>,
    pub(super) seen_imports: &'a mut HashSet<String>,
    pub(super) seen_calls: &'a mut HashSet<String>,
    pub(super) seen_refs: &'a mut HashSet<String>,
    pub(super) seen_extends: &'a mut HashSet<String>,
    pub(super) seen_implements: &'a mut HashSet<String>,
}

pub(crate) fn extract_java_dependency_edges(
    content: &str,
    path: &str,
    artefacts: &[LanguageArtefact],
) -> Result<Vec<DependencyEdge>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter java language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let callables = artefacts
        .iter()
        .filter(|artefact| {
            matches!(
                artefact.language_kind,
                LanguageKind::Java(JavaKind::Method) | LanguageKind::Java(JavaKind::Constructor)
            )
        })
        .cloned()
        .collect::<Vec<_>>();

    let mut type_targets = HashMap::new();
    let mut method_targets_by_parent_and_name = HashMap::new();
    let mut field_targets_by_parent_and_name = HashMap::new();
    let mut constructor_targets_by_type = HashMap::new();

    for artefact in artefacts {
        match artefact.language_kind {
            LanguageKind::Java(JavaKind::Class)
            | LanguageKind::Java(JavaKind::Interface)
            | LanguageKind::Java(JavaKind::Enum) => {
                type_targets
                    .entry(artefact.name.clone())
                    .or_insert_with(|| artefact.symbol_fqn.clone());
            }
            LanguageKind::Java(JavaKind::Method) => {
                if let Some(parent) = artefact.parent_symbol_fqn.as_ref() {
                    method_targets_by_parent_and_name
                        .entry((parent.clone(), artefact.name.clone()))
                        .or_insert_with(|| artefact.symbol_fqn.clone());
                }
            }
            LanguageKind::Java(JavaKind::Field) => {
                if let Some(parent) = artefact.parent_symbol_fqn.as_ref() {
                    field_targets_by_parent_and_name
                        .entry((parent.clone(), artefact.name.clone()))
                        .or_insert_with(|| artefact.symbol_fqn.clone());
                }
            }
            LanguageKind::Java(JavaKind::Constructor) => {
                if let Some(parent) = artefact.parent_symbol_fqn.as_ref() {
                    constructor_targets_by_type
                        .entry(parent.clone())
                        .or_insert_with(|| artefact.symbol_fqn.clone());
                }
            }
            _ => {}
        }
    }

    let (import_edges, imported_type_refs, imported_static_refs) =
        collect_java_import_data(root, content, path);
    let mut edges = import_edges;
    let mut seen_imports = HashSet::new();
    let mut seen_calls = HashSet::new();
    let mut seen_refs = HashSet::new();
    let mut seen_extends = HashSet::new();
    let mut seen_implements = HashSet::new();
    for edge in &edges {
        let key = format!(
            "{}|{}|{}",
            edge.from_symbol_fqn,
            edge.to_symbol_ref.as_deref().unwrap_or(""),
            edge.start_line.unwrap_or_default()
        );
        seen_imports.insert(key);
    }
    let mut traversal = JavaTraversalCtx {
        content,
        path,
        callables: &callables,
        type_targets: &type_targets,
        imported_type_refs: &imported_type_refs,
        imported_static_refs: &imported_static_refs,
        method_targets_by_parent_and_name: &method_targets_by_parent_and_name,
        field_targets_by_parent_and_name: &field_targets_by_parent_and_name,
        constructor_targets_by_type: &constructor_targets_by_type,
        out: &mut edges,
        seen_imports: &mut seen_imports,
        seen_calls: &mut seen_calls,
        seen_refs: &mut seen_refs,
        seen_extends: &mut seen_extends,
        seen_implements: &mut seen_implements,
    };
    collect_java_edges_recursive(root, &mut traversal);
    Ok(edges)
}

fn collect_java_edges_recursive(node: Node<'_>, traversal: &mut JavaTraversalCtx<'_>) {
    match node.kind() {
        "import_declaration" => collect_java_import_edge(node, traversal),
        "method_invocation" => collect_java_method_invocation_edge(node, traversal),
        "object_creation_expression" => collect_java_object_creation_edge(node, traversal),
        "explicit_constructor_invocation" => {
            collect_java_explicit_constructor_invocation(node, traversal)
        }
        "field_declaration" => collect_java_field_type_references(node, traversal),
        "method_declaration" => collect_java_method_type_references(node, traversal),
        "constructor_declaration" => collect_java_constructor_type_references(node, traversal),
        "class_declaration" => collect_java_class_relationships(node, traversal),
        "interface_declaration" => collect_java_interface_relationships(node, traversal),
        "enum_declaration" => collect_java_enum_relationships(node, traversal),
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_java_edges_recursive(child, traversal);
    }
}
