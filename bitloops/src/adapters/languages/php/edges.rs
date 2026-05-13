use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};

use crate::host::devql::{
    CallForm, CanonicalKindProjection, EdgeKind, ImportForm, RefKind, Resolution,
};
use crate::host::language_adapter::{DependencyEdge, EdgeMetadata, LanguageArtefact};

pub(crate) fn extract_php_dependency_edges(
    content: &str,
    path: &str,
    artefacts: &[LanguageArtefact],
) -> Result<Vec<DependencyEdge>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_php::LANGUAGE_PHP.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter php language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let mut edges = Vec::new();
    let callables = artefacts
        .iter()
        .filter(|artefact| {
            artefact
                .canonical_kind
                .as_deref()
                .and_then(CanonicalKindProjection::from_str)
                .is_some_and(|kind| {
                    matches!(
                        kind,
                        CanonicalKindProjection::Function | CanonicalKindProjection::Method
                    )
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    let callable_name_to_fqn = callables
        .iter()
        .map(|artefact| (artefact.name.clone(), artefact.symbol_fqn.clone()))
        .collect::<HashMap<_, _>>();

    let mut seen = HashSet::new();
    collect_php_edges_recursive(
        root,
        content,
        path,
        &callables,
        &callable_name_to_fqn,
        &mut seen,
        &mut edges,
    );

    edges.sort_by(|lhs, rhs| lhs.from_symbol_fqn.cmp(&rhs.from_symbol_fqn));
    Ok(edges)
}

fn collect_php_edges_recursive(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    callables: &[LanguageArtefact],
    callable_name_to_fqn: &HashMap<String, String>,
    seen: &mut HashSet<String>,
    out: &mut Vec<DependencyEdge>,
) {
    let line_no = node.start_position().row as i32 + 1;

    match node.kind() {
        "namespace_use_declaration" => {
            let text = node.utf8_text(content.as_bytes()).ok().unwrap_or("");
            if let Some(import_ref) = text
                .replace("use", "")
                .replace(';', "")
                .split(',')
                .map(str::trim)
                .find(|entry| !entry.is_empty())
            {
                let key = format!("import|{path}|{import_ref}|{line_no}");
                if seen.insert(key) {
                    out.push(DependencyEdge {
                        edge_kind: EdgeKind::Imports,
                        from_symbol_fqn: path.to_string(),
                        to_target_symbol_fqn: None,
                        to_symbol_ref: Some(import_ref.to_string()),
                        start_line: Some(line_no),
                        end_line: Some(line_no),
                        metadata: EdgeMetadata::import(ImportForm::Binding),
                    });
                }
            }
        }
        "function_call_expression" => {
            if let Some(owner) = smallest_enclosing_callable(line_no, callables)
                && let Some(function_node) = node.child_by_field_name("function")
                && let Ok(target_name) = function_node.utf8_text(content.as_bytes())
            {
                let target_name = target_name.trim();
                if target_name.is_empty() {
                    return;
                }
                let (to_target_symbol_fqn, to_symbol_ref, resolution) =
                    if let Some(target_fqn) = callable_name_to_fqn.get(target_name) {
                        (Some(target_fqn.clone()), None, Resolution::Local)
                    } else {
                        (
                            None,
                            Some(format!("{path}::{target_name}")),
                            Resolution::Unresolved,
                        )
                    };
                let key = format!(
                    "call|{}|{}|{}|{}",
                    owner.symbol_fqn,
                    to_target_symbol_fqn.as_deref().unwrap_or(""),
                    to_symbol_ref.as_deref().unwrap_or(""),
                    line_no
                );
                if seen.insert(key) {
                    out.push(DependencyEdge {
                        edge_kind: EdgeKind::Calls,
                        from_symbol_fqn: owner.symbol_fqn.clone(),
                        to_target_symbol_fqn,
                        to_symbol_ref,
                        start_line: Some(line_no),
                        end_line: Some(line_no),
                        metadata: EdgeMetadata::call(CallForm::Function, resolution),
                    });
                }
            }
        }
        "base_clause" => {
            if let Some(owner) = smallest_enclosing_callable(line_no, callables)
                && let Ok(target_name) = node.utf8_text(content.as_bytes())
            {
                let clean = target_name.trim();
                if !clean.is_empty() {
                    let key = format!("ref|{}|{}|{}", owner.symbol_fqn, clean, line_no);
                    if seen.insert(key) {
                        out.push(DependencyEdge {
                            edge_kind: EdgeKind::References,
                            from_symbol_fqn: owner.symbol_fqn.clone(),
                            to_target_symbol_fqn: None,
                            to_symbol_ref: Some(clean.to_string()),
                            start_line: Some(line_no),
                            end_line: Some(line_no),
                            metadata: EdgeMetadata::reference(RefKind::Type, Resolution::Unresolved),
                        });
                    }
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_php_edges_recursive(
            child,
            content,
            path,
            callables,
            callable_name_to_fqn,
            seen,
            out,
        );
    }
}

fn smallest_enclosing_callable(
    line_no: i32,
    callables: &[LanguageArtefact],
) -> Option<&LanguageArtefact> {
    callables
        .iter()
        .filter(|artefact| artefact.start_line <= line_no && artefact.end_line >= line_no)
        .min_by_key(|artefact| artefact.end_line - artefact.start_line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::languages::php::extraction::extract_php_artefacts;

    #[test]
    fn extract_php_edges_extracts_imports_and_calls() {
        let content = r#"<?php
use App\Core\Helper;

function helper() {
    return 1;
}

function run() {
    return helper();
}
"#;
        let artefacts = extract_php_artefacts(content, "src/main.php").expect("artefacts");
        let edges = extract_php_dependency_edges(content, "src/main.php", &artefacts)
            .expect("php edges");
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Imports
                && edge.to_symbol_ref.as_deref() == Some("App\\Core\\Helper")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Calls
                && edge.from_symbol_fqn == "src/main.php::run"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/main.php::helper")
        }));
    }
}
