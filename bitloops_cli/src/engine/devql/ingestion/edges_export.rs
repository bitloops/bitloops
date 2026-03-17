// Export edge extraction for JS/TS and Rust.

fn push_export_edge(
    col: &mut EdgeCollector,
    from_symbol_fqn: &str,
    target: EdgeTarget,
    export_name: &str,
    export_form: &str,
    resolution: &str,
) {
    let export_name = export_name.trim();
    if export_name.is_empty() {
        return;
    }

    let key = format!(
        "{}|{}|{}|{}|{}",
        from_symbol_fqn,
        target.to_target_symbol_fqn.as_deref().unwrap_or(""),
        target.to_symbol_ref.as_deref().unwrap_or(""),
        export_name,
        export_form
    );
    if !col.seen.insert(key) {
        return;
    }

    col.out.push(JsTsDependencyEdge {
        edge_kind: "exports".to_string(),
        from_symbol_fqn: from_symbol_fqn.to_string(),
        to_target_symbol_fqn: target.to_target_symbol_fqn,
        to_symbol_ref: target.to_symbol_ref,
        start_line: None,
        end_line: None,
        metadata: json!({
            "export_name": export_name,
            "export_form": export_form,
            "resolution": resolution,
        }),
    });
}

fn resolve_js_ts_export_target(
    original_name: &str,
    source_ref: Option<&str>,
    local_targets: &HashMap<String, String>,
    imported_symbol_refs: &HashMap<String, String>,
) -> Option<(EdgeTarget, &'static str)> {
    let original_name = original_name.trim();
    if original_name.is_empty() {
        return None;
    }

    if let Some(source_ref) = source_ref {
        return Some((
            EdgeTarget {
                to_target_symbol_fqn: None,
                to_symbol_ref: Some(format!("{source_ref}::{original_name}")),
            },
            "re_export",
        ));
    }

    if let Some(target_fqn) = local_targets.get(original_name) {
        return Some((
            EdgeTarget { to_target_symbol_fqn: Some(target_fqn.clone()), to_symbol_ref: None },
            "local",
        ));
    }

    if let Some(symbol_ref) = imported_symbol_refs.get(original_name) {
        return Some((
            EdgeTarget { to_target_symbol_fqn: None, to_symbol_ref: Some(symbol_ref.clone()) },
            "import",
        ));
    }

    None
}

fn js_ts_exported_declaration_names(
    declaration: tree_sitter::Node,
    content: &str,
) -> Vec<String> {
    match declaration.kind() {
        "function_declaration"
        | "class_declaration"
        | "interface_declaration"
        | "type_alias_declaration" => declaration
            .child_by_field_name("name")
            .and_then(|node| syntax_node_trimmed_text(node, content))
            .into_iter()
            .collect(),
        "lexical_declaration" | "variable_declaration" => {
            let mut names = Vec::new();
            let mut cursor = declaration.walk();
            for child in declaration.named_children(&mut cursor) {
                if child.kind() != "variable_declarator" {
                    continue;
                }
                if let Some(name) = child
                    .child_by_field_name("name")
                    .and_then(|node| syntax_node_trimmed_text(node, content))
                {
                    names.push(name);
                }
            }
            names
        }
        _ => Vec::new(),
    }
}

