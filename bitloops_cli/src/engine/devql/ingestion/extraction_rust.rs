// Rust artefact extraction via tree-sitter.

fn extract_rust_artefacts(content: &str, path: &str) -> Result<Vec<JsTsArtefact>> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::language();
    parser
        .set_language(&lang)
        .context("setting tree-sitter rust language")?;
    let Some(tree) = parser.parse(content, None) else {
        return Ok(Vec::new());
    };

    let root = tree.root_node();
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String, i32)> = HashSet::new();
    collect_rust_nodes_recursive(root, content, path, &mut out, &mut seen, None);
    out.sort_by_key(|i| (i.start_line, i.end_line, i.canonical_kind.clone(), i.name.clone()));
    Ok(out)
}

fn collect_rust_nodes_recursive(
    node: tree_sitter::Node,
    content: &str,
    path: &str,
    out: &mut Vec<JsTsArtefact>,
    seen: &mut HashSet<(String, String, i32)>,
    current_impl_fqn: Option<String>,
) {
    let kind = node.kind();
    let start_line = node.start_position().row as i32 + 1;
    let end_line = node.end_position().row as i32 + 1;
    let start_byte = node.start_byte() as i32;
    let end_byte = node.end_byte() as i32;
    let signature = node
        .utf8_text(content.as_bytes())
        .ok()
        .and_then(|s| s.lines().next())
        .unwrap_or("")
        .trim()
        .to_string();

    let push = |out: &mut Vec<JsTsArtefact>,
                seen: &mut HashSet<(String, String, i32)>,
                language_kind: &str,
                name: String,
                symbol_fqn: String,
                parent_symbol_fqn: Option<String>,
                inside_impl: bool| {
        if name.is_empty() {
            return;
        }
        if !rust_supports_language_kind(language_kind) {
            return;
        }
        if !seen.insert((language_kind.to_string(), name.clone(), start_line)) {
            return;
        }
        out.push(JsTsArtefact {
            canonical_kind: rust_canonical_kind(language_kind, inside_impl).map(str::to_string),
            language_kind: language_kind.to_string(),
            name,
            symbol_fqn,
            parent_symbol_fqn,
            start_line,
            end_line,
            start_byte,
            end_byte,
            signature: signature.clone(),
        });
    };

    let mut next_impl_fqn = current_impl_fqn.clone();

    match kind {
        "mod_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(
                        out,
                        seen,
                        "mod_item",
                        name.to_string(),
                        format!("{path}::{name}"),
                        None,
                        false,
                    );
                }
        }
        "struct_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(
                        out,
                        seen,
                        "struct_item",
                        name.to_string(),
                        format!("{path}::{name}"),
                        None,
                        false,
                    );
                }
        }
        "enum_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(
                        out,
                        seen,
                        "enum_item",
                        name.to_string(),
                        format!("{path}::{name}"),
                        None,
                        false,
                    );
                }
        }
        "trait_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(
                        out,
                        seen,
                        "trait_item",
                        name.to_string(),
                        format!("{path}::{name}"),
                        None,
                        false,
                    );
                }
        }
        "type_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(
                        out,
                        seen,
                        "type_item",
                        name.to_string(),
                        format!("{path}::{name}"),
                        None,
                        false,
                    );
                }
        }
        "const_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(
                        out,
                        seen,
                        "const_item",
                        name.to_string(),
                        format!("{path}::{name}"),
                        None,
                        false,
                    );
                }
        }
        "static_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    push(
                        out,
                        seen,
                        "static_item",
                        name.to_string(),
                        format!("{path}::{name}"),
                        None,
                        false,
                    );
                }
        }
        "use_declaration" => {
            let name = format!("use@{start_line}");
            push(
                out,
                seen,
                "use_declaration",
                name.clone(),
                format!("{path}::{name}"),
                None,
                false,
            );
        }
        "impl_item" => {
            let name = format!("impl@{start_line}");
            let impl_fqn = format!("{path}::{name}");
            push(
                out,
                seen,
                "impl_item",
                name.clone(),
                impl_fqn.clone(),
                None,
                false,
            );
            next_impl_fqn = Some(impl_fqn);
        }
        "function_item" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(content.as_bytes()) {
                    if let Some(impl_fqn) = current_impl_fqn.clone() {
                        push(
                            out,
                            seen,
                            "function_item",
                            name.to_string(),
                            format!("{impl_fqn}::{name}"),
                            Some(impl_fqn),
                            true,
                        );
                    } else {
                        push(
                            out,
                            seen,
                            "function_item",
                            name.to_string(),
                            format!("{path}::{name}"),
                            None,
                            false,
                        );
                    }
                }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_rust_nodes_recursive(child, content, path, out, seen, next_impl_fqn.clone());
    }
}
