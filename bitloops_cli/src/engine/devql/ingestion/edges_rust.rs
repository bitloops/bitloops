// Rust dependency edge extraction (use imports, calls, macros, impl trait).

fn extract_rust_dependency_edges(
    content: &str,
    path: &str,
    artefacts: &[JsTsArtefact],
) -> Result<Vec<JsTsDependencyEdge>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
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
        .filter(|a| matches!(a.canonical_kind.as_deref(), Some("function") | Some("method")))
        .cloned()
        .collect::<Vec<_>>();
    let mut name_to_fqn = HashMap::new();
    for c in &rust_callables {
        name_to_fqn
            .entry(c.name.clone())
            .or_insert_with(|| c.symbol_fqn.clone());
    }
    let mut imported_symbol_refs = HashMap::new();
    collect_rust_imported_symbol_refs(root, content, &mut imported_symbol_refs);
    let mut seen_calls = HashSet::new();
    let call_ctx = CallCtx {
        callables: &rust_callables,
        callable_name_to_fqn: &name_to_fqn,
        imported_symbol_refs: &imported_symbol_refs,
    };
    collect_rust_edges_recursive(
        root,
        content,
        path,
        &call_ctx,
        &mut EdgeCollector { out: &mut edges, seen: &mut seen_calls },
    );
    let (type_targets, value_targets) = rust_reference_target_maps(artefacts);
    let export_targets = top_level_export_target_map(artefacts);
    let mut seen_references = HashSet::new();
    let mut seen_extends = HashSet::new();
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
    collect_rust_extends_edges_recursive(
        root,
        content,
        &type_targets,
        &mut seen_extends,
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
    ctx: &CallCtx<'_>,
    ec: &mut EdgeCollector<'_>,
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
                ec.out.push(JsTsDependencyEdge {
                    edge_kind: EdgeKind::Imports.as_str().to_string(),
                    from_symbol_fqn: path.to_string(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(cleaned),
                    start_line: Some(start_line),
                    end_line: Some(node.end_position().row as i32 + 1),
                    metadata: json!({"import_form": ImportForm::Binding.as_str()}),
                });
            }
        }

    if matches!(kind, "call_expression" | "method_call_expression")
        && let Some(owner) = smallest_enclosing_callable(start_line, ctx.callables)
        && let Some((target_name, call_form)) = rust_call_target(node, content)
    {
        push_rust_call_edge(
            ec,
            &owner.symbol_fqn,
            &target_name,
            call_form,
            start_line,
            ctx,
            false,
        );
    }

    if kind == "macro_invocation"
        && let Some(owner) = smallest_enclosing_callable(start_line, ctx.callables)
        && let Some((target_name, call_form)) = rust_macro_target(node, content)
    {
        push_rust_call_edge(
            ec,
            &owner.symbol_fqn,
            &target_name,
            call_form,
            start_line,
            ctx,
            false,
        );
    }

    if kind == "impl_item"
        && let Ok(text) = node.utf8_text(content.as_bytes()) {
            let impl_re = Regex::new(r"impl\s+([A-Za-z0-9_:<>]+)\s+for\s+([A-Za-z0-9_:<>]+)");
            if let Ok(impl_re) = impl_re
                && let Some(cap) = impl_re.captures(text)
            {
                    let trait_name = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
                    if !trait_name.is_empty() {
                        ec.out.push(JsTsDependencyEdge {
                            edge_kind: EdgeKind::Implements.as_str().to_string(),
                            from_symbol_fqn: format!("{path}::impl@{start_line}"),
                            to_target_symbol_fqn: None,
                            to_symbol_ref: Some(trait_name),
                            start_line: Some(start_line),
                            end_line: Some(node.end_position().row as i32 + 1),
                            metadata: json!({}),
                        });
                    }
            }
        }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_rust_edges_recursive(child, content, path, ctx, ec);
    }
}

