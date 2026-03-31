use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use tree_sitter::Node;

use super::extraction::{
    import_path_stem, package_name_from_root, strip_string_literal_delimiters, trimmed_node_text,
    type_name_from_node,
};
use crate::host::devql::{
    CallForm, CanonicalKindProjection, EdgeKind, ImportForm, RefKind, Resolution,
};
use crate::host::language_adapter::{
    DependencyEdge, EdgeMetadata, LanguageArtefact,
    edges_shared::{EdgeCollector, SymbolLookup, push_extends_edge, push_reference_edge},
};

struct GoTraversalCtx<'a> {
    content: &'a str,
    path: &'a str,
    package_name: Option<&'a str>,
    callable_name_to_fqn: &'a HashMap<String, String>,
    method_targets_by_type_and_name: &'a HashMap<(String, String), String>,
    imported_package_refs: &'a HashMap<String, String>,
    type_targets: &'a HashMap<String, String>,
    artefacts: &'a [LanguageArtefact],
    out: &'a mut Vec<DependencyEdge>,
    seen_imports: &'a mut HashSet<String>,
    seen_calls: &'a mut HashSet<String>,
    seen_refs: &'a mut HashSet<String>,
    seen_extends: &'a mut HashSet<String>,
}

pub(crate) fn extract_go_dependency_edges(
    content: &str,
    path: &str,
    artefacts: &[LanguageArtefact],
) -> Result<Vec<DependencyEdge>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter go language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let package_name = package_name_from_root(root, content);
    let mut callable_name_to_fqn = HashMap::new();
    let mut method_targets_by_type_and_name = HashMap::new();
    let mut type_targets = HashMap::new();

    for artefact in artefacts {
        let projected = artefact
            .canonical_kind
            .as_deref()
            .and_then(CanonicalKindProjection::from_str);
        if projected.is_some_and(|kind| {
            matches!(
                kind,
                CanonicalKindProjection::Function | CanonicalKindProjection::Method
            )
        }) {
            callable_name_to_fqn
                .entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }
        if projected.is_some_and(|kind| {
            matches!(
                kind,
                CanonicalKindProjection::Type
                    | CanonicalKindProjection::Interface
                    | CanonicalKindProjection::Enum
            )
        }) {
            type_targets
                .entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }
        if let Some(parent) = artefact.parent_symbol_fqn.as_ref() {
            method_targets_by_type_and_name
                .entry((parent.clone(), artefact.name.clone()))
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }
    }

    let imported_package_refs = collect_go_imported_package_refs(root, content);
    let mut edges = Vec::new();
    let mut seen_imports = HashSet::new();
    let mut seen_calls = HashSet::new();
    let mut seen_refs = HashSet::new();
    let mut seen_extends = HashSet::new();
    let mut traversal = GoTraversalCtx {
        content,
        path,
        package_name: package_name.as_deref(),
        callable_name_to_fqn: &callable_name_to_fqn,
        method_targets_by_type_and_name: &method_targets_by_type_and_name,
        imported_package_refs: &imported_package_refs,
        type_targets: &type_targets,
        artefacts,
        out: &mut edges,
        seen_imports: &mut seen_imports,
        seen_calls: &mut seen_calls,
        seen_refs: &mut seen_refs,
        seen_extends: &mut seen_extends,
    };
    collect_go_edges_recursive(root, &mut traversal);
    Ok(edges)
}

fn collect_go_edges_recursive(node: Node<'_>, traversal: &mut GoTraversalCtx<'_>) {
    match node.kind() {
        "import_spec" => collect_go_import_edge(node, traversal),
        "call_expression" => collect_go_call_edge(node, traversal),
        "qualified_type" => collect_go_qualified_type_reference(node, traversal),
        "type_identifier" => collect_go_type_identifier_reference(node, traversal),
        "struct_type" => collect_go_struct_embedding_edges(node, traversal),
        "interface_type" => collect_go_interface_embedding_edges(node, traversal),
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_go_edges_recursive(child, traversal);
    }
}

