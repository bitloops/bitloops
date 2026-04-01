use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};

use crate::host::devql::{
    CallForm, CanonicalKindProjection, EdgeKind, ImportForm, RefKind, Resolution,
};
use crate::host::language_adapter::{
    DependencyEdge, EdgeMetadata, LanguageArtefact, LanguageKind, PythonKind,
    edges_shared::{
        CallCtx, EdgeCollector, ReferenceCtx, SymbolLookup, push_extends_edge, push_reference_edge,
        smallest_enclosing_callable, symbol_lookup_name_from_node,
    },
};

pub(crate) fn extract_python_dependency_edges(
    content: &str,
    path: &str,
    artefacts: &[LanguageArtefact],
) -> Result<Vec<DependencyEdge>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    parser
        .set_language(&lang)
        .context("setting tree-sitter python language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let mut edges = Vec::new();
    let callables = artefacts
        .iter()
        .filter(|artefact| is_python_callable_artefact(artefact))
        .cloned()
        .collect::<Vec<_>>();
    let mut callable_name_to_fqn = HashMap::new();
    let mut method_targets_by_parent_and_name = HashMap::new();
    for callable in &callables {
        callable_name_to_fqn
            .entry(callable.name.clone())
            .or_insert_with(|| callable.symbol_fqn.clone());
        if let Some(parent) = callable.parent_symbol_fqn.as_ref() {
            method_targets_by_parent_and_name
                .entry((parent.clone(), callable.name.clone()))
                .or_insert_with(|| callable.symbol_fqn.clone());
        }
    }

    let mut imported_symbol_refs = HashMap::new();
    collect_python_imported_symbol_refs(root, content, &mut imported_symbol_refs);

    let (type_targets, value_targets) = python_reference_target_maps(artefacts);
    let call_ctx = CallCtx {
        callables: &callables,
        callable_name_to_fqn: &callable_name_to_fqn,
        imported_symbol_refs: &imported_symbol_refs,
    };
    let reference_ctx = ReferenceCtx {
        callables: &callables,
        type_targets: &type_targets,
        value_targets: &value_targets,
        imported_symbol_refs: &imported_symbol_refs,
    };

    let mut seen_calls = HashSet::new();
    let mut seen_references = HashSet::new();
    let mut seen_extends = HashSet::new();
    let mut traversal = PythonEdgeTraversalCtx {
        content,
        path,
        call_ctx: &call_ctx,
        reference_ctx: &reference_ctx,
        method_targets_by_parent_and_name: &method_targets_by_parent_and_name,
        out: &mut edges,
        seen_calls: &mut seen_calls,
        seen_references: &mut seen_references,
        seen_extends: &mut seen_extends,
    };

    collect_python_edges_recursive(root, &mut traversal);

    Ok(edges)
}