fn collect_rust_imported_symbol_refs(
    node: tree_sitter::Node,
    content: &str,
    imported_symbol_refs: &mut HashMap<String, String>,
) {
    if node.kind() == "use_declaration"
        && let Some(argument) = node.child_by_field_name("argument")
    {
        let mut entries = Vec::new();
        collect_rust_use_export_entries(argument, content, None, &mut entries);
        for entry in entries {
            imported_symbol_refs.entry(entry.export_name).or_insert(entry.path);
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_rust_imported_symbol_refs(child, content, imported_symbol_refs);
    }
}

fn rust_call_target(node: tree_sitter::Node, content: &str) -> Option<(String, &'static str)> {
    match node.kind() {
        "method_call_expression" => {
            let name_node = node
                .child_by_field_name("name")
                .or_else(|| node.child_by_field_name("method"))?;
            let target_name = rust_callable_name_from_text(
                name_node.utf8_text(content.as_bytes()).ok()?.trim_end_matches('!'),
            )?;
            Some((target_name, "method"))
        }
        "call_expression" => {
            let function_node = node.child_by_field_name("function")?;
            let function_text = function_node.utf8_text(content.as_bytes()).ok()?;
            let target_name = rust_callable_name_from_text(function_text)?;
            let call_form = if function_text.contains("::") {
                "associated"
            } else {
                "function"
            };
            Some((target_name, call_form))
        }
        _ => None,
    }
}

fn rust_macro_target(node: tree_sitter::Node, content: &str) -> Option<(String, &'static str)> {
    let macro_node = node.child_by_field_name("macro")?;
    let target_name = rust_callable_name_from_text(
        macro_node
            .utf8_text(content.as_bytes())
            .ok()?
            .trim()
            .trim_end_matches('!'),
    )?;
    Some((target_name, "macro"))
}

fn rust_callable_name_from_text(text: &str) -> Option<String> {
    let mut candidate = text.trim();
    if candidate.is_empty() {
        return None;
    }

    while candidate.ends_with('>') {
        let mut depth = 0usize;
        let mut start_idx = None;
        for (idx, ch) in candidate.char_indices().rev() {
            match ch {
                '>' => depth += 1,
                '<' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    if depth == 0 {
                        start_idx = Some(idx);
                        break;
                    }
                }
                _ => {}
            }
        }

        let Some(start_idx) = start_idx else {
            break;
        };
        let prefix = candidate[..start_idx].trim_end();
        candidate = prefix.strip_suffix("::").unwrap_or(prefix).trim_end();
    }

    let candidate = candidate
        .trim_end_matches('!')
        .trim_end_matches('?')
        .trim();
    let tail = candidate
        .rsplit('.')
        .next()
        .unwrap_or(candidate)
        .rsplit("::")
        .next()
        .unwrap_or(candidate)
        .trim();
    let first = tail.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if !tail.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return None;
    }

    Some(tail.to_string())
}

fn push_rust_call_edge(
    ec: &mut EdgeCollector<'_>,
    from_symbol_fqn: &str,
    target_name: &str,
    call_form: &str,
    line_no: i32,
    ctx: &CallCtx<'_>,
    allow_unresolved: bool,
) {
    let resolution = if let Some(target_fqn) = ctx.callable_name_to_fqn.get(target_name) {
        let key = format!("{from_symbol_fqn}|{target_fqn}|{line_no}|local|{call_form}");
        if !ec.seen.insert(key) {
            return;
        }
        ec.out.push(JsTsDependencyEdge {
            edge_kind: EdgeKind::Calls.as_str().to_string(),
            from_symbol_fqn: from_symbol_fqn.to_string(),
            to_target_symbol_fqn: Some(target_fqn.clone()),
            to_symbol_ref: None,
            start_line: Some(line_no),
            end_line: Some(line_no),
            metadata: json!({"call_form": call_form, "resolution": Resolution::Local.as_str()}),
        });
        return;
    } else if let Some(import_ref) = ctx.imported_symbol_refs.get(target_name) {
        ("import", Some(import_ref.clone()))
    } else if allow_unresolved {
        ("unresolved", Some(format!("{from_symbol_fqn}::{target_name}")))
    } else {
        return;
    };

    let (resolution, to_symbol_ref) = resolution;
    let Some(to_symbol_ref) = to_symbol_ref else {
        return;
    };
    let key = format!("{from_symbol_fqn}|{to_symbol_ref}|{line_no}|{resolution}|{call_form}");
    if !ec.seen.insert(key) {
        return;
    }
    ec.out.push(JsTsDependencyEdge {
        edge_kind: EdgeKind::Calls.as_str().to_string(),
        from_symbol_fqn: from_symbol_fqn.to_string(),
        to_target_symbol_fqn: None,
        to_symbol_ref: Some(to_symbol_ref),
        start_line: Some(line_no),
        end_line: Some(line_no),
        metadata: json!({"call_form": call_form, "resolution": resolution}),
    });
}