fn collect_go_import_edge(node: Node<'_>, traversal: &mut GoTraversalCtx<'_>) {
    let Some(path_node) = node.child_by_field_name("path") else {
        return;
    };
    let Some(import_path) = trimmed_node_text(path_node, traversal.content)
        .map(|text| strip_string_literal_delimiters(&text))
    else {
        return;
    };
    let line_no = node.start_position().row as i32 + 1;
    let key = format!("{}|{}", traversal.path, import_path);
    if !traversal.seen_imports.insert(key) {
        return;
    }
    traversal.out.push(DependencyEdge {
        edge_kind: EdgeKind::Imports,
        from_symbol_fqn: traversal.path.to_string(),
        to_target_symbol_fqn: None,
        to_symbol_ref: Some(import_path),
        start_line: Some(line_no),
        end_line: Some(node.end_position().row as i32 + 1),
        metadata: EdgeMetadata::import(ImportForm::Binding),
    });
}

fn collect_go_call_edge(node: Node<'_>, traversal: &mut GoTraversalCtx<'_>) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_symbol(line_no, traversal.artefacts) else {
        return;
    };
    let Some(function_node) = node.child_by_field_name("function") else {
        return;
    };

    match function_node.kind() {
        "identifier" => {
            let Some(name) = trimmed_node_text(function_node, traversal.content) else {
                return;
            };
            let (to_target_symbol_fqn, to_symbol_ref, resolution) =
                if let Some(target_fqn) = traversal.callable_name_to_fqn.get(&name) {
                    (Some(target_fqn.clone()), None, Resolution::Local)
                } else {
                    (
                        None,
                        Some(package_symbol_ref(
                            traversal.package_name,
                            traversal.path,
                            &name,
                        )),
                        Resolution::Unresolved,
                    )
                };
            push_go_call_edge(
                traversal,
                owner.symbol_fqn,
                line_no,
                CallForm::Function,
                to_target_symbol_fqn,
                to_symbol_ref,
                resolution,
            );
        }
        "selector_expression" => {
            let Some(field_node) = function_node.child_by_field_name("field") else {
                return;
            };
            let Some(field_name) = trimmed_node_text(field_node, traversal.content) else {
                return;
            };
            let Some(operand_node) = function_node.child_by_field_name("operand") else {
                return;
            };
            let operand_name = trimmed_node_text(operand_node, traversal.content);

            if let Some(operand_name) = operand_name.as_ref()
                && let Some(import_path) = traversal.imported_package_refs.get(operand_name)
            {
                push_go_call_edge(
                    traversal,
                    owner.symbol_fqn,
                    line_no,
                    CallForm::Associated,
                    None,
                    Some(format!("{import_path}::{field_name}")),
                    Resolution::Import,
                );
                return;
            }

            let receiver_type = selector_receiver_type_name(operand_node, traversal.content);
            if let Some(receiver_type) = receiver_type
                && let Some(type_fqn) = traversal.type_targets.get(&receiver_type)
                && let Some(target_fqn) = traversal
                    .method_targets_by_type_and_name
                    .get(&(type_fqn.clone(), field_name.clone()))
            {
                push_go_call_edge(
                    traversal,
                    owner.symbol_fqn,
                    line_no,
                    CallForm::Method,
                    Some(target_fqn.clone()),
                    None,
                    Resolution::Local,
                );
                return;
            }

            push_go_call_edge(
                traversal,
                owner.symbol_fqn,
                line_no,
                CallForm::Member,
                None,
                Some(package_member_symbol_ref(
                    traversal.package_name,
                    traversal.path,
                    operand_name.as_deref(),
                    &field_name,
                )),
                Resolution::Unresolved,
            );
        }
        _ => {}
    }
}

fn push_go_call_edge(
    traversal: &mut GoTraversalCtx<'_>,
    from_symbol_fqn: String,
    line_no: i32,
    call_form: CallForm,
    to_target_symbol_fqn: Option<String>,
    to_symbol_ref: Option<String>,
    resolution: Resolution,
) {
    let key = format!(
        "{}|{}|{}|{}|{}|{}",
        from_symbol_fqn,
        to_target_symbol_fqn.as_deref().unwrap_or(""),
        to_symbol_ref.as_deref().unwrap_or(""),
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
        to_symbol_ref,
        start_line: Some(line_no),
        end_line: Some(line_no),
        metadata: EdgeMetadata::call(call_form, resolution),
    });
}

