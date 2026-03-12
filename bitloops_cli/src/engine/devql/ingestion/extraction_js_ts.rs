// JS/TS artefact extraction via tree-sitter.

fn extract_js_ts_artefacts(content: &str, path: &str) -> Result<Vec<JsTsArtefact>> {
    Ok(extract_js_ts_artefacts_treesitter(content, path)?.unwrap_or_default())
}

fn extract_js_ts_artefacts_treesitter(content: &str, path: &str) -> Result<Option<Vec<JsTsArtefact>>> {
    let mut parser = tree_sitter::Parser::new();
    let ts_lang: tree_sitter::Language = tree_sitter_typescript::language_typescript();
    let js_lang: tree_sitter::Language = tree_sitter_javascript::language();

    let mut out = Vec::new();
    let mut seen: HashSet<(String, String, i32)> = HashSet::new();

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
        collect_js_ts_nodes_recursive(root, content, path, &mut out, &mut seen);
        if !out.is_empty() {
            out.sort_by_key(|i| (i.start_line, i.end_line, i.canonical_kind.clone(), i.name.clone()));
            return Ok(Some(out));
        }
    }

    crate::engine::logging::warn(
        &crate::engine::logging::with_component(crate::engine::logging::background(), "devql"),
        "devql parse failure fallback",
        &[
            crate::engine::logging::string_attr("path", path),
            crate::engine::logging::string_attr(
                "language_candidates",
                "typescript,javascript",
            ),
            crate::engine::logging::string_attr("failure_kind", "parse_error"),
        ],
    );

    Ok(None)
}

fn collect_js_ts_nodes_recursive(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    out: &mut Vec<JsTsArtefact>,
    seen: &mut HashSet<(String, String, i32)>,
) {
    let kind = node.kind();
    let start_line = node.start_position().row as i32 + 1;
    let end_line = node.end_position().row as i32 + 1;
    let start_byte = node.start_byte() as i32;
    let end_byte = node.end_byte() as i32;
    let line_sig = node
        .utf8_text(content.as_bytes())
        .ok()
        .and_then(|s| s.lines().next())
        .unwrap_or("")
        .trim()
        .to_string();

    let mut push = |language_kind: &str,
                    name: String,
                    symbol_fqn: String,
                    parent_symbol_fqn: Option<String>| {
        if name.is_empty() {
            return;
        }
        if !js_ts_supports_language_kind(language_kind) {
            return;
        }
        if !seen.insert((language_kind.to_string(), name.clone(), start_line)) {
            return;
        }
        out.push(JsTsArtefact {
            canonical_kind: js_ts_canonical_kind(language_kind).map(str::to_string),
            language_kind: language_kind.to_string(),
            name,
            symbol_fqn,
            parent_symbol_fqn,
            start_line,
            end_line,
            start_byte,
            end_byte,
            signature: line_sig.clone(),
        });
    };

    match kind {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(
                        "function_declaration",
                        name.to_string(),
                        format!("{path}::{name}"),
                        None,
                    );
                }
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    let class_fqn = format!("{path}::{name}");
                    push("class_declaration", name.to_string(), class_fqn.clone(), None);
                    if let Some(body) = node.child_by_field_name("body") {
                        let mut cur = body.walk();
                        for child in body.named_children(&mut cur) {
                            if child.kind() == "method_definition"
                                && let Some(mn) = child.child_by_field_name("name")
                                    && let Ok(method_name) = mn.utf8_text(content.as_bytes()) {
                                        let m_start_line = child.start_position().row as i32 + 1;
                                        let m_end_line = child.end_position().row as i32 + 1;
                                        let m_start_byte = child.start_byte() as i32;
                                        let m_end_byte = child.end_byte() as i32;
                                        let m_sig = child
                                            .utf8_text(content.as_bytes())
                                            .ok()
                                            .and_then(|s| s.lines().next())
                                            .unwrap_or("")
                                            .trim()
                                            .to_string();
                                        let language_kind = if method_name == "constructor" {
                                            "constructor"
                                        } else {
                                            "method_definition"
                                        };
                                        if js_ts_supports_language_kind(language_kind) {
                                            if !seen.insert((
                                                language_kind.to_string(),
                                                method_name.to_string(),
                                                m_start_line,
                                            )) {
                                                continue;
                                            }
                                            out.push(JsTsArtefact {
                                                canonical_kind: js_ts_canonical_kind(language_kind)
                                                    .map(str::to_string),
                                                language_kind: language_kind.to_string(),
                                                name: method_name.to_string(),
                                                symbol_fqn: format!("{class_fqn}::{method_name}"),
                                                parent_symbol_fqn: Some(class_fqn.clone()),
                                                start_line: m_start_line,
                                                end_line: m_end_line,
                                                start_byte: m_start_byte,
                                                end_byte: m_end_byte,
                                                signature: m_sig,
                                            });
                                        }
                                    }
                        }
                    }
                }
        }
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(
                        "interface_declaration",
                        name.to_string(),
                        format!("{path}::{name}"),
                        None,
                    );
                }
        }
        "type_alias_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(
                        "type_alias_declaration",
                        name.to_string(),
                        format!("{path}::{name}"),
                        None,
                    );
                }
        }
        "variable_declarator" => {
            if is_js_ts_top_level_variable(node)
                && let Some(name_node) = node.child_by_field_name("name")
                    && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                        push(
                            "variable_declarator",
                            name.to_string(),
                            format!("{path}::{name}"),
                            None,
                        );
                    }
        }
        "import_statement" => {
            let import_name = format!("import@{start_line}");
            push(
                "import_statement",
                import_name.clone(),
                format!("{path}::import::{import_name}"),
                None,
            );
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_js_ts_nodes_recursive(child, content, path, out, seen);
    }
}

fn is_js_ts_top_level_variable(node: tree_sitter::Node) -> bool {
    let mut current = Some(node);
    while let Some(cursor) = current {
        let Some(parent) = cursor.parent() else {
            return false;
        };
        match parent.kind() {
            "program" => return true,
            "export_statement" | "lexical_declaration" | "variable_declaration" => {
                current = Some(parent);
            }
            _ => return false,
        }
    }
    false
}
