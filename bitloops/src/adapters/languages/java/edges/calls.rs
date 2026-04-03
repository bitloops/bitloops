use super::support::{java_type_name_from_node, smallest_enclosing_callable};
use super::JavaTraversalCtx;
use crate::host::devql::{CallForm, EdgeKind, Resolution};
use crate::host::language_adapter::{DependencyEdge, EdgeMetadata};
use crate::adapters::languages::java::extraction::trimmed_node_text;
use tree_sitter::Node;

struct JavaCallEdgeSpec {
    from_symbol_fqn: String,
    to_target_symbol_fqn: Option<String>,
    to_symbol_ref: Option<String>,
    line_no: i32,
    call_form: CallForm,
    resolution: Resolution,
    unresolved_fallback: Option<String>,
}

pub(super) fn collect_java_method_invocation_edge(
    node: Node<'_>,
    traversal: &mut JavaTraversalCtx<'_>,
) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_callable(line_no, traversal.callables) else {
        return;
    };
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Some(name) = trimmed_node_text(name_node, traversal.content) else {
        return;
    };

    if let Some(object_node) = node.child_by_field_name("object") {
        let object_text = trimmed_node_text(object_node, traversal.content).unwrap_or_default();
        if object_text == "this" {
            let local_target = owner.parent_symbol_fqn.as_ref().and_then(|parent| {
                traversal
                    .method_targets_by_parent_and_name
                    .get(&(parent.clone(), name.clone()))
                    .cloned()
            });
            push_java_call_edge(traversal, JavaCallEdgeSpec {
                from_symbol_fqn: owner.symbol_fqn.clone(),
                to_target_symbol_fqn: local_target.clone(),
                to_symbol_ref: None,
                line_no,
                call_form: CallForm::Method,
                resolution: if local_target.is_some() {
                    Resolution::Local
                } else {
                    Resolution::Unresolved
                },
                unresolved_fallback: Some(format!("this::{name}")),
            });
            return;
        }

        if object_text == "super" {
            push_java_call_edge(traversal, JavaCallEdgeSpec {
                from_symbol_fqn: owner.symbol_fqn.clone(),
                to_target_symbol_fqn: None,
                to_symbol_ref: Some(format!("super::{name}")),
                line_no,
                call_form: CallForm::Method,
                resolution: Resolution::Unresolved,
                unresolved_fallback: None,
            });
            return;
        }

        if let Some(type_fqn) = traversal.type_targets.get(&object_text) {
            let local_target = traversal
                .method_targets_by_parent_and_name
                .get(&(type_fqn.clone(), name.clone()))
                .cloned();
            push_java_call_edge(traversal, JavaCallEdgeSpec {
                from_symbol_fqn: owner.symbol_fqn.clone(),
                to_target_symbol_fqn: local_target.clone(),
                to_symbol_ref: local_target.is_none().then(|| format!("{type_fqn}::{name}")),
                line_no,
                call_form: CallForm::Associated,
                resolution: if local_target.is_some() {
                    Resolution::Local
                } else {
                    Resolution::Unresolved
                },
                unresolved_fallback: None,
            });
            return;
        }

        if let Some(import_ref) = traversal.imported_type_refs.get(&object_text) {
            push_java_call_edge(traversal, JavaCallEdgeSpec {
                from_symbol_fqn: owner.symbol_fqn.clone(),
                to_target_symbol_fqn: None,
                to_symbol_ref: Some(format!("{import_ref}::{name}")),
                line_no,
                call_form: CallForm::Associated,
                resolution: Resolution::Import,
                unresolved_fallback: None,
            });
            return;
        }

        if let Some(field_target) = owner.parent_symbol_fqn.as_ref().and_then(|parent| {
            traversal
                .field_targets_by_parent_and_name
                .get(&(parent.clone(), object_text.clone()))
        }) {
            push_java_call_edge(traversal, JavaCallEdgeSpec {
                from_symbol_fqn: owner.symbol_fqn.clone(),
                to_target_symbol_fqn: None,
                to_symbol_ref: Some(format!("{field_target}::{name}")),
                line_no,
                call_form: CallForm::Member,
                resolution: Resolution::Unresolved,
                unresolved_fallback: None,
            });
            return;
        }

        push_java_call_edge(traversal, JavaCallEdgeSpec {
            from_symbol_fqn: owner.symbol_fqn.clone(),
            to_target_symbol_fqn: None,
            to_symbol_ref: Some(format!("{object_text}::{name}")),
            line_no,
            call_form: CallForm::Member,
            resolution: Resolution::Unresolved,
            unresolved_fallback: None,
        });
        return;
    }

    if let Some(parent) = owner.parent_symbol_fqn.as_ref()
        && let Some(target_fqn) = traversal
            .method_targets_by_parent_and_name
            .get(&(parent.clone(), name.clone()))
    {
        push_java_call_edge(traversal, JavaCallEdgeSpec {
            from_symbol_fqn: owner.symbol_fqn.clone(),
            to_target_symbol_fqn: Some(target_fqn.clone()),
            to_symbol_ref: None,
            line_no,
            call_form: CallForm::Method,
            resolution: Resolution::Local,
            unresolved_fallback: None,
        });
        return;
    }

    if let Some(import_ref) = traversal.imported_static_refs.get(&name) {
        push_java_call_edge(traversal, JavaCallEdgeSpec {
            from_symbol_fqn: owner.symbol_fqn.clone(),
            to_target_symbol_fqn: None,
            to_symbol_ref: Some(import_ref.clone()),
            line_no,
            call_form: CallForm::Associated,
            resolution: Resolution::Import,
            unresolved_fallback: None,
        });
        return;
    }

    push_java_call_edge(traversal, JavaCallEdgeSpec {
        from_symbol_fqn: owner.symbol_fqn.clone(),
        to_target_symbol_fqn: None,
        to_symbol_ref: Some(format!("{}::{name}", traversal.path)),
        line_no,
        call_form: CallForm::Method,
        resolution: Resolution::Unresolved,
        unresolved_fallback: None,
    });
}