fn collect_go_qualified_type_reference(node: Node<'_>, traversal: &mut GoTraversalCtx<'_>) {
    let Some(owner) =
        smallest_enclosing_symbol(node.start_position().row as i32 + 1, traversal.artefacts)
    else {
        return;
    };
    let Some(package_node) = node.child_by_field_name("package") else {
        return;
    };
    let Some(package_name) = trimmed_node_text(package_node, traversal.content) else {
        return;
    };
    let Some(type_node) = node.child_by_field_name("name") else {
        return;
    };
    let Some(type_name) = trimmed_node_text(type_node, traversal.content) else {
        return;
    };
    let Some(import_path) = traversal.imported_package_refs.get(&package_name) else {
        return;
    };
    let key = format!(
        "{}|{}|{}|{}",
        owner.symbol_fqn,
        import_path,
        type_name,
        node.start_position().row
    );
    if !traversal.seen_refs.insert(key) {
        return;
    }
    traversal.out.push(DependencyEdge {
        edge_kind: EdgeKind::References,
        from_symbol_fqn: owner.symbol_fqn,
        to_target_symbol_fqn: None,
        to_symbol_ref: Some(format!("{import_path}::{type_name}")),
        start_line: Some(node.start_position().row as i32 + 1),
        end_line: Some(node.end_position().row as i32 + 1),
        metadata: EdgeMetadata::reference(RefKind::Type, Resolution::Import),
    });
}

fn collect_go_type_identifier_reference(node: Node<'_>, traversal: &mut GoTraversalCtx<'_>) {
    if !is_type_identifier_reference(node) {
        return;
    }
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_symbol(line_no, traversal.artefacts) else {
        return;
    };
    let Some(name) = trimmed_node_text(node, traversal.content) else {
        return;
    };
    push_reference_edge(
        &mut EdgeCollector {
            out: traversal.out,
            seen: traversal.seen_refs,
        },
        &owner.symbol_fqn,
        &name,
        line_no,
        RefKind::Type,
        &SymbolLookup {
            local_targets: traversal.type_targets,
            imported_symbol_refs: None,
        },
    );
}

fn collect_go_struct_embedding_edges(node: Node<'_>, traversal: &mut GoTraversalCtx<'_>) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_type(line_no, traversal.artefacts) else {
        return;
    };
    let Some(field_list) = node.named_child(0) else {
        return;
    };
    let mut cursor = field_list.walk();
    for field in field_list.named_children(&mut cursor) {
        if field.kind() != "field_declaration" || has_named_field(field) {
            continue;
        }
        if let Some(type_node) = field.child_by_field_name("type") {
            push_go_embedding_edge(owner.symbol_fqn.clone(), type_node, traversal);
        }
    }
}

fn collect_go_interface_embedding_edges(node: Node<'_>, traversal: &mut GoTraversalCtx<'_>) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_type(line_no, traversal.artefacts) else {
        return;
    };
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "type_elem"
            && let Some(type_node) = child.named_child(0)
        {
            push_go_embedding_edge(owner.symbol_fqn.clone(), type_node, traversal);
        }
    }
}

fn push_go_embedding_edge(
    from_symbol_fqn: String,
    type_node: Node<'_>,
    traversal: &mut GoTraversalCtx<'_>,
) {
    let line_no = type_node.start_position().row as i32 + 1;
    match type_node.kind() {
        "qualified_type" => {
            let Some(package_node) = type_node.child_by_field_name("package") else {
                return;
            };
            let Some(package_name) = trimmed_node_text(package_node, traversal.content) else {
                return;
            };
            let Some(type_name_node) = type_node.child_by_field_name("name") else {
                return;
            };
            let Some(type_name) = trimmed_node_text(type_name_node, traversal.content) else {
                return;
            };
            let Some(import_path) = traversal.imported_package_refs.get(&package_name) else {
                return;
            };
            let key = format!("{from_symbol_fqn}|import|{import_path}|{type_name}");
            if !traversal.seen_extends.insert(key) {
                return;
            }
            traversal.out.push(DependencyEdge {
                edge_kind: EdgeKind::Extends,
                from_symbol_fqn,
                to_target_symbol_fqn: None,
                to_symbol_ref: Some(format!("{import_path}::{type_name}")),
                start_line: Some(line_no),
                end_line: Some(line_no),
                metadata: EdgeMetadata::none(),
            });
        }
        _ => {
            let Some(type_name) = type_name_from_node(type_node, traversal.content) else {
                return;
            };
            let dedupe_key = format!("{from_symbol_fqn}|local|{type_name}");
            if !traversal.seen_extends.insert(dedupe_key) {
                return;
            }
            push_extends_edge(
                &mut EdgeCollector {
                    out: traversal.out,
                    seen: traversal.seen_extends,
                },
                &from_symbol_fqn,
                &type_name,
                line_no,
                &SymbolLookup {
                    local_targets: traversal.type_targets,
                    imported_symbol_refs: None,
                },
            );
        }
    }
}

