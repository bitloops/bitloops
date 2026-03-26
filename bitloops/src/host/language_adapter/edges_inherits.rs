use std::collections::{HashMap, HashSet};

use super::edges_shared::{
    push_extends_edge, symbol_lookup_name_from_node, EdgeCollector, SymbolLookup,
};
use super::DependencyEdge;

// Extension edge extraction for JS/TS (extends) and Rust (supertraits).

pub(crate) fn collect_js_ts_extends_edges_recursive(
    node: tree_sitter::Node,
    content: &str,
    type_targets: &HashMap<String, String>,
    imported_symbol_refs: &HashMap<String, String>,
    seen: &mut HashSet<String>,
    out: &mut Vec<DependencyEdge>,
) {
    match node.kind() {
        "class_declaration" => {
            let Some(owner_name) = node
                .child_by_field_name("name")
                .and_then(|name| name.utf8_text(content.as_bytes()).ok())
                .map(str::trim)
                .filter(|name| !name.is_empty())
            else {
                return;
            };
            let Some(owner_fqn) = type_targets.get(owner_name).cloned() else {
                return;
            };

            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "class_heritage" {
                    continue;
                }
                let mut heritage_cursor = child.walk();
                for clause in child.named_children(&mut heritage_cursor) {
                    if clause.kind() != "extends_clause" {
                        continue;
                    }
                    let mut value_cursor = clause.walk();
                    for target in clause.children_by_field_name("value", &mut value_cursor) {
                        let line_no = target.start_position().row as i32 + 1;
                        if let Some(target_name) = symbol_lookup_name_from_node(target, content) {
                            push_extends_edge(
                                &mut EdgeCollector { out, seen },
                                &owner_fqn,
                                &target_name,
                                line_no,
                                &SymbolLookup {
                                    local_targets: type_targets,
                                    imported_symbol_refs: Some(imported_symbol_refs),
                                },
                            );
                        }
                    }
                }
            }
        }
        "interface_declaration" => {
            let Some(owner_name) = node
                .child_by_field_name("name")
                .and_then(|name| name.utf8_text(content.as_bytes()).ok())
                .map(str::trim)
                .filter(|name| !name.is_empty())
            else {
                return;
            };
            let Some(owner_fqn) = type_targets.get(owner_name).cloned() else {
                return;
            };

            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "extends_type_clause" {
                    continue;
                }
                let mut type_cursor = child.walk();
                for target in child.children_by_field_name("type", &mut type_cursor) {
                    let line_no = target.start_position().row as i32 + 1;
                    if let Some(target_name) = symbol_lookup_name_from_node(target, content) {
                        push_extends_edge(
                            &mut EdgeCollector { out, seen },
                            &owner_fqn,
                            &target_name,
                            line_no,
                            &SymbolLookup {
                                local_targets: type_targets,
                                imported_symbol_refs: Some(imported_symbol_refs),
                            },
                        );
                    }
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_js_ts_extends_edges_recursive(
            child,
            content,
            type_targets,
            imported_symbol_refs,
            seen,
            out,
        );
    }
}

pub(crate) fn collect_rust_extends_edges_recursive(
    node: tree_sitter::Node,
    content: &str,
    type_targets: &HashMap<String, String>,
    seen: &mut HashSet<String>,
    out: &mut Vec<DependencyEdge>,
) {
    if node.kind() == "trait_item" {
        let Some(owner_name) = node
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(content.as_bytes()).ok())
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            return;
        };
        let Some(owner_fqn) = type_targets.get(owner_name).cloned() else {
            return;
        };

        if let Some(bounds) = node.child_by_field_name("bounds") {
            let mut bounds_cursor = bounds.walk();
            for target in bounds.named_children(&mut bounds_cursor) {
                if target.kind() == "lifetime" {
                    continue;
                }
                let line_no = target.start_position().row as i32 + 1;
                if let Some(target_name) = symbol_lookup_name_from_node(target, content) {
                    push_extends_edge(
                        &mut EdgeCollector { out, seen },
                        &owner_fqn,
                        &target_name,
                        line_no,
                        &SymbolLookup {
                            local_targets: type_targets,
                            imported_symbol_refs: None,
                        },
                    );
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_rust_extends_edges_recursive(child, content, type_targets, seen, out);
    }
}