fn collect_js_ts_export_edges_recursive(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    local_targets: &HashMap<String, String>,
    imported_symbol_refs: &HashMap<String, String>,
    seen: &mut HashSet<String>,
    out: &mut Vec<JsTsDependencyEdge>,
) {
    if node.kind() == "export_statement" {
        let source_ref = node
            .child_by_field_name("source")
            .and_then(|source| syntax_node_trimmed_text(source, content))
            .map(|source| strip_string_delimiters(&source));
        let export_stmt_text = syntax_node_trimmed_text(node, content).unwrap_or_default();

        if let Some(declaration) = node.child_by_field_name("declaration") {
            let export_form = if export_stmt_text.starts_with("export default") {
                "default"
            } else {
                "declaration"
            };
            for original_name in js_ts_exported_declaration_names(declaration, content) {
                let export_name = if export_form == "default" {
                    "default"
                } else {
                    original_name.as_str()
                };
                if let Some((target, resolution)) =
                    resolve_js_ts_export_target(
                        &original_name,
                        None,
                        local_targets,
                        imported_symbol_refs,
                    )
                {
                    push_export_edge(
                        &mut EdgeCollector { out, seen },
                        path,
                        target,
                        export_name,
                        export_form,
                        resolution,
                    );
                }
            }
        }

        if source_ref.is_some()
            && export_stmt_text.starts_with("export *")
            && !export_stmt_text.contains(" as ")
        {
            push_export_edge(
                &mut EdgeCollector { out, seen },
                path,
                EdgeTarget {
                    to_target_symbol_fqn: None,
                    to_symbol_ref: source_ref.as_ref().map(|source| format!("{source}::*")),
                },
                "*",
                "re_export_all",
                "re_export",
            );
        }

        if export_stmt_text.starts_with("export default")
            && let Some(value) = node.child_by_field_name("value")
            && let Some(original_name) = syntax_node_trimmed_text(value, content)
            && let Some((target, resolution)) = resolve_js_ts_export_target(
                &original_name,
                None,
                local_targets,
                imported_symbol_refs,
            )
        {
            push_export_edge(
                &mut EdgeCollector { out, seen },
                path,
                target,
                "default",
                "default",
                resolution,
            );
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "export_clause" => {
                    let mut spec_cursor = child.walk();
                    for specifier in child.named_children(&mut spec_cursor) {
                        if specifier.kind() != "export_specifier" {
                            continue;
                        }
                        let Some(original_name) = specifier
                            .child_by_field_name("name")
                            .and_then(|name| syntax_node_trimmed_text(name, content))
                            .map(|name| strip_string_delimiters(&name))
                        else {
                            continue;
                        };
                        let export_name = specifier
                            .child_by_field_name("alias")
                            .and_then(|alias| syntax_node_trimmed_text(alias, content))
                            .map(|alias| strip_string_delimiters(&alias))
                            .unwrap_or_else(|| original_name.clone());
                        let export_form = if source_ref.is_some() {
                            "re_export"
                        } else {
                            "named"
                        };

                        if let Some((target, resolution)) =
                            resolve_js_ts_export_target(
                                &original_name,
                                source_ref.as_deref(),
                                local_targets,
                                imported_symbol_refs,
                            )
                        {
                            push_export_edge(
                                &mut EdgeCollector { out, seen },
                                path,
                                target,
                                &export_name,
                                export_form,
                                resolution,
                            );
                        }
                    }
                }
                "namespace_export" => {
                    let Some(source_ref) = source_ref.as_deref() else {
                        continue;
                    };
                    let Some(export_name) = syntax_node_trimmed_text(child, content)
                        .map(|name| strip_string_delimiters(&name))
                    else {
                        continue;
                    };
                    push_export_edge(
                        &mut EdgeCollector { out, seen },
                        path,
                        EdgeTarget {
                            to_target_symbol_fqn: None,
                            to_symbol_ref: Some(format!("{source_ref}::*")),
                        },
                        &export_name,
                        "re_export_namespace",
                        "re_export",
                    );
                }
                _ => {}
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_js_ts_export_edges_recursive(
            child,
            content,
            path,
            local_targets,
            imported_symbol_refs,
            seen,
            out,
        );
    }
}

fn join_rust_use_path(prefix: Option<&str>, segment: &str) -> String {
    let segment = segment.trim();
    if segment.is_empty() {
        return prefix.unwrap_or_default().to_string();
    }

    match prefix {
        Some(prefix) if !prefix.is_empty() => {
            if segment == "self" {
                prefix.to_string()
            } else if matches!(segment, "crate" | "self" | "super") || segment.contains("::") {
                segment.to_string()
            } else {
                format!("{prefix}::{segment}")
            }
        }
        _ => segment.to_string(),
    }
}

fn rust_use_default_export_name(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    if path.ends_with("::*") {
        return Some("*".to_string());
    }
    path.rsplit("::")
        .next()
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
}

fn rust_use_is_public(node: tree_sitter::Node, content: &str) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).any(|child| {
        child.kind() == "visibility_modifier"
            && syntax_node_trimmed_text(child, content)
                .map(|text| text.starts_with("pub"))
                .unwrap_or(false)
    })
}

