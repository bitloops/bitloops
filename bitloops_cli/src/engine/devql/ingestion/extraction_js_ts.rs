// JS/TS artefact extraction via tree-sitter.

struct JsTsArtefactDescriptor<'a> {
    language_kind: &'a str,
    name: &'a str,
    symbol_fqn: String,
    parent_symbol_fqn: Option<String>,
}

fn extract_js_ts_artefacts(content: &str, path: &str) -> Result<Vec<JsTsArtefact>> {
    Ok(extract_js_ts_artefacts_treesitter(content, path)?.unwrap_or_default())
}

fn extract_js_ts_artefacts_treesitter(
    content: &str,
    path: &str,
) -> Result<Option<Vec<JsTsArtefact>>> {
    let mut parser = tree_sitter::Parser::new();
    let ts_lang: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let js_lang: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();

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
            out.sort_by_key(|i| {
                (
                    i.start_line,
                    i.end_line,
                    i.canonical_kind.clone(),
                    i.name.clone(),
                )
            });
            return Ok(Some(out));
        }
    }

    crate::engine::logging::warn(
        &crate::engine::logging::with_component(crate::engine::logging::background(), "devql"),
        "devql parse failure fallback",
        &[
            crate::engine::logging::string_attr("path", path),
            crate::engine::logging::string_attr("language_candidates", "typescript,javascript"),
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
    match node.kind() {
        "function_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                push_js_ts_artefact(
                    out,
                    seen,
                    node,
                    content,
                    JsTsArtefactDescriptor {
                        language_kind: node.kind(),
                        name,
                        symbol_fqn: format!("{path}::{name}"),
                        parent_symbol_fqn: None,
                    },
                );
            }
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                let class_fqn = format!("{path}::{name}");
                push_js_ts_artefact(
                    out,
                    seen,
                    node,
                    content,
                    JsTsArtefactDescriptor {
                        language_kind: "class_declaration",
                        name,
                        symbol_fqn: class_fqn.clone(),
                        parent_symbol_fqn: None,
                    },
                );
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cur = body.walk();
                    for child in body.named_children(&mut cur) {
                        match child.kind() {
                            "method_definition" => {
                                if let Some(name_node) = child.child_by_field_name("name")
                                    && let Ok(name) = name_node.utf8_text(content.as_bytes())
                                {
                                    let language_kind = if name == "constructor" {
                                        "constructor"
                                    } else {
                                        "method_definition"
                                    };
                                    push_js_ts_artefact(
                                        out,
                                        seen,
                                        child,
                                        content,
                                        JsTsArtefactDescriptor {
                                            language_kind,
                                            name,
                                            symbol_fqn: format!("{class_fqn}::{name}"),
                                            parent_symbol_fqn: Some(class_fqn.clone()),
                                        },
                                    );
                                }
                            }
                            "public_field_definition" => {
                                if let Some(name_node) = child.child_by_field_name("name")
                                    && let Ok(name) = name_node.utf8_text(content.as_bytes())
                                {
                                    push_js_ts_artefact(
                                        out,
                                        seen,
                                        child,
                                        content,
                                        JsTsArtefactDescriptor {
                                            language_kind: "public_field_definition",
                                            name,
                                            symbol_fqn: format!("{class_fqn}::{name}"),
                                            parent_symbol_fqn: Some(class_fqn.clone()),
                                        },
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        "variable_declarator" => {
            if is_js_ts_top_level_variable(node)
                && let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes())
            {
                push_js_ts_artefact(
                    out,
                    seen,
                    node,
                    content,
                    JsTsArtefactDescriptor {
                        language_kind: "variable_declarator",
                        name,
                        symbol_fqn: format!("{path}::{name}"),
                        parent_symbol_fqn: None,
                    },
                );
            }
        }
        "import_statement" => {
            let start_line = node.start_position().row as i32 + 1;
            let import_name = format!("import@{start_line}");
            push_js_ts_artefact(
                out,
                seen,
                node,
                content,
                JsTsArtefactDescriptor {
                    language_kind: "import_statement",
                    name: &import_name,
                    symbol_fqn: format!("{path}::import::{import_name}"),
                    parent_symbol_fqn: None,
                },
            );
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_js_ts_nodes_recursive(child, content, path, out, seen);
    }
}

fn push_js_ts_artefact(
    out: &mut Vec<JsTsArtefact>,
    seen: &mut HashSet<(String, String, i32)>,
    node: tree_sitter::Node,
    content: &str,
    descriptor: JsTsArtefactDescriptor<'_>,
) {
    let JsTsArtefactDescriptor {
        language_kind,
        name,
        symbol_fqn,
        parent_symbol_fqn,
    } = descriptor;

    if name.is_empty() || !js_ts_supports_language_kind(language_kind) {
        return;
    }

    let start_line = node.start_position().row as i32 + 1;
    if !seen.insert((language_kind.to_string(), name.to_string(), start_line)) {
        return;
    }

    let signature = node
        .utf8_text(content.as_bytes())
        .ok()
        .and_then(|s| s.lines().next())
        .unwrap_or("")
        .trim()
        .to_string();

    out.push(JsTsArtefact {
        canonical_kind: js_ts_canonical_kind(language_kind).map(str::to_string),
        language_kind: language_kind.to_string(),
        name: name.to_string(),
        symbol_fqn,
        parent_symbol_fqn,
        start_line,
        end_line: node.end_position().row as i32 + 1,
        start_byte: node.start_byte() as i32,
        end_byte: node.end_byte() as i32,
        signature,
        modifiers: extract_js_ts_modifiers(node, content),
        docstring: extract_js_ts_docstring(node, content),
    });
}

fn extract_js_ts_modifiers(node: tree_sitter::Node, content: &str) -> Vec<String> {
    let mut modifiers = Vec::new();
    let mut current = node;
    let mut wrappers = Vec::new();

    while let Some(parent) = current.parent() {
        if matches!(parent.kind(), "export_statement" | "ambient_declaration") {
            wrappers.push((parent, current));
        }
        current = parent;
    }

    wrappers.reverse();
    for (wrapper, child) in wrappers {
        collect_js_ts_wrapper_modifiers(wrapper, child, content, &mut modifiers);
    }
    collect_js_ts_inline_modifiers(node, content, &mut modifiers);
    modifiers
}

fn collect_js_ts_wrapper_modifiers(
    wrapper: tree_sitter::Node,
    child: tree_sitter::Node,
    content: &str,
    modifiers: &mut Vec<String>,
) {
    for index in 0..wrapper.child_count() {
        let Some(candidate) = wrapper.child(index) else {
            continue;
        };
        if candidate.start_byte() == child.start_byte()
            && candidate.end_byte() == child.end_byte()
            && candidate.kind() == child.kind()
        {
            break;
        }
        push_js_ts_modifier(modifiers, &js_ts_modifier_name(candidate, content));
    }
}

fn collect_js_ts_inline_modifiers(
    node: tree_sitter::Node,
    content: &str,
    modifiers: &mut Vec<String>,
) {
    let cutoff = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("body"))
        .map(|child| child.start_byte())
        .unwrap_or_else(|| node.end_byte());

    for index in 0..node.child_count() {
        let Some(child) = node.child(index) else {
            continue;
        };
        if child.end_byte() > cutoff {
            break;
        }
        push_js_ts_modifier(modifiers, &js_ts_modifier_name(child, content));
    }
}

fn js_ts_modifier_name(node: tree_sitter::Node, content: &str) -> Option<String> {
    let kind = node.kind();
    let text = node.utf8_text(content.as_bytes()).ok()?.trim();
    let modifier = match kind {
        "accessibility_modifier" | "override_modifier" => text,
        "public" | "protected" | "private" | "static" | "readonly" | "abstract" | "async"
        | "declare" | "export" | "default" | "get" | "set" | "accessor" => kind,
        _ => return None,
    };

    Some(modifier.to_ascii_lowercase())
}

fn push_js_ts_modifier(modifiers: &mut Vec<String>, modifier: &Option<String>) {
    let Some(modifier) = modifier.as_ref() else {
        return;
    };
    if !modifiers.iter().any(|existing| existing == modifier) {
        modifiers.push(modifier.clone());
    }
}

fn extract_js_ts_docstring(node: tree_sitter::Node, content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let anchor_line = js_ts_doc_anchor_line(node);
    if anchor_line <= 1 {
        return None;
    }

    let mut blocks = Vec::new();
    let mut line_idx = anchor_line - 2;

    while line_idx >= 0 {
        let trimmed = lines[line_idx as usize].trim();
        if trimmed.is_empty() {
            break;
        }

        if trimmed.starts_with("//") {
            let mut start = line_idx;
            while start > 0 && lines[(start - 1) as usize].trim().starts_with("//") {
                start -= 1;
            }
            blocks.push(normalize_js_ts_line_comment_block(
                &lines[start as usize..=line_idx as usize],
            ));
            line_idx = start - 1;
            continue;
        }

        if trimmed.contains("*/") || trimmed.starts_with('*') {
            let mut start = line_idx;
            while start >= 0 && !lines[start as usize].contains("/*") {
                start -= 1;
            }
            if start < 0 {
                break;
            }
            blocks.push(normalize_js_ts_block_comment_block(
                &lines[start as usize..=line_idx as usize],
            ));
            line_idx = start - 1;
            continue;
        }

        break;
    }

    if blocks.is_empty() {
        return None;
    }

    blocks.reverse();
    Some(
        blocks
            .into_iter()
            .filter(|block| !block.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
    .filter(|doc| !doc.trim().is_empty())
}

fn js_ts_doc_anchor_line(node: tree_sitter::Node) -> i32 {
    let mut anchor_line = node.start_position().row as i32 + 1;
    let mut current = node;

    while let Some(parent) = current.parent() {
        if matches!(parent.kind(), "export_statement" | "ambient_declaration") {
            anchor_line = anchor_line.min(parent.start_position().row as i32 + 1);
        }
        current = parent;
    }

    for index in 0..node.child_count() {
        let Some(child) = node.child(index) else {
            continue;
        };
        if child.kind() == "decorator" {
            anchor_line = anchor_line.min(child.start_position().row as i32 + 1);
        }
    }

    anchor_line
}

fn normalize_js_ts_line_comment_block(lines: &[&str]) -> String {
    lines
        .iter()
        .map(|line| {
            line.trim()
                .trim_start_matches('/')
                .trim_start_matches('/')
                .trim()
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn normalize_js_ts_block_comment_block(lines: &[&str]) -> String {
    let mut normalized = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let mut text = line.trim().to_string();
        if index == 0 {
            text = text
                .trim_start_matches('/')
                .trim_start_matches('*')
                .trim_start_matches('*')
                .trim()
                .to_string();
        }
        if index + 1 == lines.len() {
            text = text
                .trim_end_matches('/')
                .trim_end_matches('*')
                .trim()
                .to_string();
        }
        text = text.trim_start_matches('*').trim().to_string();
        normalized.push(text);
    }
    normalized.join("\n").trim().to_string()
}

fn is_js_ts_top_level_variable(node: tree_sitter::Node) -> bool {
    let mut current = Some(node);
    while let Some(cursor) = current {
        let Some(parent) = cursor.parent() else {
            return false;
        };
        match parent.kind() {
            "program" => return true,
            "export_statement" | "ambient_declaration" | "lexical_declaration"
            | "variable_declaration" => {
                current = Some(parent);
            }
            _ => return false,
        }
    }
    false
}
