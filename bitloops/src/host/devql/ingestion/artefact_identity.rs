use super::*;

// Artefact symbol identity helpers: normalisation, semantic IDs, and the
// legacy regex-based function extractor (test-only).

pub(super) fn normalize_identity_fragment(input: &str) -> String {
    let normalized = input
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if normalized.is_empty() {
        input.trim().to_string()
    } else {
        normalized
    }
}

pub(super) fn has_positional_identity_name(name: &str) -> bool {
    name.rsplit_once('@')
        .map(|(_, suffix)| !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
        .unwrap_or(false)
}

pub(super) fn source_path_from_symbol_fqn(symbol_fqn: &str) -> &str {
    symbol_fqn.split("::").next().unwrap_or(symbol_fqn)
}

pub(super) fn semantic_name_for_artefact(item: &LanguageArtefact) -> String {
    if has_positional_identity_name(&item.name) {
        normalize_identity_fragment(&identity_signature_for_artefact(item))
    } else {
        normalize_identity_fragment(&item.name)
    }
}

pub(super) fn identity_signature_for_artefact(item: &LanguageArtefact) -> String {
    let mut signature = item.signature.clone();
    let mut modifiers = item
        .modifiers
        .iter()
        .filter(|modifier| !matches!(modifier.as_str(), "get" | "set"))
        .collect::<Vec<_>>();
    modifiers.sort_by_key(|modifier| std::cmp::Reverse(modifier.len()));

    for modifier in modifiers {
        let escaped = regex::escape(modifier);
        let pattern = Regex::new(&format!(r"(^|[\s(]){}($|[\s(])", escaped))
            .expect("modifier regex should compile");
        signature = pattern.replace_all(&signature, "$1$2").to_string();
    }

    signature
}

pub(super) fn structural_symbol_id_for_artefact(
    item: &LanguageArtefact,
    parent_symbol_id: Option<&str>,
) -> String {
    deterministic_uuid(&format!(
        "{}|{}|{}|{}|{}|{}",
        source_path_from_symbol_fqn(&item.symbol_fqn),
        item.canonical_kind.as_deref().unwrap_or("<null>"),
        item.language_kind,
        parent_symbol_id.unwrap_or(""),
        semantic_name_for_artefact(item),
        normalize_identity_fragment(&identity_signature_for_artefact(item))
    ))
}

pub(super) fn file_symbol_id(path: &str) -> String {
    deterministic_uuid(&format!("{path}|file"))
}

pub(super) fn revision_artefact_id(repo_id: &str, blob_sha: &str, symbol_id: &str) -> String {
    let provenance = CanonicalProvenanceRef::for_blob(repo_id, blob_sha);
    deterministic_uuid(&format!(
        "{}|{symbol_id}",
        provenance.artefact_identity_scope()
    ))
}

#[cfg(test)]
pub(super) fn extract_js_ts_functions(content: &str) -> Result<Vec<FunctionArtefact>> {
    let function_decl = Regex::new(
        r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(",
    )?;
    let function_expr = Regex::new(
        r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:async\s*)?(?:function\s*)?\([^)]*\)\s*=>",
    )?;

    let lines: Vec<&str> = content.lines().collect();
    let line_spans = line_byte_spans(content);
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let start_line = (idx + 1) as i32;

        let name = function_decl
            .captures(line)
            .or_else(|| function_expr.captures(line))
            .and_then(|captures| captures.get(1).map(|m| m.as_str().to_string()));

        let Some(name) = name else {
            continue;
        };

        if !seen.insert((name.clone(), start_line)) {
            continue;
        }

        let end_line = find_block_end_line(&lines, idx).unwrap_or(start_line);
        out.push(FunctionArtefact {
            start_byte: line_start_byte(&line_spans, start_line),
            end_byte: line_end_byte(&line_spans, end_line),
            end_line,
            name,
            signature: line.trim().to_string(),
            start_line,
        });
    }

    Ok(out)
}