pub(super) fn collect_java_object_creation_edge(
    node: Node<'_>,
    traversal: &mut JavaTraversalCtx<'_>,
) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_callable(line_no, traversal.callables) else {
        return;
    };
    let Some(type_node) = node.child_by_field_name("type") else {
        return;
    };
    let Some(type_name) = java_type_name_from_node(type_node, traversal.content) else {
        return;
    };

    if let Some(type_fqn) = traversal.type_targets.get(&type_name) {
        let target = traversal
            .constructor_targets_by_type
            .get(type_fqn)
            .cloned()
            .unwrap_or_else(|| format!("{type_fqn}::<init>"));
        push_java_call_edge(traversal, JavaCallEdgeSpec {
            from_symbol_fqn: owner.symbol_fqn.clone(),
            to_target_symbol_fqn: Some(target),
            to_symbol_ref: None,
            line_no,
            call_form: CallForm::Associated,
            resolution: Resolution::Local,
            unresolved_fallback: None,
        });
        return;
    }

    if let Some(import_ref) = traversal.imported_type_refs.get(&type_name) {
        push_java_call_edge(traversal, JavaCallEdgeSpec {
            from_symbol_fqn: owner.symbol_fqn.clone(),
            to_target_symbol_fqn: None,
            to_symbol_ref: Some(format!("{import_ref}::<init>")),
            line_no,
            call_form: CallForm::Associated,
            resolution: Resolution::Import,
            unresolved_fallback: None,
        });
        return;
    }

    push_java_call_edge(traversal, JavaCallEdgeSpec {
        from_symbol_fqn: owner.symbol_fqn.clone(),
        to_target_symbol_fqn: None,
        to_symbol_ref: Some(format!("{type_name}::<init>")),
        line_no,
        call_form: CallForm::Associated,
        resolution: Resolution::Unresolved,
        unresolved_fallback: None,
    });
}

pub(super) fn collect_java_explicit_constructor_invocation(
    node: Node<'_>,
    traversal: &mut JavaTraversalCtx<'_>,
) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_callable(line_no, traversal.callables) else {
        return;
    };
    let Some(parent) = owner.parent_symbol_fqn.as_ref() else {
        return;
    };
    let constructor_kind = node
        .child_by_field_name("constructor")
        .and_then(|constructor| trimmed_node_text(constructor, traversal.content))
        .unwrap_or_default();
    match constructor_kind.as_str() {
        "this" => {
            let target = traversal
                .constructor_targets_by_type
                .get(parent)
                .cloned()
                .unwrap_or_else(|| format!("{parent}::<init>"));
            push_java_call_edge(traversal, JavaCallEdgeSpec {
                from_symbol_fqn: owner.symbol_fqn.clone(),
                to_target_symbol_fqn: Some(target),
                to_symbol_ref: None,
                line_no,
                call_form: CallForm::Associated,
                resolution: Resolution::Local,
                unresolved_fallback: None,
            });
        }
        "super" => {
            push_java_call_edge(traversal, JavaCallEdgeSpec {
                from_symbol_fqn: owner.symbol_fqn.clone(),
                to_target_symbol_fqn: None,
                to_symbol_ref: Some("super::<init>".to_string()),
                line_no,
                call_form: CallForm::Associated,
                resolution: Resolution::Unresolved,
                unresolved_fallback: None,
            });
        }
        _ => {}
    }
}

fn push_java_call_edge(traversal: &mut JavaTraversalCtx<'_>, spec: JavaCallEdgeSpec) {
    let JavaCallEdgeSpec {
        from_symbol_fqn,
        to_target_symbol_fqn,
        to_symbol_ref,
        line_no,
        call_form,
        resolution,
        unresolved_fallback,
    } = spec;
    let symbol_ref = to_symbol_ref.or(unresolved_fallback);
    let key = format!(
        "{}|{}|{}|{}|{}|{}",
        from_symbol_fqn,
        to_target_symbol_fqn.as_deref().unwrap_or(""),
        symbol_ref.as_deref().unwrap_or(""),
        line_no,
        call_form.as_str(),
        resolution.as_str()
    );
    if !traversal.seen_calls.insert(key) {
        return;
    }
    traversal.out.push(DependencyEdge {
        edge_kind: EdgeKind::Calls,
        from_symbol_fqn,
        to_target_symbol_fqn,
        to_symbol_ref: symbol_ref,
        start_line: Some(line_no),
        end_line: Some(line_no),
        metadata: EdgeMetadata::call(call_form, resolution),
    });
}