fn collect_go_imported_package_refs(root: Node<'_>, content: &str) -> HashMap<String, String> {
    let mut imports = HashMap::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "import_spec"
            && let Some(path_node) = node.child_by_field_name("path")
            && let Some(import_path) = trimmed_node_text(path_node, content)
        {
            let import_path = strip_string_literal_delimiters(&import_path);
            let alias = node
                .child_by_field_name("name")
                .and_then(|name_node| trimmed_node_text(name_node, content))
                .unwrap_or_else(|| import_path_stem(&import_path));
            if alias != "." && alias != "_" {
                imports.entry(alias).or_insert(import_path);
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    imports
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
            matches!(
                artefact.canonical_kind.as_deref(),
                Some("type") | Some("interface")
            )
        })
        .min_by_key(|artefact| artefact.end_line - artefact.start_line)
        .cloned()
}

fn selector_receiver_type_name(node: Node<'_>, content: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" | "field_identifier" => trimmed_node_text(node, content),
        "parenthesized_expression" => node
            .named_child(0)
            .and_then(|child| selector_receiver_type_name(child, content)),
        "unary_expression" => node
            .named_child(0)
            .and_then(|child| selector_receiver_type_name(child, content)),
        "composite_literal" => node
            .child_by_field_name("type")
            .and_then(|type_node| type_name_from_node(type_node, content)),
        "selector_expression" => node
            .child_by_field_name("field")
            .and_then(|field_node| trimmed_node_text(field_node, content)),
        _ => None,
    }
}

fn has_named_field(field_declaration: Node<'_>) -> bool {
    let mut cursor = field_declaration.walk();
    field_declaration
        .children_by_field_name("name", &mut cursor)
        .next()
        .is_some()
}

fn is_type_identifier_reference(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if matches!(parent.kind(), "type_spec" | "type_alias" | "qualified_type") {
        return false;
    }
    if let Some(name_node) = parent.child_by_field_name("name")
        && name_node.start_byte() == node.start_byte()
        && name_node.end_byte() == node.end_byte()
    {
        return false;
    }
    matches!(
        parent.kind(),
        "parameter_declaration"
            | "variadic_parameter_declaration"
            | "field_declaration"
            | "type_elem"
            | "type_instantiation_expression"
            | "generic_type"
            | "pointer_type"
            | "slice_type"
            | "array_type"
            | "map_type"
            | "channel_type"
            | "parenthesized_type"
            | "function_declaration"
            | "method_declaration"
            | "type_conversion_expression"
            | "type_assertion_expression"
            | "var_spec"
            | "const_spec"
    )
}

fn package_symbol_ref(package_name: Option<&str>, path: &str, name: &str) -> String {
    package_name
        .map(|package_name| format!("package::{package_name}::{name}"))
        .unwrap_or_else(|| format!("{path}::{name}"))
}

fn package_member_symbol_ref(
    package_name: Option<&str>,
    path: &str,
    operand_name: Option<&str>,
    field_name: &str,
) -> String {
    if let Some(package_name) = package_name {
        if let Some(operand_name) = operand_name {
            return format!("package::{package_name}::{operand_name}::{field_name}");
        }
        return format!("package::{package_name}::member::{field_name}");
    }
    if let Some(operand_name) = operand_name {
        return format!("{path}::{operand_name}::{field_name}");
    }
    format!("{path}::member::{field_name}")
}
