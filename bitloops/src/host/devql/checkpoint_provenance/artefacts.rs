use super::*;
use crate::host::language_adapter::{DependencyEdge, LanguageArtefact};

pub(crate) fn collect_checkpoint_artefact_provenance_rows(
    repo_root: &Path,
    ctx: CheckpointProvenanceContext<'_>,
    file_rows: &[CheckpointFileProvenanceRow],
) -> Result<Vec<CheckpointArtefactProvenanceRow>> {
    let mut rows = Vec::new();
    for file_row in file_rows {
        let before_artefacts = file_row
            .path_before
            .as_deref()
            .zip(file_row.blob_sha_before.as_deref())
            .map(|(path, blob_sha)| {
                extract_checkpoint_artefacts(repo_root, ctx.repo_id, path, blob_sha)
            })
            .transpose()?
            .unwrap_or_default();
        let after_artefacts = file_row
            .path_after
            .as_deref()
            .zip(file_row.blob_sha_after.as_deref())
            .map(|(path, blob_sha)| {
                extract_checkpoint_artefacts(repo_root, ctx.repo_id, path, blob_sha)
            })
            .transpose()?
            .unwrap_or_default();

        let mut before_by_key = BTreeMap::<String, ExtractedCheckpointArtefact>::new();
        let mut after_by_key = BTreeMap::<String, ExtractedCheckpointArtefact>::new();
        for artefact in before_artefacts {
            before_by_key.insert(artefact.match_key.clone(), artefact);
        }
        for artefact in after_artefacts {
            after_by_key.insert(artefact.match_key.clone(), artefact);
        }

        let all_keys = before_by_key
            .keys()
            .chain(after_by_key.keys())
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();

        for key in all_keys {
            match (before_by_key.get(&key), after_by_key.get(&key)) {
                (Some(before), Some(after)) => {
                    if before.semantic_fingerprint == after.semantic_fingerprint {
                        continue;
                    }
                    let mut row = CheckpointArtefactProvenanceRow {
                        relation_id: String::new(),
                        repo_id: ctx.repo_id.to_string(),
                        checkpoint_id: ctx.checkpoint_id.to_string(),
                        session_id: ctx.session_id.to_string(),
                        event_time: ctx.event_time.to_string(),
                        agent: ctx.agent.to_string(),
                        branch: ctx.branch.to_string(),
                        strategy: ctx.strategy.to_string(),
                        commit_sha: ctx.commit_sha.to_string(),
                        change_kind: CheckpointArtefactChangeKind::Modify,
                        before_symbol_id: Some(before.symbol_id.clone()),
                        after_symbol_id: Some(after.symbol_id.clone()),
                        before_artefact_id: Some(before.artefact_id.clone()),
                        after_artefact_id: Some(after.artefact_id.clone()),
                    };
                    row.relation_id = row.deterministic_id();
                    rows.push(row);
                }
                (Some(before), None) => {
                    let mut row = CheckpointArtefactProvenanceRow {
                        relation_id: String::new(),
                        repo_id: ctx.repo_id.to_string(),
                        checkpoint_id: ctx.checkpoint_id.to_string(),
                        session_id: ctx.session_id.to_string(),
                        event_time: ctx.event_time.to_string(),
                        agent: ctx.agent.to_string(),
                        branch: ctx.branch.to_string(),
                        strategy: ctx.strategy.to_string(),
                        commit_sha: ctx.commit_sha.to_string(),
                        change_kind: CheckpointArtefactChangeKind::Delete,
                        before_symbol_id: Some(before.symbol_id.clone()),
                        after_symbol_id: None,
                        before_artefact_id: Some(before.artefact_id.clone()),
                        after_artefact_id: None,
                    };
                    row.relation_id = row.deterministic_id();
                    rows.push(row);
                }
                (None, Some(after)) => {
                    let mut row = CheckpointArtefactProvenanceRow {
                        relation_id: String::new(),
                        repo_id: ctx.repo_id.to_string(),
                        checkpoint_id: ctx.checkpoint_id.to_string(),
                        session_id: ctx.session_id.to_string(),
                        event_time: ctx.event_time.to_string(),
                        agent: ctx.agent.to_string(),
                        branch: ctx.branch.to_string(),
                        strategy: ctx.strategy.to_string(),
                        commit_sha: ctx.commit_sha.to_string(),
                        change_kind: CheckpointArtefactChangeKind::Add,
                        before_symbol_id: None,
                        after_symbol_id: Some(after.symbol_id.clone()),
                        before_artefact_id: None,
                        after_artefact_id: Some(after.artefact_id.clone()),
                    };
                    row.relation_id = row.deterministic_id();
                    rows.push(row);
                }
                (None, None) => {}
            }
        }
    }

    Ok(rows)
}

