use tree_sitter::Node;

use super::JavaTraversalCtx;
use super::support::java_type_name_from_node;
use crate::adapters::languages::java::extraction::trimmed_node_text;
use crate::host::devql::{EdgeKind, Resolution};
use crate::host::language_adapter::{
    DependencyEdge, EdgeMetadata,
    edges_shared::{EdgeCollector, SymbolLookup, push_extends_edge},
};

pub(super) fn collect_java_class_relationships(
    node: Node<'_>,
    traversal: &mut JavaTraversalCtx<'_>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Some(owner_name) = trimmed_node_text(name_node, traversal.content) else {
        return;
    };
    let Some(owner_fqn) = traversal.type_targets.get(&owner_name) else {
        return;
    };

    if let Some(superclass) = node.child_by_field_name("superclass")
        && let Some(type_node) = superclass.named_child(0)
        && let Some(type_name) = java_type_name_from_node(type_node, traversal.content)
    {
        push_extends_edge(
            &mut EdgeCollector {
                out: traversal.out,
                seen: traversal.seen_extends,
            },
            owner_fqn,
            &type_name,
            type_node.start_position().row as i32 + 1,
            &SymbolLookup {
                local_targets: traversal.type_targets,
                imported_symbol_refs: Some(traversal.imported_type_refs),
            },
        );
    }

    if let Some(interfaces) = node.child_by_field_name("interfaces") {
        collect_java_implements_edges(owner_fqn, interfaces, traversal);
    }
}

pub(super) fn collect_java_interface_relationships(
    node: Node<'_>,
    traversal: &mut JavaTraversalCtx<'_>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Some(owner_name) = trimmed_node_text(name_node, traversal.content) else {
        return;
    };
    let Some(owner_fqn) = traversal.type_targets.get(&owner_name) else {
        return;
    };

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "extends_interfaces" {
            continue;
        }
        if let Some(type_list) = child.named_child(0) {
            let mut list_cursor = type_list.walk();
            for type_node in type_list.named_children(&mut list_cursor) {
                if let Some(type_name) = java_type_name_from_node(type_node, traversal.content) {
                    push_extends_edge(
                        &mut EdgeCollector {
                            out: traversal.out,
                            seen: traversal.seen_extends,
                        },
                        owner_fqn,
                        &type_name,
                        type_node.start_position().row as i32 + 1,
                        &SymbolLookup {
                            local_targets: traversal.type_targets,
                            imported_symbol_refs: Some(traversal.imported_type_refs),
                        },
                    );
                }
            }
        }
    }
}

pub(super) fn collect_java_enum_relationships(
    node: Node<'_>,
    traversal: &mut JavaTraversalCtx<'_>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Some(owner_name) = trimmed_node_text(name_node, traversal.content) else {
        return;
    };
    let Some(owner_fqn) = traversal.type_targets.get(&owner_name) else {
        return;
    };
    if let Some(interfaces) = node.child_by_field_name("interfaces") {
        collect_java_implements_edges(owner_fqn, interfaces, traversal);
    }
}

fn collect_java_implements_edges(
    owner_fqn: &str,
    interfaces: Node<'_>,
    traversal: &mut JavaTraversalCtx<'_>,
) {
    let Some(type_list) = interfaces.named_child(0) else {
        return;
    };
    let mut cursor = type_list.walk();
    for type_node in type_list.named_children(&mut cursor) {
        let Some(type_name) = java_type_name_from_node(type_node, traversal.content) else {
            continue;
        };
        let line_no = type_node.start_position().row as i32 + 1;
        let (to_target_symbol_fqn, to_symbol_ref, _resolution) =
            if let Some(target_fqn) = traversal.type_targets.get(&type_name) {
                (Some(target_fqn.clone()), None, Resolution::Local)
            } else if let Some(symbol_ref) = traversal.imported_type_refs.get(&type_name) {
                (None, Some(symbol_ref.clone()), Resolution::Import)
            } else {
                (None, Some(type_name.clone()), Resolution::Unresolved)
            };
        let key = format!(
            "{}|{}|{}|{}",
            owner_fqn,
            to_target_symbol_fqn.as_deref().unwrap_or(""),
            to_symbol_ref.as_deref().unwrap_or(""),
            line_no
        );
        if !traversal.seen_implements.insert(key) {
            continue;
        }
        traversal.out.push(DependencyEdge {
            edge_kind: EdgeKind::Implements,
            from_symbol_fqn: owner_fqn.to_string(),
            to_target_symbol_fqn,
            to_symbol_ref,
            start_line: Some(line_no),
            end_line: Some(line_no),
            metadata: EdgeMetadata::none(),
        });
    }
}
