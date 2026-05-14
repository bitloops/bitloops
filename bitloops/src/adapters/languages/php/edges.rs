use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};

use crate::host::devql::{
    CallForm, CanonicalKindProjection, EdgeKind, ImportForm, RefKind, Resolution,
};
use crate::host::language_adapter::{DependencyEdge, EdgeMetadata, LanguageArtefact};

struct PhpTraversalCtx<'a> {
    content: &'a str,
    path: &'a str,
    artefacts: &'a [LanguageArtefact],
    callables: &'a [LanguageArtefact],
    callable_name_to_fqn: &'a HashMap<String, String>,
    type_targets: &'a HashMap<String, String>,
    imported_type_refs: &'a HashMap<String, String>,
    out: &'a mut Vec<DependencyEdge>,
    seen_imports: &'a mut HashSet<String>,
    seen_calls: &'a mut HashSet<String>,
    seen_refs: &'a mut HashSet<String>,
    seen_extends: &'a mut HashSet<String>,
    seen_implements: &'a mut HashSet<String>,
}

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
    let types = artefacts
        .iter()
        .filter(|artefact| {
            artefact
                .canonical_kind
                .as_deref()
                .and_then(CanonicalKindProjection::from_str)
                .is_some_and(|kind| {
                    matches!(
                        kind,
                        CanonicalKindProjection::Type
                            | CanonicalKindProjection::Interface
                            | CanonicalKindProjection::Enum
                    )
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    let type_targets = types
        .iter()
        .map(|artefact| (artefact.name.clone(), artefact.symbol_fqn.clone()))
        .collect::<HashMap<_, _>>();
    let imported_type_refs = collect_php_imported_type_refs(root, content);

    let mut edges = Vec::new();
    let mut seen_imports = HashSet::new();
    let mut seen_calls = HashSet::new();
    let mut seen_refs = HashSet::new();
    let mut seen_extends = HashSet::new();
    let mut seen_implements = HashSet::new();
    let mut ctx = PhpTraversalCtx {
        content,
        path,
        artefacts,
        callables: &callables,
        callable_name_to_fqn: &callable_name_to_fqn,
        type_targets: &type_targets,
        imported_type_refs: &imported_type_refs,
        out: &mut edges,
        seen_imports: &mut seen_imports,
        seen_calls: &mut seen_calls,
        seen_refs: &mut seen_refs,
        seen_extends: &mut seen_extends,
        seen_implements: &mut seen_implements,
    };
    collect_php_edges_recursive(root, &mut ctx);

    edges.sort_by(|lhs, rhs| lhs.from_symbol_fqn.cmp(&rhs.from_symbol_fqn));
    Ok(edges)
}

fn collect_php_edges_recursive(node: tree_sitter::Node, ctx: &mut PhpTraversalCtx<'_>) {
    match node.kind() {
        "namespace_use_declaration" => collect_php_import_edges(node, ctx),
        "function_call_expression" => collect_php_call_edge(node, ctx),
        "base_clause" => collect_php_extends_edges(node, ctx),
        "class_interface_clause" => collect_php_implements_edges(node, ctx),
        "named_type" => collect_php_type_reference_edge(node, ctx),
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_php_edges_recursive(child, ctx);
    }
}

fn collect_php_import_edges(node: tree_sitter::Node, ctx: &mut PhpTraversalCtx<'_>) {
    let line_no = node.start_position().row as i32 + 1;
    for entry in php_namespace_use_entries(node, ctx.content) {
        let import_ref = php_import_target_ref(&entry);
        if import_ref.is_empty() {
            continue;
        }
        let key = format!("{}|{}|{}", ctx.path, import_ref, line_no);
        if !ctx.seen_imports.insert(key) {
            continue;
        }
        ctx.out.push(DependencyEdge {
            edge_kind: EdgeKind::Imports,
            from_symbol_fqn: ctx.path.to_string(),
            to_target_symbol_fqn: None,
            to_symbol_ref: Some(import_ref),
            start_line: Some(line_no),
            end_line: Some(line_no),
            metadata: EdgeMetadata::import(ImportForm::Binding),
        });
    }
}

fn collect_php_call_edge(node: tree_sitter::Node, ctx: &mut PhpTraversalCtx<'_>) {
    let line_no = node.start_position().row as i32 + 1;
    if let Some(owner) = smallest_enclosing_callable(line_no, ctx.callables)
        && let Some(function_node) = node.child_by_field_name("function")
        && let Ok(target_name) = function_node.utf8_text(ctx.content.as_bytes())
    {
        let target_name = target_name.trim();
        if target_name.is_empty() {
            return;
        }
        let (to_target_symbol_fqn, to_symbol_ref, resolution) =
            if let Some(target_fqn) = ctx.callable_name_to_fqn.get(target_name) {
                (Some(target_fqn.clone()), None, Resolution::Local)
            } else {
                (
                    None,
                    Some(format!("{}::{target_name}", ctx.path)),
                    Resolution::Unresolved,
                )
            };
        let key = format!(
            "{}|{}|{}|{}|{}",
            owner.symbol_fqn,
            to_target_symbol_fqn.as_deref().unwrap_or(""),
            to_symbol_ref.as_deref().unwrap_or(""),
            line_no,
            resolution.as_str()
        );
        if !ctx.seen_calls.insert(key) {
            return;
        }
        ctx.out.push(DependencyEdge {
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

fn collect_php_extends_edges(node: tree_sitter::Node, ctx: &mut PhpTraversalCtx<'_>) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_type(line_no, ctx.artefacts) else {
        return;
    };
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let Some(target_name) = php_type_name_from_node(child, ctx.content) else {
            continue;
        };
        push_php_relationship_edge(
            EdgeKind::Extends,
            &owner.symbol_fqn,
            &target_name,
            child.start_position().row as i32 + 1,
            ctx.type_targets,
            ctx.imported_type_refs,
            ctx.seen_extends,
            ctx.out,
        );
    }
}

fn collect_php_implements_edges(node: tree_sitter::Node, ctx: &mut PhpTraversalCtx<'_>) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_type(line_no, ctx.artefacts) else {
        return;
    };
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let Some(target_name) = php_type_name_from_node(child, ctx.content) else {
            continue;
        };
        push_php_relationship_edge(
            EdgeKind::Implements,
            &owner.symbol_fqn,
            &target_name,
            child.start_position().row as i32 + 1,
            ctx.type_targets,
            ctx.imported_type_refs,
            ctx.seen_implements,
            ctx.out,
        );
    }
}

fn collect_php_type_reference_edge(node: tree_sitter::Node, ctx: &mut PhpTraversalCtx<'_>) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_symbol(line_no, ctx.artefacts) else {
        return;
    };
    let Some(target_name) = php_type_name_from_node(node, ctx.content) else {
        return;
    };
    let Some((to_target_symbol_fqn, to_symbol_ref, resolution)) =
        resolve_php_type_target(&target_name, ctx.type_targets, ctx.imported_type_refs)
    else {
        return;
    };
    let key = format!(
        "{}|{}|{}|{}|{}",
        owner.symbol_fqn,
        to_target_symbol_fqn.as_deref().unwrap_or(""),
        to_symbol_ref.as_deref().unwrap_or(""),
        line_no,
        resolution.as_str()
    );
    if !ctx.seen_refs.insert(key) {
        return;
    }
    ctx.out.push(DependencyEdge {
        edge_kind: EdgeKind::References,
        from_symbol_fqn: owner.symbol_fqn,
        to_target_symbol_fqn,
        to_symbol_ref,
        start_line: Some(line_no),
        end_line: Some(line_no),
        metadata: EdgeMetadata::reference(RefKind::Type, resolution),
    });
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

fn smallest_enclosing_type(
    line_no: i32,
    artefacts: &[LanguageArtefact],
) -> Option<&LanguageArtefact> {
    artefacts
        .iter()
        .filter(|artefact| artefact.start_line <= line_no && artefact.end_line >= line_no)
        .filter(|artefact| {
            matches!(
                artefact.canonical_kind.as_deref(),
                Some("type") | Some("interface") | Some("enum")
            )
        })
        .min_by_key(|artefact| artefact.end_line - artefact.start_line)
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

fn collect_php_imported_type_refs(
    root: tree_sitter::Node,
    content: &str,
) -> HashMap<String, String> {
    let mut refs = HashMap::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "namespace_use_declaration" {
            for entry in php_namespace_use_entries(node, content) {
                let import_ref = php_import_target_ref(&entry);
                if import_ref.is_empty() {
                    continue;
                }
                let alias = php_import_alias(&entry);
                refs.entry(alias).or_insert(import_ref);
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    refs
}

fn php_namespace_use_entries(node: tree_sitter::Node, content: &str) -> Vec<String> {
    let Ok(text) = node.utf8_text(content.as_bytes()) else {
        return Vec::new();
    };
    let trimmed = text.trim();
    let rest = strip_case_insensitive_prefix(trimmed, "use")
        .unwrap_or(trimmed)
        .trim()
        .trim_end_matches(';')
        .trim();
    rest.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

fn php_import_target_ref(entry: &str) -> String {
    let without_kind = strip_case_insensitive_prefix(entry.trim(), "function")
        .or_else(|| strip_case_insensitive_prefix(entry.trim(), "const"))
        .unwrap_or(entry)
        .trim();
    split_case_insensitive_once(without_kind, " as ")
        .map(|(target, _)| target.trim())
        .unwrap_or(without_kind)
        .trim_start_matches('\\')
        .trim()
        .to_string()
}

fn php_import_alias(entry: &str) -> String {
    let without_kind = strip_case_insensitive_prefix(entry.trim(), "function")
        .or_else(|| strip_case_insensitive_prefix(entry.trim(), "const"))
        .unwrap_or(entry)
        .trim();
    let alias = split_case_insensitive_once(without_kind, " as ")
        .map(|(_, alias)| alias.trim())
        .unwrap_or_else(|| php_qualified_lookup_name(without_kind));
    alias.trim().to_string()
}

fn php_type_name_from_node(node: tree_sitter::Node, content: &str) -> Option<String> {
    node.utf8_text(content.as_bytes())
        .ok()
        .map(str::trim)
        .map(|text| text.trim_start_matches('\\').to_string())
        .filter(|text| !text.is_empty())
}

fn php_qualified_lookup_name(name: &str) -> &str {
    name.trim()
        .trim_start_matches('\\')
        .rsplit('\\')
        .next()
        .unwrap_or(name)
        .trim()
}

fn resolve_php_type_target(
    target_name: &str,
    type_targets: &HashMap<String, String>,
    imported_type_refs: &HashMap<String, String>,
) -> Option<(Option<String>, Option<String>, Resolution)> {
    let lookup_name = php_qualified_lookup_name(target_name);
    if let Some(target_fqn) = type_targets.get(lookup_name) {
        Some((Some(target_fqn.clone()), None, Resolution::Local))
    } else {
        imported_type_refs
            .get(lookup_name)
            .map(|symbol_ref| (None, Some(symbol_ref.clone()), Resolution::Import))
    }
}

fn push_php_relationship_edge(
    edge_kind: EdgeKind,
    from_symbol_fqn: &str,
    target_name: &str,
    line_no: i32,
    type_targets: &HashMap<String, String>,
    imported_type_refs: &HashMap<String, String>,
    seen: &mut HashSet<String>,
    out: &mut Vec<DependencyEdge>,
) {
    let (to_target_symbol_fqn, to_symbol_ref, resolution) =
        resolve_php_type_target(target_name, type_targets, imported_type_refs).unwrap_or_else(
            || {
                (
                    None,
                    Some(target_name.trim_start_matches('\\').to_string()),
                    Resolution::Unresolved,
                )
            },
        );
    let key = format!(
        "{}|{}|{}|{}|{}|{}",
        edge_kind.as_str(),
        from_symbol_fqn,
        to_target_symbol_fqn.as_deref().unwrap_or(""),
        to_symbol_ref.as_deref().unwrap_or(""),
        line_no,
        resolution.as_str()
    );
    if !seen.insert(key) {
        return;
    }
    out.push(DependencyEdge {
        edge_kind,
        from_symbol_fqn: from_symbol_fqn.to_string(),
        to_target_symbol_fqn,
        to_symbol_ref,
        start_line: Some(line_no),
        end_line: Some(line_no),
        metadata: EdgeMetadata::none(),
    });
}

fn strip_case_insensitive_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    if value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
    {
        Some(&value[prefix.len()..])
    } else {
        None
    }
}

fn split_case_insensitive_once<'a>(value: &'a str, needle: &str) -> Option<(&'a str, &'a str)> {
    let lower = value.to_ascii_lowercase();
    let index = lower.find(needle)?;
    Some((&value[..index], &value[index + needle.len()..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::languages::php::extraction::extract_php_artefacts;

    #[test]
    fn extract_php_edges_extracts_imports_calls_relationships_and_references() {
        let content = r#"<?php
use App\Contracts\RemoteRunner;

class BaseService {}
interface LocalRunner {}
class Helper {}

class UserService extends BaseService implements RemoteRunner, LocalRunner {
    private Helper $helper;

    public function run(Helper $helper): Helper {
        return helper();
    }
}

function helper(): Helper {
    return new Helper();
}
"#;
        let artefacts = extract_php_artefacts(content, "src/UserService.php").expect("artefacts");
        let edges = extract_php_dependency_edges(content, "src/UserService.php", &artefacts)
            .expect("php edges");
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Imports
                && edge.to_symbol_ref.as_deref() == Some("App\\Contracts\\RemoteRunner")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Calls
                && edge.from_symbol_fqn == "src/UserService.php::UserService::run"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/UserService.php::helper")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Extends
                && edge.from_symbol_fqn == "src/UserService.php::UserService"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/UserService.php::BaseService")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Implements
                && edge.from_symbol_fqn == "src/UserService.php::UserService"
                && edge.to_symbol_ref.as_deref() == Some("App\\Contracts\\RemoteRunner")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::Implements
                && edge.from_symbol_fqn == "src/UserService.php::UserService"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/UserService.php::LocalRunner")
        }));
        assert!(edges.iter().any(|edge| {
            edge.edge_kind == EdgeKind::References
                && edge.from_symbol_fqn == "src/UserService.php::UserService::$helper"
                && edge.to_target_symbol_fqn.as_deref() == Some("src/UserService.php::Helper")
        }));
    }
}