fn collect_python_edges_recursive(
    node: tree_sitter::Node,
    traversal: &mut PythonEdgeTraversalCtx<'_>,
) {
    let line_no = node.start_position().row as i32 + 1;

    match node.kind() {
        "import_statement" | "import_from_statement" | "future_import_statement" => {
            for import_ref in python_import_refs(node, traversal.content) {
                traversal.out.push(DependencyEdge {
                    edge_kind: EdgeKind::Imports,
                    from_symbol_fqn: traversal.path.to_string(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(import_ref),
                    start_line: Some(line_no),
                    end_line: Some(node.end_position().row as i32 + 1),
                    metadata: EdgeMetadata::import(ImportForm::Binding),
                });
            }
        }
        "call" => collect_python_call_edge(
            node,
            traversal.content,
            traversal.path,
            traversal.call_ctx,
            traversal.method_targets_by_parent_and_name,
            &mut EdgeCollector {
                out: traversal.out,
                seen: traversal.seen_calls,
            },
        ),
        "identifier" => collect_python_reference_edge(
            node,
            traversal.content,
            traversal.reference_ctx,
            &mut EdgeCollector {
                out: traversal.out,
                seen: traversal.seen_references,
            },
        ),
        "class_definition" => collect_python_extends_edges(
            node,
            traversal.content,
            traversal.reference_ctx,
            &mut EdgeCollector {
                out: traversal.out,
                seen: traversal.seen_extends,
            },
        ),
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_python_edges_recursive(child, traversal);
    }
}

fn collect_python_call_edge(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    ctx: &CallCtx<'_>,
    method_targets_by_parent_and_name: &HashMap<(String, String), String>,
    collector: &mut EdgeCollector<'_>,
) {
    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_callable(line_no, ctx.callables) else {
        return;
    };
    let Some(function_node) = node.child_by_field_name("function") else {
        return;
    };

    match function_node.kind() {
        "identifier" => {
            let Ok(name) = function_node.utf8_text(content.as_bytes()) else {
                return;
            };
            push_python_call_edge(
                collector,
                ctx,
                PythonCallDescriptor {
                    from_symbol_fqn: owner.symbol_fqn.clone(),
                    target_name: name.trim().to_string(),
                    call_form: CallForm::Function,
                    line_no,
                    local_override: None,
                    path: path.to_string(),
                },
            );
        }
        "attribute" => {
            let Some(attribute_node) = function_node.child_by_field_name("attribute") else {
                return;
            };
            let Ok(name) = attribute_node.utf8_text(content.as_bytes()) else {
                return;
            };
            let name = name.trim();
            if name.is_empty() {
                return;
            }

            let target = if let Some(object_node) = function_node.child_by_field_name("object")
                && let Ok(object_text) = object_node.utf8_text(content.as_bytes())
            {
                let object_text = object_text.trim();
                if matches!(object_text, "self" | "cls") {
                    owner
                        .parent_symbol_fqn
                        .as_ref()
                        .and_then(|parent| {
                            method_targets_by_parent_and_name
                                .get(&(parent.clone(), name.to_string()))
                        })
                        .cloned()
                } else {
                    None
                }
            } else {
                None
            };

            push_python_call_edge(
                collector,
                ctx,
                PythonCallDescriptor {
                    from_symbol_fqn: owner.symbol_fqn.clone(),
                    target_name: name.to_string(),
                    call_form: CallForm::Method,
                    line_no,
                    local_override: target,
                    path: path.to_string(),
                },
            );
        }
        _ => {}
    }
}

fn push_python_call_edge(
    collector: &mut EdgeCollector<'_>,
    ctx: &CallCtx<'_>,
    descriptor: PythonCallDescriptor,
) {
    let PythonCallDescriptor {
        from_symbol_fqn,
        target_name,
        call_form,
        line_no,
        local_override,
        path,
    } = descriptor;
    let target_name = target_name.trim();
    if target_name.is_empty() {
        return;
    }

    let (to_target_symbol_fqn, to_symbol_ref, resolution) = if let Some(target_fqn) = local_override
    {
        (Some(target_fqn), None, Resolution::Local)
    } else if let Some(target_fqn) = ctx.callable_name_to_fqn.get(target_name) {
        (Some(target_fqn.clone()), None, Resolution::Local)
    } else if let Some(symbol_ref) = ctx.imported_symbol_refs.get(target_name) {
        (None, Some(symbol_ref.clone()), Resolution::Import)
    } else {
        (
            None,
            Some(format!("{path}::{target_name}")),
            Resolution::Unresolved,
        )
    };

    let key = format!(
        "{}|{}|{}|{}|{}|{}",
        from_symbol_fqn.as_str(),
        to_target_symbol_fqn.as_deref().unwrap_or(""),
        to_symbol_ref.as_deref().unwrap_or(""),
        line_no,
        call_form.as_str(),
        resolution.as_str()
    );
    if !collector.seen.insert(key) {
        return;
    }

    collector.out.push(DependencyEdge {
        edge_kind: EdgeKind::Calls,
        from_symbol_fqn,
        to_target_symbol_fqn,
        to_symbol_ref,
        start_line: Some(line_no),
        end_line: Some(line_no),
        metadata: EdgeMetadata::call(call_form, resolution),
    });
}

struct PythonEdgeTraversalCtx<'a> {
    content: &'a str,
    path: &'a str,
    call_ctx: &'a CallCtx<'a>,
    reference_ctx: &'a ReferenceCtx<'a>,
    method_targets_by_parent_and_name: &'a HashMap<(String, String), String>,
    out: &'a mut Vec<DependencyEdge>,
    seen_calls: &'a mut HashSet<String>,
    seen_references: &'a mut HashSet<String>,
    seen_extends: &'a mut HashSet<String>,
}

struct PythonCallDescriptor {
    from_symbol_fqn: String,
    target_name: String,
    call_form: CallForm,
    line_no: i32,
    local_override: Option<String>,
    path: String,
}

