// Shared edge-building utilities used by all language edge extractors.

struct EdgeCollector<'a> {
    out: &'a mut Vec<JsTsDependencyEdge>,
    seen: &'a mut HashSet<String>,
}

struct SymbolLookup<'a> {
    local_targets: &'a HashMap<String, String>,
    imported_symbol_refs: Option<&'a HashMap<String, String>>,
}

struct EdgeTarget {
    to_target_symbol_fqn: Option<String>,
    to_symbol_ref: Option<String>,
}

struct ReferenceCtx<'a> {
    callables: &'a [JsTsArtefact],
    type_targets: &'a HashMap<String, String>,
    value_targets: &'a HashMap<String, String>,
    imported_symbol_refs: &'a HashMap<String, String>,
}

fn smallest_enclosing_callable(line_no: i32, callables: &[JsTsArtefact]) -> Option<JsTsArtefact> {
    callables
        .iter()
        .filter(|c| c.start_line <= line_no && c.end_line >= line_no)
        .min_by_key(|c| c.end_line - c.start_line)
        .cloned()
}

fn is_control_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "for" | "while" | "switch" | "catch" | "return" | "new" | "typeof"
    )
}

fn parse_import_clause_symbols(
    clause: &str,
    module_ref: &str,
    imported_symbol_refs: &mut HashMap<String, String>,
) {
    let trimmed = clause.trim();
    if trimmed.is_empty() {
        return;
    }

    if let Some((default_part, rest)) = trimmed.split_once(',') {
        let default_alias = default_part.trim();
        if !default_alias.is_empty() {
            imported_symbol_refs.insert(default_alias.to_string(), format!("{module_ref}::default"));
        }
        parse_import_clause_symbols(rest, module_ref, imported_symbol_refs);
        return;
    }

    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let inner = &trimmed[1..trimmed.len() - 1];
        for part in inner.split(',') {
            let token = part.trim();
            if token.is_empty() {
                continue;
            }
            if let Some((orig, alias)) = token.split_once(" as ") {
                let orig = orig.trim();
                let alias = alias.trim();
                if !orig.is_empty() && !alias.is_empty() {
                    imported_symbol_refs.insert(alias.to_string(), format!("{module_ref}::{orig}"));
                }
            } else {
                imported_symbol_refs.insert(token.to_string(), format!("{module_ref}::{token}"));
            }
        }
        return;
    }

    if let Some(ns) = trimmed.strip_prefix("* as ") {
        let ns = ns.trim();
        if !ns.is_empty() {
            imported_symbol_refs.insert(ns.to_string(), format!("{module_ref}::*"));
        }
        return;
    }

    imported_symbol_refs.insert(trimmed.to_string(), format!("{module_ref}::default"));
}

fn js_ts_reference_target_maps(
    artefacts: &[JsTsArtefact],
) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut type_targets = HashMap::new();
    let mut value_targets = HashMap::new();

    for artefact in artefacts {
        if matches!(
            artefact.canonical_kind.as_str(),
            "interface" | "type" | "enum"
        ) || artefact.language_kind == "class_declaration"
        {
            type_targets
                .entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }
        if matches!(artefact.canonical_kind.as_str(), "variable" | "function")
            || artefact.language_kind == "class_declaration"
        {
            value_targets
                .entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }
    }

    (type_targets, value_targets)
}

fn rust_reference_target_maps(
    artefacts: &[JsTsArtefact],
) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut type_targets = HashMap::new();
    let mut value_targets = HashMap::new();

    for artefact in artefacts {
        if matches!(
            artefact.language_kind.as_str(),
            "struct_item" | "enum_item" | "trait_item" | "type_item"
        ) {
            type_targets
                .entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }
        if matches!(
            artefact.language_kind.as_str(),
            "const_item" | "static_item" | "function_item"
        ) {
            value_targets
                .entry(artefact.name.clone())
                .or_insert_with(|| artefact.symbol_fqn.clone());
        }
    }

    (type_targets, value_targets)
}