#[derive(Debug, Clone)]
struct ExtractedCheckpointArtefact {
    match_key: String,
    symbol_id: String,
    artefact_id: String,
    semantic_fingerprint: String,
}

fn extract_checkpoint_artefacts(
    repo_root: &Path,
    repo_id: &str,
    path: &str,
    blob_sha: &str,
) -> Result<Vec<ExtractedCheckpointArtefact>> {
    let Some(language) = resolve_language_id_for_file_path(path)
        .map(str::to_string)
        .or_else(|| {
            Path::new(path)
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::trim)
                .filter(|extension| !extension.is_empty())
                .map(str::to_ascii_lowercase)
        })
    else {
        return Ok(Vec::new());
    };
    let Some(pack_id) = resolve_language_pack_owner_for_input(&language, Some(path))
        .or_else(|| resolve_language_pack_owner(&language))
    else {
        return Ok(Vec::new());
    };
    let Some(content) = super::git::git_blob_content(repo_root, blob_sha) else {
        return Ok(Vec::new());
    };
    let registry = language_adapter_registry()?;
    let items = registry.extract_artefacts(pack_id, &content, path)?;
    let dependency_edges = registry.extract_dependency_edges(pack_id, &content, path, &items)?;
    let dependency_signals = build_dependency_signal_map(&dependency_edges);

    let file_symbol_id = super::ingestion_artefact_identity::file_symbol_id(path);
    let mut path_parent_symbols = HashMap::<String, String>::new();
    let mut semantic_parent_keys = HashMap::<String, String>::new();
    path_parent_symbols.insert(path.to_string(), file_symbol_id.clone());
    semantic_parent_keys.insert(path.to_string(), "file".to_string());

    let mut duplicate_keys = HashMap::<String, usize>::new();
    let mut out = Vec::new();
    for item in &items {
        if item.canonical_kind.as_deref() == Some("file") {
            continue;
        }

        let parent_symbol_id = item
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| path_parent_symbols.get(fqn))
            .cloned()
            .unwrap_or_else(|| file_symbol_id.clone());
        let semantic_parent_key = item
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| semantic_parent_keys.get(fqn))
            .cloned()
            .unwrap_or_else(|| "file".to_string());
        let symbol_id = super::ingestion_artefact_identity::structural_symbol_id_for_artefact(
            item,
            Some(parent_symbol_id.as_str()),
        );
        let artefact_id =
            super::ingestion_artefact_identity::revision_artefact_id(repo_id, blob_sha, &symbol_id);
        let semantic_key = deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}",
            item.canonical_kind.as_deref().unwrap_or("<null>"),
            item.language_kind,
            semantic_parent_key,
            super::ingestion_artefact_identity::semantic_name_for_artefact(item),
            super::ingestion_artefact_identity::normalize_identity_fragment(
                &super::ingestion_artefact_identity::identity_signature_for_artefact(item)
            ),
        ));
        let occurrence = duplicate_keys.entry(semantic_key.clone()).or_insert(0);
        *occurrence += 1;
        let match_key = format!("{semantic_key}@{}", *occurrence);

        let fingerprint = semantic_fingerprint(
            item,
            &content,
            dependency_signals
                .get(&item.symbol_fqn)
                .cloned()
                .unwrap_or_default()
                .as_slice(),
        );
        out.push(ExtractedCheckpointArtefact {
            match_key,
            symbol_id: symbol_id.clone(),
            artefact_id: artefact_id.clone(),
            semantic_fingerprint: fingerprint,
        });

        path_parent_symbols.insert(item.symbol_fqn.clone(), symbol_id);
        semantic_parent_keys.insert(item.symbol_fqn.clone(), semantic_key);
    }

    Ok(out)
}