fn collect_rust_use_export_entries(
    node: tree_sitter::Node,
    content: &str,
    prefix: Option<&str>,
    out: &mut Vec<RustUseExportEntry>,
) {
    match node.kind() {
        "use_as_clause" => {
            let Some(path_node) = node.child_by_field_name("path") else {
                return;
            };
            let Some(alias_node) = node.child_by_field_name("alias") else {
                return;
            };
            let Some(path_text) = syntax_node_trimmed_text(path_node, content) else {
                return;
            };
            let Some(alias_text) = syntax_node_trimmed_text(alias_node, content) else {
                return;
            };
            let path = join_rust_use_path(prefix, &path_text);
            if path.is_empty() {
                return;
            }
            out.push(RustUseExportEntry {
                path,
                export_name: alias_text,
            });
        }
        "scoped_use_list" => {
            let next_prefix = node
                .child_by_field_name("path")
                .and_then(|path_node| syntax_node_trimmed_text(path_node, content))
                .map(|path_text| join_rust_use_path(prefix, &path_text))
                .or_else(|| prefix.map(str::to_string));
            let Some(list_node) = node.child_by_field_name("list") else {
                return;
            };
            collect_rust_use_export_entries(list_node, content, next_prefix.as_deref(), out);
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_rust_use_export_entries(child, content, prefix, out);
            }
        }
        "crate" | "identifier" | "scoped_identifier" | "self" | "super" | "use_wildcard" => {
            let Some(text) = syntax_node_trimmed_text(node, content) else {
                return;
            };
            let path = if node.kind() == "use_wildcard" {
                let Some(prefix) = prefix else {
                    return;
                };
                format!("{prefix}::*")
            } else {
                join_rust_use_path(prefix, &text)
            };
            let Some(export_name) = rust_use_default_export_name(&path) else {
                return;
            };
            out.push(RustUseExportEntry { path, export_name });
        }
        _ => {}
    }
}

fn resolve_rust_export_target(
    export_path: &str,
    local_targets: &HashMap<String, String>,
) -> Option<(EdgeTarget, &'static str)> {
    let export_path = export_path.trim();
    if export_path.is_empty() {
        return None;
    }

    let local_candidate = if export_path.starts_with("self::") || !export_path.contains("::") {
        symbol_lookup_name_from_text(export_path)
    } else {
        None
    };

    if let Some(local_name) = local_candidate
        && let Some(target_fqn) = local_targets.get(&local_name)
    {
        return Some((
            EdgeTarget { to_target_symbol_fqn: Some(target_fqn.clone()), to_symbol_ref: None },
            "local",
        ));
    }

    let resolution = if export_path.starts_with("crate::")
        || export_path.starts_with("super::")
        || export_path.contains("::")
    {
        "external"
    } else {
        "unresolved"
    };

    Some((
        EdgeTarget { to_target_symbol_fqn: None, to_symbol_ref: Some(export_path.to_string()) },
        resolution,
    ))
}

fn collect_rust_export_edges_recursive(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    local_targets: &HashMap<String, String>,
    seen: &mut HashSet<String>,
    out: &mut Vec<JsTsDependencyEdge>,
) {
    if node.kind() == "use_declaration" && rust_use_is_public(node, content)
        && let Some(argument) = node.child_by_field_name("argument") {
            let mut entries = Vec::new();
            collect_rust_use_export_entries(argument, content, None, &mut entries);
            for entry in entries {
                if let Some((target, resolution)) =
                    resolve_rust_export_target(&entry.path, local_targets)
                {
                    push_export_edge(
                        &mut EdgeCollector { out, seen },
                        path,
                        target,
                        &entry.export_name,
                        "pub_use",
                        resolution,
                    );
                }
            }
        }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_rust_export_edges_recursive(child, content, path, local_targets, seen, out);
    }
}