fn collect_python_reference_edge(
    node: tree_sitter::Node,
    content: &str,
    ctx: &ReferenceCtx<'_>,
    collector: &mut EdgeCollector<'_>,
) {
    if !python_identifier_is_value_reference(node) {
        return;
    }

    let line_no = node.start_position().row as i32 + 1;
    let Some(owner) = smallest_enclosing_callable(line_no, ctx.callables) else {
        return;
    };
    let Ok(name) = node.utf8_text(content.as_bytes()) else {
        return;
    };
    push_reference_edge(
        collector,
        &owner.symbol_fqn,
        name,
        line_no,
        RefKind::Value,
        &SymbolLookup {
            local_targets: ctx.value_targets,
            imported_symbol_refs: Some(ctx.imported_symbol_refs),
        },
    );
}

fn collect_python_extends_edges(
    node: tree_sitter::Node,
    content: &str,
    ctx: &ReferenceCtx<'_>,
    collector: &mut EdgeCollector<'_>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Ok(owner_name) = name_node.utf8_text(content.as_bytes()) else {
        return;
    };
    let owner_name = owner_name.trim();
    let Some(owner_fqn) = ctx.type_targets.get(owner_name) else {
        return;
    };
    let Some(superclasses) = node.child_by_field_name("superclasses") else {
        return;
    };

    let mut cursor = superclasses.walk();
    for child in superclasses.named_children(&mut cursor) {
        if let Some(target_name) = symbol_lookup_name_from_node(child, content) {
            let line_no = child.start_position().row as i32 + 1;
            let lookup = SymbolLookup {
                local_targets: ctx.type_targets,
                imported_symbol_refs: Some(ctx.imported_symbol_refs),
            };
            if ctx.type_targets.contains_key(target_name.as_str())
                || ctx.imported_symbol_refs.contains_key(target_name.as_str())
            {
                push_extends_edge(collector, owner_fqn, &target_name, line_no, &lookup);
                continue;
            }

            let key = format!("{owner_fqn}|{target_name}|{line_no}|unresolved");
            if !collector.seen.insert(key) {
                continue;
            }

            collector.out.push(DependencyEdge {
                edge_kind: EdgeKind::Extends,
                from_symbol_fqn: owner_fqn.clone(),
                to_target_symbol_fqn: None,
                to_symbol_ref: Some(target_name),
                start_line: Some(line_no),
                end_line: Some(line_no),
                metadata: EdgeMetadata::none(),
            });
        }
    }
}