fn top_level_export_target_map(artefacts: &[JsTsArtefact]) -> HashMap<String, String> {
    let mut targets = HashMap::new();

    for artefact in artefacts {
        if artefact.parent_symbol_fqn.is_some() || artefact.name.is_empty() {
            continue;
        }

        if matches!(
            artefact.language_kind.as_str(),
            "import_statement" | "use_declaration" | "impl_item"
        ) {
            continue;
        }

        targets
            .entry(artefact.name.clone())
            .or_insert_with(|| artefact.symbol_fqn.clone());
    }

    targets
}

fn syntax_node_trimmed_text(node: tree_sitter::Node, content: &str) -> Option<String> {
    node.utf8_text(content.as_bytes())
        .ok()
        .map(str::trim)
        .map(str::to_string)
        .filter(|text| !text.is_empty())
}

fn strip_string_delimiters(text: &str) -> String {
    text.trim()
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`'))
        .to_string()
}

fn push_reference_edge(
    col: &mut EdgeCollector,
    from_symbol_fqn: &str,
    name: &str,
    line_no: i32,
    ref_kind: &str,
    lookup: &SymbolLookup,
) {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return;
    }

    let (to_target_symbol_fqn, to_symbol_ref, resolution) =
        if let Some(target_fqn) = lookup.local_targets.get(trimmed) {
            (Some(target_fqn.clone()), None, "local")
        } else if let Some(symbol_ref) = lookup.imported_symbol_refs.and_then(|m| m.get(trimmed)) {
            (None, Some(symbol_ref.clone()), "import")
        } else {
            return;
        };

    let key = format!(
        "{}|{}|{}|{}|{}|{}",
        from_symbol_fqn,
        to_target_symbol_fqn.as_deref().unwrap_or(""),
        to_symbol_ref.as_deref().unwrap_or(""),
        line_no,
        ref_kind,
        resolution
    );
    if !col.seen.insert(key) {
        return;
    }

    col.out.push(JsTsDependencyEdge {
        edge_kind: "references".to_string(),
        from_symbol_fqn: from_symbol_fqn.to_string(),
        to_target_symbol_fqn,
        to_symbol_ref,
        start_line: Some(line_no),
        end_line: Some(line_no),
        metadata: json!({
            "ref_kind": ref_kind,
            "resolution": resolution,
        }),
    });
}

fn push_inherits_edge(
    col: &mut EdgeCollector,
    from_symbol_fqn: &str,
    name: &str,
    line_no: i32,
    inherit_form: &str,
    lookup: &SymbolLookup,
) {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return;
    }

    let (to_target_symbol_fqn, to_symbol_ref, resolution) =
        if let Some(target_fqn) = lookup.local_targets.get(trimmed) {
            (Some(target_fqn.clone()), None, "local")
        } else if let Some(symbol_ref) =
            lookup.imported_symbol_refs.and_then(|m| m.get(trimmed))
        {
            (None, Some(symbol_ref.clone()), "import")
        } else {
            return;
        };

    let key = format!(
        "{}|{}|{}|{}|{}",
        from_symbol_fqn,
        to_target_symbol_fqn.as_deref().unwrap_or(""),
        to_symbol_ref.as_deref().unwrap_or(""),
        line_no,
        resolution
    );
    if !col.seen.insert(key) {
        return;
    }

    col.out.push(JsTsDependencyEdge {
        edge_kind: "inherits".to_string(),
        from_symbol_fqn: from_symbol_fqn.to_string(),
        to_target_symbol_fqn,
        to_symbol_ref,
        start_line: Some(line_no),
        end_line: Some(line_no),
        metadata: json!({
            "inherit_form": inherit_form,
            "resolution": resolution,
        }),
    });
}

fn symbol_lookup_name_from_node(node: tree_sitter::Node, content: &str) -> Option<String> {
    let text = node.utf8_text(content.as_bytes()).ok()?;
    symbol_lookup_name_from_text(text)
}

fn symbol_lookup_name_from_text(text: &str) -> Option<String> {
    let mut candidate = text.trim();
    if candidate.is_empty() {
        return None;
    }

    if candidate.starts_with("for<") {
        candidate = candidate
            .rsplit_once(' ')
            .map(|(_, tail)| tail)
            .unwrap_or(candidate)
            .trim();
    }

    candidate = candidate.trim_start_matches('&').trim();
    candidate = candidate.trim_start_matches("mut ").trim();
    candidate = candidate.trim_start_matches("dyn ").trim();
    candidate = candidate.trim_start_matches("impl ").trim();
    candidate = candidate.split('<').next().unwrap_or(candidate).trim();
    candidate = candidate.rsplit("::").next().unwrap_or(candidate).trim();
    candidate = candidate.rsplit('.').next().unwrap_or(candidate).trim();
    candidate = candidate.trim_matches(|ch: char| matches!(ch, '&' | '(' | ')' | ',' | ';'));

    let first = candidate.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }

    Some(candidate.to_string())
}

fn js_ts_identifier_is_value_reference(node: tree_sitter::Node) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };

    match parent.kind() {
        "function_declaration"
        | "method_definition"
        | "class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "required_parameter"
        | "optional_parameter"
        | "rest_pattern"
        | "import_specifier"
        | "namespace_import"
        | "pair_pattern"
        | "shorthand_property_identifier_pattern" => false,
        "variable_declarator" => !node_matches_parent_field(node, parent, "name"),
        "call_expression" => !node_matches_parent_field(node, parent, "function"),
        "new_expression" => !node_matches_parent_field(node, parent, "constructor"),
        "member_expression" => !node_matches_parent_field(node, parent, "property"),
        "pair" => !node_matches_parent_field(node, parent, "key"),
        "property_signature" => !node_matches_parent_field(node, parent, "name"),
        _ => true,
    }
}

fn rust_identifier_is_value_reference(node: tree_sitter::Node) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };

    match parent.kind() {
        "function_item" | "struct_item" | "trait_item" | "type_item" | "const_item"
        | "static_item" | "parameter" | "self_parameter" | "use_as_clause" => false,
        "let_declaration" => !node_matches_parent_field(node, parent, "pattern"),
        "call_expression" => !node_matches_parent_field(node, parent, "function"),
        "field_expression" => !node_matches_parent_field(node, parent, "field"),
        "macro_invocation" => !node_matches_parent_field(node, parent, "macro"),
        _ => true,
    }
}

fn node_matches_parent_field(
    node: tree_sitter::Node,
    parent: tree_sitter::Node,
    field: &str,
) -> bool {
    parent
        .child_by_field_name(field)
        .map(|child| same_syntax_node(child, node))
        .unwrap_or(false)
}

fn same_syntax_node(left: tree_sitter::Node, right: tree_sitter::Node) -> bool {
    left.kind() == right.kind()
        && left.start_byte() == right.start_byte()
        && left.end_byte() == right.end_byte()
}

#[cfg(test)]
fn line_byte_spans(content: &str) -> Vec<(i32, i32)> {
    if content.is_empty() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut cursor: i32 = 0;
    for segment in content.split_inclusive('\n') {
        let start = cursor;
        let end = start + segment.len() as i32;
        spans.push((start, end));
        cursor = end;
    }
    spans
}

#[cfg(test)]
fn line_start_byte(spans: &[(i32, i32)], line: i32) -> i32 {
    if line <= 0 {
        return 0;
    }
    spans
        .get((line - 1) as usize)
        .map(|(start, _)| *start)
        .unwrap_or(0)
}

#[cfg(test)]
fn line_end_byte(spans: &[(i32, i32)], line: i32) -> i32 {
    if line <= 0 {
        return 0;
    }
    if let Some((_, end)) = spans.get((line - 1) as usize) {
        return *end;
    }
    spans.last().map(|(_, end)| *end).unwrap_or(0)
}

#[cfg(test)]
fn find_block_end_line(lines: &[&str], start_index: usize) -> Option<i32> {
    let mut found_open = false;
    let mut depth = 0i32;

    for (line_idx, line) in lines.iter().enumerate().skip(start_index) {
        for ch in line.chars() {
            match ch {
                '{' => {
                    found_open = true;
                    depth += 1;
                }
                '}' if found_open => {
                    depth -= 1;
                    if depth <= 0 {
                        return Some((line_idx + 1) as i32);
                    }
                }
                _ => {}
            }
        }
    }

    if found_open {
        Some(lines.len() as i32)
    } else {
        None
    }
}
