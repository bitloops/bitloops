use super::*;
use crate::host::language_adapter::{DependencyEdge, LanguageArtefact};

pub(crate) fn collect_checkpoint_artefact_provenance_rows(
    repo_root: &Path,
    ctx: CheckpointProvenanceContext<'_>,
    file_rows: &[CheckpointFileProvenanceRow],
) -> Result<Vec<CheckpointArtefactProvenanceRow>> {
    Ok(collect_checkpoint_artefact_provenance(repo_root, ctx, file_rows)?.semantic_rows)
}

pub(crate) fn collect_checkpoint_artefact_provenance(
    repo_root: &Path,
    ctx: CheckpointProvenanceContext<'_>,
    file_rows: &[CheckpointFileProvenanceRow],
) -> Result<CheckpointArtefactProvenanceBundle> {
    let mut bundle = CheckpointArtefactProvenanceBundle::default();
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
        let copy_source_artefacts = file_row
            .copy_source_path
            .as_deref()
            .zip(file_row.copy_source_blob_sha.as_deref())
            .map(|(path, blob_sha)| {
                extract_checkpoint_artefacts(repo_root, ctx.repo_id, path, blob_sha)
            })
            .transpose()?
            .unwrap_or_default();

        bundle.semantic_rows.extend(build_semantic_provenance_rows(
            ctx,
            &before_artefacts,
            &after_artefacts,
        ));
        bundle.lineage_rows.extend(build_copy_lineage_rows(
            ctx,
            &copy_source_artefacts,
            &after_artefacts,
        ));
    }

    Ok(bundle)
}

#[derive(Debug, Clone)]
struct ExtractedCheckpointArtefact {
    match_key: String,
    symbol_id: String,
    artefact_id: String,
    semantic_fingerprint: String,
}

