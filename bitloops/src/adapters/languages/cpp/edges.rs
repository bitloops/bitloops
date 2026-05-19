use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::host::devql::{CallForm, EdgeKind, ImportForm, RefKind, Resolution};
use crate::host::language_adapter::{
    DependencyEdge, EdgeMetadata, LanguageArtefact,
    edges_shared::{EdgeCollector, SymbolLookup, push_extends_edge, push_reference_edge},
};

pub(crate) fn extract_cpp_dependency_edges(
    content: &str,
    path: &str,
    artefacts: &[LanguageArtefact],
) -> Result<Vec<DependencyEdge>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_cpp::LANGUAGE.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter c++ language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let callable_name_to_fqn = callable_name_to_fqn(artefacts);
    let type_targets = type_targets(artefacts);

    let root = tree.root_node();
    let mut edges = Vec::new();
    let mut seen_imports = HashSet::new();
    let mut seen_calls = HashSet::new();
    let mut seen_refs = HashSet::new();
    let mut seen_extends = HashSet::new();

    collect_cpp_edges_recursive(
        root,
        content,
        path,
        artefacts,
        &callable_name_to_fqn,
        &type_targets,
        &mut edges,
        &mut seen_imports,
        &mut seen_calls,
        &mut seen_refs,
        &mut seen_extends,
    );

    Ok(edges)
}

#[allow(clippy::too_many_arguments)]
fn collect_cpp_edges_recursive(
    node: Node<'_>,
    content: &str,
    path: &str,
    artefacts: &[LanguageArtefact],
    callable_name_to_fqn: &HashMap<String, String>,
    type_targets: &HashMap<String, String>,
    edges: &mut Vec<DependencyEdge>,
    seen_imports: &mut HashSet<String>,
    seen_calls: &mut HashSet<String>,
    seen_refs: &mut HashSet<String>,
    seen_extends: &mut HashSet<String>,
) {
    match node.kind() {
        "preproc_include" => {
            let include_ref = node
                .utf8_text(content.as_bytes())
                .ok()
                .map(str::trim)
                .map(str::to_string)
                .unwrap_or_default();
            let key = format!("{path}|{include_ref}");
            if seen_imports.insert(key) {
                edges.push(DependencyEdge {
                    edge_kind: EdgeKind::Imports,
                    from_symbol_fqn: path.to_string(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(include_ref),
                    start_line: Some(node.start_position().row as i32 + 1),
                    end_line: Some(node.end_position().row as i32 + 1),
                    metadata: EdgeMetadata::import(ImportForm::Binding),
                });
            }
        }
        "call_expression" => {
            collect_call_edge(
                node,
                content,
                path,
                artefacts,
                callable_name_to_fqn,
                edges,
                seen_calls,
            );
        }
        "base_class_clause" => {
            collect_extends_edges(node, content, artefacts, type_targets, edges, seen_extends);
        }
        "identifier" | "type_identifier" | "qualified_identifier" => {
            collect_reference_edge(node, content, artefacts, type_targets, edges, seen_refs);
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_cpp_edges_recursive(
            child,
            content,
            path,
            artefacts,
            callable_name_to_fqn,
            type_targets,
            edges,
            seen_imports,
            seen_calls,
            seen_refs,
            seen_extends,
        );
    }
}

fn collect_call_edge(
    node: Node<'_>,
    content: &str,
    path: &str,
    artefacts: &[LanguageArtefact],
    callable_name_to_fqn: &HashMap<String, String>,
    edges: &mut Vec<DependencyEdge>,
    seen_calls: &mut HashSet<String>,
) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_symbol(line_no, artefacts) else {
        return;
    };

    let Some(function_node) = node.child_by_field_name("function") else {
        return;
    };
    let Some(raw_name) = trimmed_node_text(function_node, content) else {
        return;
    };
    let call_name = raw_name
        .rsplit("::")
        .next()
        .map(str::to_string)
        .unwrap_or(raw_name.clone());

    let (to_target_symbol_fqn, to_symbol_ref, resolution) =
        if let Some(target_fqn) = callable_name_to_fqn.get(&call_name) {
            (Some(target_fqn.clone()), None, Resolution::Local)
        } else {
            (
                None,
                Some(format!("{path}::{raw_name}")),
                Resolution::Unresolved,
            )
        };

    let key = format!(
        "{}|{}|{}|{}",
        owner.symbol_fqn,
        to_target_symbol_fqn.as_deref().unwrap_or(""),
        to_symbol_ref.as_deref().unwrap_or(""),
        line_no,
    );
    if !seen_calls.insert(key) {
        return;
    }

    edges.push(DependencyEdge {
        edge_kind: EdgeKind::Calls,
        from_symbol_fqn: owner.symbol_fqn,
        to_target_symbol_fqn,
        to_symbol_ref,
        start_line: Some(line_no),
        end_line: Some(line_no),
        metadata: EdgeMetadata::call(CallForm::Function, resolution),
    });
}

fn collect_extends_edges(
    node: Node<'_>,
    content: &str,
    artefacts: &[LanguageArtefact],
    type_targets: &HashMap<String, String>,
    edges: &mut Vec<DependencyEdge>,
    seen_extends: &mut HashSet<String>,
) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_type(line_no, artefacts) else {
        return;
    };

    let mut collector = EdgeCollector {
        out: edges,
        seen: seen_extends,
    };
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(name) = trimmed_node_text(child, content) {
            let base_name = name.split('<').next().unwrap_or(&name).to_string();
            push_extends_edge(
                &mut collector,
                &owner.symbol_fqn,
                &base_name,
                line_no,
                &SymbolLookup {
                    local_targets: type_targets,
                    imported_symbol_refs: None,
                },
            );
        }
    }
}