fn collect_python_imported_symbol_refs(
    node: tree_sitter::Node,
    content: &str,
    imported_symbol_refs: &mut HashMap<String, String>,
) {
    match node.kind() {
        "import_statement" => {
            let mut cursor = node.walk();
            for child in node.children_by_field_name("name", &mut cursor) {
                match child.kind() {
                    "aliased_import" => {
                        let Some(alias) = child.child_by_field_name("alias") else {
                            continue;
                        };
                        let Some(name) = child.child_by_field_name("name") else {
                            continue;
                        };
                        let Ok(alias_text) = alias.utf8_text(content.as_bytes()) else {
                            continue;
                        };
                        let Ok(name_text) = name.utf8_text(content.as_bytes()) else {
                            continue;
                        };
                        imported_symbol_refs
                            .insert(alias_text.trim().to_string(), name_text.trim().to_string());
                    }
                    "dotted_name" => {
                        let Ok(name_text) = child.utf8_text(content.as_bytes()) else {
                            continue;
                        };
                        let import_ref = name_text.trim();
                        if import_ref.is_empty() {
                            continue;
                        }
                        let binding_name =
                            import_ref.split('.').next().unwrap_or(import_ref).trim();
                        if !binding_name.is_empty() {
                            imported_symbol_refs
                                .entry(binding_name.to_string())
                                .or_insert_with(|| import_ref.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        "import_from_statement" | "future_import_statement" => {
            let module_ref = if node.kind() == "future_import_statement" {
                "__future__".to_string()
            } else {
                let Some(module_name) = node.child_by_field_name("module_name") else {
                    return;
                };
                let Ok(module_ref) = module_name.utf8_text(content.as_bytes()) else {
                    return;
                };
                module_ref.trim().to_string()
            };
            let module_ref = module_ref.trim();
            if module_ref.is_empty() {
                return;
            }
            let mut cursor = node.walk();
            for child in node.children_by_field_name("name", &mut cursor) {
                match child.kind() {
                    "aliased_import" => {
                        let Some(alias) = child.child_by_field_name("alias") else {
                            continue;
                        };
                        let Some(name) = child.child_by_field_name("name") else {
                            continue;
                        };
                        let Ok(alias_text) = alias.utf8_text(content.as_bytes()) else {
                            continue;
                        };
                        let Ok(name_text) = name.utf8_text(content.as_bytes()) else {
                            continue;
                        };
                        imported_symbol_refs.insert(
                            alias_text.trim().to_string(),
                            format!("{module_ref}::{}", name_text.trim()),
                        );
                    }
                    "dotted_name" => {
                        let Ok(name_text) = child.utf8_text(content.as_bytes()) else {
                            continue;
                        };
                        let symbol_name = name_text.trim();
                        if symbol_name.is_empty() {
                            continue;
                        }
                        let binding_name = symbol_name
                            .split('.')
                            .next_back()
                            .unwrap_or(symbol_name)
                            .trim();
                        imported_symbol_refs.insert(
                            binding_name.to_string(),
                            format!("{module_ref}::{symbol_name}"),
                        );
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_python_imported_symbol_refs(child, content, imported_symbol_refs);
    }
}

fn python_import_refs(node: tree_sitter::Node, content: &str) -> Vec<String> {
    let mut refs = Vec::new();
    match node.kind() {
        "import_statement" => {
            let mut cursor = node.walk();
            for child in node.children_by_field_name("name", &mut cursor) {
                let value = if child.kind() == "aliased_import" {
                    child
                        .child_by_field_name("name")
                        .and_then(|name| name.utf8_text(content.as_bytes()).ok())
                } else {
                    child.utf8_text(content.as_bytes()).ok()
                };
                if let Some(value) = value {
                    let value = value.trim();
                    if !value.is_empty() {
                        refs.push(value.to_string());
                    }
                }
            }
        }
        "import_from_statement" | "future_import_statement" => {
            let module_ref = if node.kind() == "future_import_statement" {
                "__future__".to_string()
            } else {
                let Some(module_name) = node.child_by_field_name("module_name") else {
                    return refs;
                };
                let Ok(module_ref) = module_name.utf8_text(content.as_bytes()) else {
                    return refs;
                };
                module_ref.trim().to_string()
            };
            let module_ref = module_ref.trim();
            if !module_ref.is_empty() {
                refs.push(module_ref.to_string());
            }
        }
        _ => {}
    }
    refs
}

fn python_reference_target_maps(
    artefacts: &[LanguageArtefact],
) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut type_targets = HashMap::new();
    let mut value_targets = HashMap::new();

    for artefact in artefacts {
        let projected_kind = artefact
            .canonical_kind
            .as_deref()
            .and_then(CanonicalKindProjection::from_str);

        if projected_kind == Some(CanonicalKindProjection::Type)
            || artefact.language_kind == LanguageKind::python(PythonKind::ClassDefinition)
        {
            type_targets
                .entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
            value_targets
                .entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }

        if matches!(
            projected_kind,
            Some(CanonicalKindProjection::Variable)
                | Some(CanonicalKindProjection::Function)
                | Some(CanonicalKindProjection::Method)
        ) {
            value_targets
                .entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }
    }

    (type_targets, value_targets)
}

fn is_python_callable_artefact(artefact: &LanguageArtefact) -> bool {
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
}

fn python_identifier_is_value_reference(node: tree_sitter::Node) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };

    match parent.kind() {
        "function_definition" | "class_definition" | "parameters" | "keyword_argument" => false,
        "assignment" => !node_matches_parent_field(node, parent, "left"),
        "call" => !node_matches_parent_field(node, parent, "function"),
        "attribute" => !node_matches_parent_field(node, parent, "attribute"),
        "import_statement"
        | "import_from_statement"
        | "future_import_statement"
        | "aliased_import" => false,
        "typed_parameter"
        | "typed_default_parameter"
        | "default_parameter"
        | "lambda_parameters"
        | "for_in_clause"
        | "with_item" => !node_matches_parent_field(node, parent, "value"),
        _ => true,
    }
}

fn node_matches_parent_field(
    node: tree_sitter::Node,
    parent: tree_sitter::Node,
    field: &str,
) -> bool {
    let mut cursor = parent.walk();
    for child in parent.children_by_field_name(field, &mut cursor) {
        if child == node {
            return true;
        }
    }
    false
}