fn build_dependency_signal_map(edges: &[DependencyEdge]) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::<String, Vec<String>>::new();
    for edge in edges {
        let target = edge
            .to_target_symbol_fqn
            .as_deref()
            .or(edge.to_symbol_ref.as_deref())
            .unwrap_or("");
        let signal = format!(
            "{}|{}",
            edge.edge_kind.as_str(),
            normalize_identity_fragment(target)
        );
        out.entry(edge.from_symbol_fqn.clone())
            .or_default()
            .push(signal);
    }
    for values in out.values_mut() {
        values.sort();
        values.dedup();
    }
    out
}

fn semantic_fingerprint(
    item: &LanguageArtefact,
    content: &str,
    dependency_signals: &[String],
) -> String {
    let source_slice =
        super::ingestion_artefact_persistence_symbols::artefact_source_slice(content, item);
    let without_docstring = item
        .docstring
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|docstring| source_slice.replace(docstring, " "))
        .unwrap_or_else(|| source_slice.to_string());
    let mut modifiers = item.modifiers.clone();
    modifiers.sort();
    let body = normalise_semantic_source(&without_docstring);
    deterministic_uuid(&format!(
        "{}|{}|{}|{}|{}|{}|{}",
        item.canonical_kind.as_deref().unwrap_or("<null>"),
        item.language_kind,
        super::ingestion_artefact_identity::semantic_name_for_artefact(item),
        super::ingestion_artefact_identity::normalize_identity_fragment(
            &super::ingestion_artefact_identity::identity_signature_for_artefact(item)
        ),
        serde_json::to_string(&modifiers).unwrap_or_else(|_| "[]".to_string()),
        body,
        dependency_signals.join("|"),
    ))
}

pub(crate) fn normalise_semantic_source(source: &str) -> String {
    static BLOCK_COMMENT_RE: OnceLock<Regex> = OnceLock::new();
    static LINE_COMMENT_RE: OnceLock<Regex> = OnceLock::new();
    static HASH_COMMENT_RE: OnceLock<Regex> = OnceLock::new();
    static PYTHON_DOCSTRING_RE: OnceLock<Regex> = OnceLock::new();

    let block_comment_re = BLOCK_COMMENT_RE
        .get_or_init(|| Regex::new(r"(?s)/\*.*?\*/").expect("block comment regex should compile"));
    let line_comment_re =
        LINE_COMMENT_RE.get_or_init(|| Regex::new(r"(?m)//.*$").expect("line comment regex"));
    let hash_comment_re =
        HASH_COMMENT_RE.get_or_init(|| Regex::new(r"(?m)#.*$").expect("hash comment regex"));
    let python_docstring_re = PYTHON_DOCSTRING_RE.get_or_init(|| {
        Regex::new(r#"(?s)("{3}.*?"{3}|'{3}.*?'{3})"#)
            .expect("python docstring regex should compile")
    });

    let without_block_comments = block_comment_re.replace_all(source, " ");
    let without_line_comments = line_comment_re.replace_all(&without_block_comments, " ");
    let without_hash_comments = hash_comment_re.replace_all(&without_line_comments, " ");
    let without_docstrings = python_docstring_re.replace_all(&without_hash_comments, " ");
    without_docstrings
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
}