fn build_semantic_provenance_rows(
    ctx: CheckpointProvenanceContext<'_>,
    before_artefacts: &[ExtractedCheckpointArtefact],
    after_artefacts: &[ExtractedCheckpointArtefact],
) -> Vec<CheckpointArtefactProvenanceRow> {
    let before_by_key = artefacts_by_key(before_artefacts);
    let after_by_key = artefacts_by_key(after_artefacts);
    let all_keys = before_by_key
        .keys()
        .chain(after_by_key.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();

    let mut rows = Vec::new();
    for key in all_keys {
        match (before_by_key.get(&key), after_by_key.get(&key)) {
            (Some(before), Some(after)) => {
                if before.semantic_fingerprint == after.semantic_fingerprint {
                    continue;
                }
                rows.push(build_semantic_row(
                    ctx,
                    CheckpointArtefactChangeKind::Modify,
                    Some(before),
                    Some(after),
                ));
            }
            (Some(before), None) => rows.push(build_semantic_row(
                ctx,
                CheckpointArtefactChangeKind::Delete,
                Some(before),
                None,
            )),
            (None, Some(after)) => rows.push(build_semantic_row(
                ctx,
                CheckpointArtefactChangeKind::Add,
                None,
                Some(after),
            )),
            (None, None) => {}
        }
    }
    rows
}

fn build_copy_lineage_rows(
    ctx: CheckpointProvenanceContext<'_>,
    source_artefacts: &[ExtractedCheckpointArtefact],
    dest_artefacts: &[ExtractedCheckpointArtefact],
) -> Vec<CheckpointArtefactLineageRow> {
    if source_artefacts.is_empty() || dest_artefacts.is_empty() {
        return Vec::new();
    }

    let source_by_key = artefacts_by_key(source_artefacts);
    let dest_by_key = artefacts_by_key(dest_artefacts);
    let mut rows = Vec::new();
    for key in source_by_key
        .keys()
        .filter(|key| dest_by_key.contains_key(*key))
    {
        let source = source_by_key.get(key).expect("key exists in source map");
        let dest = dest_by_key.get(key).expect("key exists in dest map");
        if source.semantic_fingerprint != dest.semantic_fingerprint {
            continue;
        }

        let mut row = CheckpointArtefactLineageRow {
            relation_id: String::new(),
            repo_id: ctx.repo_id.to_string(),
            checkpoint_id: ctx.checkpoint_id.to_string(),
            session_id: ctx.session_id.to_string(),
            event_time: ctx.event_time.to_string(),
            agent: ctx.agent.to_string(),
            branch: ctx.branch.to_string(),
            strategy: ctx.strategy.to_string(),
            commit_sha: ctx.commit_sha.to_string(),
            lineage_kind: CheckpointArtefactLineageKind::Copy,
            source_symbol_id: source.symbol_id.clone(),
            source_artefact_id: source.artefact_id.clone(),
            dest_symbol_id: dest.symbol_id.clone(),
            dest_artefact_id: dest.artefact_id.clone(),
        };
        row.relation_id = row.deterministic_id();
        rows.push(row);
    }
    rows
}

fn artefacts_by_key(
    artefacts: &[ExtractedCheckpointArtefact],
) -> BTreeMap<String, ExtractedCheckpointArtefact> {
    let mut by_key = BTreeMap::<String, ExtractedCheckpointArtefact>::new();
    for artefact in artefacts {
        by_key.insert(artefact.match_key.clone(), artefact.clone());
    }
    by_key
}

fn build_semantic_row(
    ctx: CheckpointProvenanceContext<'_>,
    change_kind: CheckpointArtefactChangeKind,
    before: Option<&ExtractedCheckpointArtefact>,
    after: Option<&ExtractedCheckpointArtefact>,
) -> CheckpointArtefactProvenanceRow {
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
        change_kind,
        before_symbol_id: before.map(|artefact| artefact.symbol_id.clone()),
        after_symbol_id: after.map(|artefact| artefact.symbol_id.clone()),
        before_artefact_id: before.map(|artefact| artefact.artefact_id.clone()),
        after_artefact_id: after.map(|artefact| artefact.artefact_id.clone()),
    };
    row.relation_id = row.deterministic_id();
    row
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::devql::checkpoint_provenance::collect_checkpoint_file_provenance_rows;
    use crate::test_support::git_fixtures::{git_ok, init_test_repo};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn formatting_only_edit_produces_no_semantic_rows() {
        let repo = TempDir::new().expect("temp dir");
        init_test_repo(
            repo.path(),
            "main",
            "Checkpoint Provenance",
            "provenance@example.com",
        );
        fs::create_dir_all(repo.path().join("src")).expect("create src dir");
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn demo() -> i32 {\n    1\n}\n",
        )
        .expect("write source file");
        git_ok(repo.path(), &["add", "src/lib.rs"]);
        git_ok(repo.path(), &["commit", "-m", "seed"]);

        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn demo() -> i32 {\n    // keep the same behaviour\n    1\n}\n",
        )
        .expect("rewrite source file");
        git_ok(repo.path(), &["add", "src/lib.rs"]);
        git_ok(repo.path(), &["commit", "-m", "formatting only"]);

        let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
        let ctx = test_context(commit_sha.trim());
        let file_rows =
            collect_checkpoint_file_provenance_rows(repo.path(), ctx).expect("collect file rows");
        let provenance = collect_checkpoint_artefact_provenance(repo.path(), ctx, &file_rows)
            .expect("collect provenance");

        assert_eq!(file_rows.len(), 1);
        assert!(provenance.semantic_rows.is_empty());
        assert!(provenance.lineage_rows.is_empty());
    }

    #[test]
    fn pure_copy_produces_add_rows_and_copy_lineage() {
        let repo = TempDir::new().expect("temp dir");
        init_test_repo(
            repo.path(),
            "main",
            "Checkpoint Provenance",
            "provenance@example.com",
        );
        fs::create_dir_all(repo.path().join("src")).expect("create src dir");
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn demo() -> i32 {\n    1\n}\n",
        )
        .expect("write source file");
        git_ok(repo.path(), &["add", "src/lib.rs"]);
        git_ok(repo.path(), &["commit", "-m", "seed"]);

        fs::copy(
            repo.path().join("src/lib.rs"),
            repo.path().join("src/copied.rs"),
        )
        .expect("copy source file");
        git_ok(repo.path(), &["add", "src/copied.rs"]);
        git_ok(repo.path(), &["commit", "-m", "copy"]);

        let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
        let ctx = test_context(commit_sha.trim());
        let file_rows =
            collect_checkpoint_file_provenance_rows(repo.path(), ctx).expect("collect file rows");
        let provenance = collect_checkpoint_artefact_provenance(repo.path(), ctx, &file_rows)
            .expect("collect provenance");

        assert_eq!(file_rows.len(), 1);
        assert_eq!(file_rows[0].change_kind, CheckpointFileChangeKind::Copy);
        assert!(!provenance.semantic_rows.is_empty());
        assert!(
            provenance
                .semantic_rows
                .iter()
                .all(|row| row.change_kind == CheckpointArtefactChangeKind::Add)
        );
        assert!(!provenance.lineage_rows.is_empty());
        assert!(provenance.lineage_rows.iter().all(|row| {
            row.lineage_kind == CheckpointArtefactLineageKind::Copy
                && row.source_artefact_id != row.dest_artefact_id
        }));
    }

    #[test]
    fn copy_with_semantic_edit_only_links_unchanged_artefacts() {
        let repo = TempDir::new().expect("temp dir");
        init_test_repo(
            repo.path(),
            "main",
            "Checkpoint Provenance",
            "provenance@example.com",
        );
        fs::create_dir_all(repo.path().join("src")).expect("create src dir");
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn first() -> i32 {\n    1\n}\n\npub fn second() -> i32 {\n    2\n}\n",
        )
        .expect("write source file");
        git_ok(repo.path(), &["add", "src/lib.rs"]);
        git_ok(repo.path(), &["commit", "-m", "seed"]);

        fs::copy(
            repo.path().join("src/lib.rs"),
            repo.path().join("src/copied.rs"),
        )
        .expect("copy source file");
        fs::write(
            repo.path().join("src/copied.rs"),
            "pub fn first() -> i32 {\n    10\n}\n\npub fn second() -> i32 {\n    2\n}\n",
        )
        .expect("rewrite copied file");
        git_ok(repo.path(), &["add", "src/copied.rs"]);
        git_ok(repo.path(), &["commit", "-m", "copy and edit"]);

        let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
        let ctx = test_context(commit_sha.trim());
        let file_rows =
            collect_checkpoint_file_provenance_rows(repo.path(), ctx).expect("collect file rows");
        let provenance = collect_checkpoint_artefact_provenance(repo.path(), ctx, &file_rows)
            .expect("collect provenance");

        assert_eq!(file_rows.len(), 1);
        assert_eq!(file_rows[0].change_kind, CheckpointFileChangeKind::Copy);
        assert_eq!(provenance.lineage_rows.len(), 1);
        assert!(
            provenance
                .semantic_rows
                .iter()
                .all(|row| row.change_kind == CheckpointArtefactChangeKind::Add)
        );
    }

    fn test_context<'a>(commit_sha: &'a str) -> CheckpointProvenanceContext<'a> {
        CheckpointProvenanceContext {
            repo_id: "repo-1",
            checkpoint_id: "checkpoint-1",
            session_id: "session-1",
            event_time: "2026-03-20T10:00:00Z",
            agent: "codex",
            branch: "main",
            strategy: "manual",
            commit_sha,
        }
    }
}
