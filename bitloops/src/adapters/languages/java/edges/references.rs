use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use super::JavaTraversalCtx;
use super::support::{
    java_type_name_from_node, smallest_enclosing_callable, smallest_enclosing_type,
};
use crate::host::devql::RefKind;
use crate::host::language_adapter::{
    DependencyEdge,
    edges_shared::{EdgeCollector, SymbolLookup, push_reference_edge},
};

pub(super) fn collect_java_field_type_references(
    node: Node<'_>,
    traversal: &mut JavaTraversalCtx<'_>,
) {
    let Some(type_node) = node.child_by_field_name("type") else {
        return;
    };
    let Some(type_name) = java_type_name_from_node(type_node, traversal.content) else {
        return;
    };
    let mut cursor = node.walk();
    for declarator in node.children_by_field_name("declarator", &mut cursor) {
        let line_no = declarator.start_position().row as i32 + 1;
        let Some(owner) = field_owner_symbol_fqn(declarator, line_no, traversal) else {
            continue;
        };
        push_reference_edge(
            &mut EdgeCollector {
                out: traversal.out,
                seen: traversal.seen_refs,
            },
            &owner,
            &type_name,
            line_no,
            RefKind::Type,
            &SymbolLookup {
                local_targets: traversal.type_targets,
                imported_symbol_refs: Some(traversal.imported_type_refs),
            },
        );
    }
}

pub(super) fn collect_java_method_type_references(
    node: Node<'_>,
    traversal: &mut JavaTraversalCtx<'_>,
) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_callable(line_no, traversal.callables) else {
        return;
    };

    if let Some(type_node) = node.child_by_field_name("type")
        && let Some(type_name) = java_type_name_from_node(type_node, traversal.content)
    {
        push_reference_edge(
            &mut EdgeCollector {
                out: traversal.out,
                seen: traversal.seen_refs,
            },
            &owner.symbol_fqn,
            &type_name,
            line_no,
            RefKind::Type,
            &SymbolLookup {
                local_targets: traversal.type_targets,
                imported_symbol_refs: Some(traversal.imported_type_refs),
            },
        );
    }

    collect_parameter_type_references(
        node,
        &owner.symbol_fqn,
        traversal.imported_type_refs,
        traversal.type_targets,
        traversal.out,
        traversal.seen_refs,
        traversal.content,
    );
}

fn field_owner_symbol_fqn(
    declarator: Node<'_>,
    line_no: i32,
    traversal: &JavaTraversalCtx<'_>,
) -> Option<String> {
    let field_name = declarator
        .child_by_field_name("name")
        .or_else(|| declarator.named_child(0))
        .and_then(|name_node| name_node.utf8_text(traversal.content.as_bytes()).ok())
        .map(str::trim)
        .filter(|name| !name.is_empty())?;

    let enclosing_type = smallest_enclosing_type(line_no, traversal.types)?;

    traversal
        .field_targets_by_parent_and_name
        .get(&(enclosing_type.symbol_fqn, field_name.to_string()))
        .cloned()
}

pub(super) fn collect_java_constructor_type_references(
    node: Node<'_>,
    traversal: &mut JavaTraversalCtx<'_>,
) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_callable(line_no, traversal.callables) else {
        return;
    };
    collect_parameter_type_references(
        node,
        &owner.symbol_fqn,
        traversal.imported_type_refs,
        traversal.type_targets,
        traversal.out,
        traversal.seen_refs,
        traversal.content,
    );
}

fn collect_parameter_type_references(
    node: Node<'_>,
    from_symbol_fqn: &str,
    imported_type_refs: &HashMap<String, String>,
    type_targets: &HashMap<String, String>,
    out: &mut Vec<DependencyEdge>,
    seen_refs: &mut HashSet<String>,
    content: &str,
) {
    let Some(parameters) = node.child_by_field_name("parameters") else {
        return;
    };
    let mut cursor = parameters.walk();
    for parameter in parameters.named_children(&mut cursor) {
        let Some(type_node) = parameter.child_by_field_name("type") else {
            continue;
        };
        let Some(type_name) = java_type_name_from_node(type_node, content) else {
            continue;
        };
        push_reference_edge(
            &mut EdgeCollector {
                out,
                seen: seen_refs,
            },
            from_symbol_fqn,
            &type_name,
            parameter.start_position().row as i32 + 1,
            RefKind::Type,
            &SymbolLookup {
                local_targets: type_targets,
                imported_symbol_refs: Some(imported_type_refs),
            },
        );
    }
}
