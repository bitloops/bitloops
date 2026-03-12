// Rust dependency edge extraction (use imports, calls, macros, impl trait).

fn extract_rust_dependency_edges(
    content: &str,
    path: &str,
    artefacts: &[JsTsArtefact],
) -> Result<Vec<JsTsDependencyEdge>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::language();
    parser
        .set_language(&lang)
        .context("setting tree-sitter rust language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let mut edges = Vec::new();
    let rust_callables = artefacts
        .iter()
        .filter(|a| a.canonical_kind == "function" || a.canonical_kind == "method")
        .cloned()
        .collect::<Vec<_>>();
    let mut name_to_fqn = HashMap::new();
    for c in &rust_callables {
        name_to_fqn
            .entry(c.name.clone())
            .or_insert_with(|| c.symbol_fqn.clone());
    }

    collect_rust_edges_recursive(
        root,
        content,
        path,
        &rust_callables,
        &name_to_fqn,
        &mut edges,
    );
    let (type_targets, value_targets) = rust_reference_target_maps(artefacts);
    let export_targets = top_level_export_target_map(artefacts);
    let mut seen_references = HashSet::new();
    let mut seen_inherits = HashSet::new();
    let mut seen_exports = HashSet::new();
    let empty_imports = HashMap::new();
    collect_rust_reference_edges_recursive(
        root,
        content,
        &ReferenceCtx {
            callables: &rust_callables,
            type_targets: &type_targets,
            value_targets: &value_targets,
            imported_symbol_refs: &empty_imports,
        },
        &mut EdgeCollector { out: &mut edges, seen: &mut seen_references },
    );
    collect_rust_inherits_edges_recursive(
        root,
        content,
        &type_targets,
        &mut seen_inherits,
        &mut edges,
    );
    collect_rust_export_edges_recursive(
        root,
        content,
        path,
        &export_targets,
        &mut seen_exports,
        &mut edges,
    );
    Ok(edges)
}

fn collect_rust_edges_recursive(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    rust_callables: &[JsTsArtefact],
    callable_name_to_fqn: &HashMap<String, String>,
    out: &mut Vec<JsTsDependencyEdge>,
) {
    let kind = node.kind();
    let start_line = node.start_position().row as i32 + 1;

    if kind == "use_declaration"
        && let Ok(text) = node.utf8_text(content.as_bytes()) {
            let cleaned = text
                .trim()
                .trim_start_matches("use")
                .trim()
                .trim_end_matches(';')
                .trim()
                .to_string();
            if !cleaned.is_empty() {
                out.push(JsTsDependencyEdge {
                    edge_kind: "imports".to_string(),
                    from_symbol_fqn: path.to_string(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(cleaned),
                    start_line: Some(start_line),
                    end_line: Some(node.end_position().row as i32 + 1),
                    metadata: json!({"import_form":"use"}),
                });
            }
        }

    if kind == "call_expression" || kind == "method_call_expression" {
        let owner = smallest_enclosing_callable(start_line, rust_callables);
        if let Some(owner) = owner {
            let target_name = node
                .child_by_field_name("function")
                .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                .map(|s| s.split("::").last().unwrap_or(s).to_string())
                .or_else(|| {
                    node.child_by_field_name("name")
                        .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                        .map(|s| s.to_string())
                });

            if let Some(target_name) = target_name {
                if let Some(target_fqn) = callable_name_to_fqn.get(&target_name) {
                    out.push(JsTsDependencyEdge {
                        edge_kind: "calls".to_string(),
                        from_symbol_fqn: owner.symbol_fqn.clone(),
                        to_target_symbol_fqn: Some(target_fqn.clone()),
                        to_symbol_ref: None,
                        start_line: Some(start_line),
                        end_line: Some(start_line),
                        metadata: json!({"call_form":"rust","resolution":"local"}),
                    });
                } else {
                    out.push(JsTsDependencyEdge {
                        edge_kind: "calls".to_string(),
                        from_symbol_fqn: owner.symbol_fqn.clone(),
                        to_target_symbol_fqn: None,
                        to_symbol_ref: Some(format!("{path}::{target_name}")),
                        start_line: Some(start_line),
                        end_line: Some(start_line),
                        metadata: json!({"call_form":"rust","resolution":"unresolved"}),
                    });
                }
            }
        }
    }

    if kind == "macro_invocation" {
        let owner = smallest_enclosing_callable(start_line, rust_callables);
        if let Some(owner) = owner {
            let target_ref = node
                .child_by_field_name("macro")
                .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                .map(str::trim)
                .map(|name| name.trim_end_matches('!').to_string())
                .filter(|name| !name.is_empty());

            if let Some(target_ref) = target_ref {
                let to_symbol_ref = if target_ref.contains("::") {
                    target_ref
                } else {
                    format!("{path}::{target_ref}")
                };
                out.push(JsTsDependencyEdge {
                    edge_kind: "calls".to_string(),
                    from_symbol_fqn: owner.symbol_fqn.clone(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(to_symbol_ref),
                    start_line: Some(start_line),
                    end_line: Some(start_line),
                    metadata: json!({"call_form":"macro","resolution":"unresolved"}),
                });
            }
        }
    }

    if kind == "impl_item"
        && let Ok(text) = node.utf8_text(content.as_bytes()) {
            let impl_re = Regex::new(r"impl\s+([A-Za-z0-9_:<>]+)\s+for\s+([A-Za-z0-9_:<>]+)");
            if let Ok(impl_re) = impl_re
                && let Some(cap) = impl_re.captures(text)
            {
                    let trait_name = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
                    if !trait_name.is_empty() {
                        out.push(JsTsDependencyEdge {
                            edge_kind: "implements".to_string(),
                            from_symbol_fqn: format!("{path}::impl@{start_line}"),
                            to_target_symbol_fqn: None,
                            to_symbol_ref: Some(trait_name),
                            start_line: Some(start_line),
                            end_line: Some(node.end_position().row as i32 + 1),
                            metadata: json!({"relation":"trait_impl"}),
                        });
                    }
            }
        }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_rust_edges_recursive(
            child,
            content,
            path,
            rust_callables,
            callable_name_to_fqn,
            out,
        );
    }
}
