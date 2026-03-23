use super::*;

// JS/TS dependency edge extraction (imports, calls, references, extends, exports).

pub(super) fn extract_js_ts_dependency_edges(
    content: &str,
    path: &str,
    artefacts: &[JsTsArtefact],
) -> Result<Vec<JsTsDependencyEdge>> {
    let mut edges = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let import_re = Regex::new(r#"^\s*import\s+(.+?)\s+from\s+['"]([^'"]+)['"]\s*;?\s*$"#)?;
    let side_effect_import_re = Regex::new(r#"^\s*import\s+['"]([^'"]+)['"]\s*;?\s*$"#)?;
    let call_ident_re = Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(")?;
    let call_member_re = Regex::new(r"\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(")?;
    let function_decl_re =
        Regex::new(r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+[A-Za-z_]")?;
    let method_decl_re = Regex::new(
        r"^\s*(?:(?:public|private|protected|static|async|readonly|get|set)\s+)*[A-Za-z_][A-Za-z0-9_]*\s*\([^;]*\)\s*\{",
    )?;

    let callables = artefacts
        .iter()
        .filter(|a| {
            artefact_has_core_kind(
                a.canonical_kind.as_deref(),
                CoreCanonicalArtefactKind::Callable,
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut callable_name_to_fqn: HashMap<String, String> = HashMap::new();
    for c in &callables {
        callable_name_to_fqn
            .entry(c.name.clone())
            .or_insert_with(|| c.symbol_fqn.clone());
    }

    let mut imported_symbol_refs: HashMap<String, String> = HashMap::new();

    for (idx, line) in lines.iter().enumerate() {
        let line_no = (idx + 1) as i32;
        let trimmed = line.trim();

        if let Some(caps) = import_re.captures(line) {
            let clause = caps.get(1).map(|m| m.as_str()).unwrap_or("").trim();
            let module_ref = caps.get(2).map(|m| m.as_str()).unwrap_or("").to_string();
            if !module_ref.is_empty() {
                edges.push(JsTsDependencyEdge {
                    edge_kind: EdgeKind::Imports,
                    from_symbol_fqn: path.to_string(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(module_ref.clone()),
                    start_line: Some(line_no),
                    end_line: Some(line_no),
                    metadata: EdgeMetadata::import(ImportForm::Binding),
                });
            }
            parse_import_clause_symbols(clause, &module_ref, &mut imported_symbol_refs);
            continue;
        }

        if let Some(caps) = side_effect_import_re.captures(line) {
            let module_ref = caps.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
            if !module_ref.is_empty() {
                edges.push(JsTsDependencyEdge {
                    edge_kind: EdgeKind::Imports,
                    from_symbol_fqn: path.to_string(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(module_ref),
                    start_line: Some(line_no),
                    end_line: Some(line_no),
                    metadata: EdgeMetadata::import(ImportForm::SideEffect),
                });
            }
            continue;
        }

        let Some(owner) = smallest_enclosing_callable(line_no, &callables) else {
            continue;
        };
        if function_decl_re.is_match(line) || method_decl_re.is_match(line) {
            continue;
        }

        for caps in call_ident_re.captures_iter(line) {
            let Some(name_m) = caps.get(1) else {
                continue;
            };
            let name = name_m.as_str().to_string();
            if is_control_keyword(&name) {
                continue;
            }
            // Skip if the identifier is immediately preceded by '.' — it is a member access and
            // call_member_re will emit the correct member-form edge for it.
            if name_m.start() > 0
                && line.as_bytes().get(name_m.start() - 1).copied() == Some(b'.')
            {
                continue;
            }

            if let Some(target_fqn) = callable_name_to_fqn.get(&name) {
                edges.push(JsTsDependencyEdge {
                    edge_kind: EdgeKind::Calls,
                    from_symbol_fqn: owner.symbol_fqn.clone(),
                    to_target_symbol_fqn: Some(target_fqn.clone()),
                    to_symbol_ref: None,
                    start_line: Some(line_no),
                    end_line: Some(line_no),
                    metadata: EdgeMetadata::call(CallForm::Identifier, Resolution::Local),
                });
            } else if let Some(import_ref) = imported_symbol_refs.get(&name) {
                edges.push(JsTsDependencyEdge {
                    edge_kind: EdgeKind::Calls,
                    from_symbol_fqn: owner.symbol_fqn.clone(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(import_ref.clone()),
                    start_line: Some(line_no),
                    end_line: Some(line_no),
                    metadata: EdgeMetadata::call(CallForm::Identifier, Resolution::Import),
                });
            } else {
                edges.push(JsTsDependencyEdge {
                    edge_kind: EdgeKind::Calls,
                    from_symbol_fqn: owner.symbol_fqn.clone(),
                    to_target_symbol_fqn: None,
                    to_symbol_ref: Some(format!("{path}::{name}")),
                    start_line: Some(line_no),
                    end_line: Some(line_no),
                    metadata: EdgeMetadata::call(CallForm::Identifier, Resolution::Unresolved),
                });
            }
        }

        for caps in call_member_re.captures_iter(line) {
            let Some(name_m) = caps.get(1) else {
                continue;
            };
            let name = name_m.as_str().to_string();
            edges.push(JsTsDependencyEdge {
                edge_kind: EdgeKind::Calls,
                from_symbol_fqn: owner.symbol_fqn.clone(),
                to_target_symbol_fqn: None,
                to_symbol_ref: Some(format!("{path}::member::{name}")),
                start_line: Some(line_no),
                end_line: Some(line_no),
                metadata: EdgeMetadata::call(CallForm::Member, Resolution::Unresolved),
            });
        }

        if trimmed.is_empty() {
            continue;
        }
    }

    let mut parser = tree_sitter::Parser::new();
    let ts_lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let js_lang: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
    let callables = artefacts
        .iter()
        .filter(|a| {
            artefact_has_core_kind(
                a.canonical_kind.as_deref(),
                CoreCanonicalArtefactKind::Callable,
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let (type_targets, value_targets) = js_ts_reference_target_maps(artefacts);
    let export_targets = top_level_export_target_map(artefacts);
    let mut seen_references = HashSet::new();
    let mut seen_extends = HashSet::new();
    let mut seen_exports = HashSet::new();

    for lang in [ts_lang, js_lang] {
        if parser.set_language(&lang).is_err() {
            continue;
        }
        let Some(tree) = parser.parse(content, None) else {
            continue;
        };
        let root = tree.root_node();
        if root.has_error() {
            continue;
        }
        collect_js_ts_reference_edges_recursive(
            root,
            content,
            &ReferenceCtx {
                callables: &callables,
                type_targets: &type_targets,
                value_targets: &value_targets,
                imported_symbol_refs: &imported_symbol_refs,
            },
            &mut EdgeCollector {
                out: &mut edges,
                seen: &mut seen_references,
            },
        );
        collect_js_ts_extends_edges_recursive(
            root,
            content,
            &type_targets,
            &imported_symbol_refs,
            &mut seen_extends,
            &mut edges,
        );
        collect_js_ts_export_edges_recursive(
            root,
            content,
            path,
            &export_targets,
            &imported_symbol_refs,
            &mut seen_exports,
            &mut edges,
        );
        break;
    }

    Ok(edges)
}