fn collect_reference_edge(
    node: Node<'_>,
    content: &str,
    artefacts: &[LanguageArtefact],
    type_targets: &HashMap<String, String>,
    edges: &mut Vec<DependencyEdge>,
    seen_refs: &mut HashSet<String>,
) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_symbol(line_no, artefacts) else {
        return;
    };
    let Some(name) = trimmed_node_text(node, content) else {
        return;
    };
    if is_trivial_reference_name(&name) {
        return;
    }

    push_reference_edge(
        &mut EdgeCollector {
            out: edges,
            seen: seen_refs,
        },
        &owner.symbol_fqn,
        &name,
        line_no,
        RefKind::Type,
        &SymbolLookup {
            local_targets: type_targets,
            imported_symbol_refs: None,
        },
    );
}

fn callable_name_to_fqn(artefacts: &[LanguageArtefact]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for artefact in artefacts {
        if artefact
            .canonical_kind
            .as_deref()
            .is_some_and(|kind| kind == "function" || kind == "method")
        {
            map.entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }
    }
    map
}

fn type_targets(artefacts: &[LanguageArtefact]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for artefact in artefacts {
        if artefact
            .canonical_kind
            .as_deref()
            .is_some_and(|kind| kind == "type" || kind == "interface" || kind == "enum")
        {
            map.entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }
    }
    map
}

fn trimmed_node_text(node: Node<'_>, content: &str) -> Option<String> {
    node.utf8_text(content.as_bytes())
        .ok()
        .map(str::trim)
        .map(str::to_string)
        .filter(|text| !text.is_empty())
}

fn smallest_enclosing_symbol(
    line_no: i32,
    artefacts: &[LanguageArtefact],
) -> Option<LanguageArtefact> {
    artefacts
        .iter()
        .filter(|artefact| artefact.start_line <= line_no && artefact.end_line >= line_no)
        .min_by_key(|artefact| artefact.end_line - artefact.start_line)
        .cloned()
}

fn smallest_enclosing_type(
    line_no: i32,
    artefacts: &[LanguageArtefact],
) -> Option<LanguageArtefact> {
    artefacts
        .iter()
        .filter(|artefact| artefact.start_line <= line_no && artefact.end_line >= line_no)
        .filter(|artefact| {
            artefact
                .canonical_kind
                .as_deref()
                .is_some_and(|kind| kind == "type" || kind == "interface" || kind == "enum")
        })
        .min_by_key(|artefact| artefact.end_line - artefact.start_line)
        .cloned()
}

fn is_trivial_reference_name(name: &str) -> bool {
    matches!(name, "if" | "for" | "while" | "return" | "new" | "delete")
}

#[cfg(test)]
mod tests {
    use super::extract_cpp_dependency_edges;
    use crate::adapters::languages::cpp::extraction::extract_cpp_artefacts;
    use crate::host::devql::EdgeKind;

    #[test]
    fn extract_cpp_dependency_edges_collects_import_and_call_edges() {
        let path = "src/main.cpp";
        let content = r#"
#include <vector>

class Base {};
class Child : public Base {
public:
    int helper(int x) { return x + 1; }
    int run(int x) { return helper(x); }
};
"#;
        let artefacts = extract_cpp_artefacts(content, path).expect("extract cpp artefacts");
        let edges = extract_cpp_dependency_edges(content, path, &artefacts).expect("extract edges");

        assert!(edges.iter().any(|edge| edge.edge_kind == EdgeKind::Imports));
        assert!(edges.iter().any(|edge| edge.edge_kind == EdgeKind::Calls));
    }

    #[test]
    fn extract_cpp_dependency_edges_emit_extends_and_reference_edges() {
        let content = r#"#include <vector>

class Base {};

template <typename T>
class Box {};

class Helper {
public:
    int adjust(int value) { return value + 1; }
};

class UserService : public Base, public Box<Helper> {
public:
    Helper helper;

    int local_helper(int value) { return value + 1; }

    int run() {
        return local_helper(1) + external::lookup();
    }
};
"#;
        let path = "src/main.cpp";
        let artefacts = extract_cpp_artefacts(content, path).expect("extract cpp artefacts");
        let edges = extract_cpp_dependency_edges(content, path, &artefacts).expect("extract edges");

        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Imports
                && edge.to_symbol_ref.as_deref() == Some("#include <vector>")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Calls
                && edge.from_symbol_fqn == "src/main.cpp::UserService::run"
                && edge.to_target_symbol_fqn.as_deref()
                    == Some("src/main.cpp::UserService::local_helper")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Calls
                && edge.from_symbol_fqn == "src/main.cpp::UserService::run"
                && edge.to_symbol_ref.as_deref() == Some("src/main.cpp::external::lookup")
                && edge.to_target_symbol_fqn.is_none()
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Extends
                && edge.from_symbol_fqn == "src/main.cpp::UserService"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/main.cpp::Base")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Extends
                && edge.from_symbol_fqn == "src/main.cpp::UserService"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/main.cpp::Box")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::References
                && edge.from_symbol_fqn == "src/main.cpp::UserService::helper"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/main.cpp::Helper")
        }));
    }

    #[test]
    fn extract_cpp_dependency_edges_deduplicates_repeated_imports_in_one_file() {
        let content = r#"#include <vector>
#include <vector>

class UserService {};
"#;

        let path = "src/main.cpp";
        let artefacts = extract_cpp_artefacts(content, path).expect("extract cpp artefacts");
        let edges = extract_cpp_dependency_edges(content, path, &artefacts).expect("extract edges");

        let import_edges = edges
            .iter()
            .filter(|edge| {
                edge.edge_kind == EdgeKind::Imports
                    && edge.to_symbol_ref.as_deref() == Some("#include <vector>")
            })
            .count();

        assert_eq!(import_edges, 1);
    }
}
