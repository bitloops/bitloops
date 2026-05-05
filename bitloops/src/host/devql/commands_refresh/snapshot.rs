use std::collections::BTreeSet;

use anyhow::{Context, Result};

use crate::config::resolve_store_backend_config_for_repo;
use crate::host::devql::{DevqlConfig, RelationalStorage, esc_pg, sql_string_list_pg};

use super::super::normalize_repo_path;

pub(crate) async fn snapshot_committed_current_rows_for_commit_for_config(
    cfg: &DevqlConfig,
    snapshot: &crate::daemon::PostCommitSnapshotSpec,
) -> Result<()> {
    let commit_sha = snapshot.commit_sha.trim();
    if commit_sha.is_empty() {
        return Ok(());
    }
    let current_head = crate::host::checkpoints::strategy::manual_commit::head_hash(&cfg.repo_root)
        .context("resolving HEAD before post-commit semantic snapshot")?;
    if current_head.trim() != commit_sha {
        log::debug!(
            "skipping stale post-commit semantic snapshot for commit {} because repository HEAD is {}",
            commit_sha,
            current_head.trim()
        );
        return Ok(());
    }

    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .context("resolving DevQL backend config for post-commit semantic snapshot")?;
    let relational =
        RelationalStorage::connect(cfg, &backends.relational, "post-commit semantic snapshot")
            .await?;
    snapshot_committed_current_rows_for_commit(
        &relational,
        &cfg.repo.repo_id,
        commit_sha,
        &snapshot.changed_paths,
    )
    .await
}

pub(super) async fn snapshot_committed_current_rows_for_commit(
    relational: &RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
    changed_paths: &[String],
) -> Result<()> {
    let commit_sha = commit_sha.trim();
    if repo_id.trim().is_empty() || commit_sha.is_empty() {
        return Ok(());
    }

    let repo_id = esc_pg(repo_id);
    let commit_sha = esc_pg(commit_sha);
    let scoped_paths = changed_paths
        .iter()
        .map(|raw| normalize_repo_path(raw))
        .filter(|path| !path.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let current_scope_predicate = sql_optional_path_scope_predicate_pg("c.path", &scoped_paths);
    let clone_scope_predicate = sql_clone_scope_predicate_pg("src.path", "tgt.path", &scoped_paths);
    relational
        .exec_batch_transactional(&[
            format!(
                "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) \
                 SELECT c.repo_id, '{commit_sha}', c.path, c.effective_content_id \
                 FROM current_file_state c \
                 WHERE c.repo_id = '{repo_id}' \
                   AND c.effective_source = 'head' \
                   AND c.effective_content_id IS NOT NULL \
                 ON CONFLICT (repo_id, commit_sha, path) DO UPDATE SET blob_sha = EXCLUDED.blob_sha"
            ),
            format!(
                "INSERT INTO artefacts (
                    artefact_id, symbol_id, repo_id, language, canonical_kind, language_kind,
                    symbol_fqn, signature, modifiers, docstring, content_hash
                 )
                 SELECT
                    a.artefact_id, a.symbol_id, a.repo_id, a.language, a.canonical_kind,
                    a.language_kind, a.symbol_fqn, a.signature, a.modifiers, a.docstring,
                    a.content_id
                 FROM artefacts_current a
                 JOIN current_file_state c
                   ON c.repo_id = a.repo_id
                  AND c.path = a.path
                  AND c.effective_source = 'head'
                  AND c.effective_content_id = a.content_id
                 WHERE a.repo_id = '{repo_id}'
                   {current_scope_predicate}
                 ON CONFLICT (artefact_id) DO UPDATE SET
                    symbol_id = EXCLUDED.symbol_id,
                    repo_id = EXCLUDED.repo_id,
                    language = EXCLUDED.language,
                    canonical_kind = EXCLUDED.canonical_kind,
                    language_kind = EXCLUDED.language_kind,
                    symbol_fqn = EXCLUDED.symbol_fqn,
                    signature = EXCLUDED.signature,
                    modifiers = EXCLUDED.modifiers,
                    docstring = EXCLUDED.docstring,
                    content_hash = EXCLUDED.content_hash",
                current_scope_predicate = current_scope_predicate,
            ),
            format!(
                "INSERT INTO artefact_snapshots (
                    repo_id, blob_sha, path, artefact_id, parent_artefact_id,
                    start_line, end_line, start_byte, end_byte
                 )
                 SELECT
                    a.repo_id, a.content_id, a.path, a.artefact_id, a.parent_artefact_id,
                    a.start_line, a.end_line, a.start_byte, a.end_byte
                 FROM artefacts_current a
                 JOIN current_file_state c
                   ON c.repo_id = a.repo_id
                  AND c.path = a.path
                  AND c.effective_source = 'head'
                  AND c.effective_content_id = a.content_id
                 WHERE a.repo_id = '{repo_id}'
                   {current_scope_predicate}
                 ON CONFLICT (repo_id, blob_sha, artefact_id) DO UPDATE SET
                    path = EXCLUDED.path,
                    parent_artefact_id = EXCLUDED.parent_artefact_id,
                    start_line = EXCLUDED.start_line,
                    end_line = EXCLUDED.end_line,
                    start_byte = EXCLUDED.start_byte,
                    end_byte = EXCLUDED.end_byte",
                current_scope_predicate = current_scope_predicate,
            ),
            format!(
                "INSERT INTO symbol_semantics (
                    artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                    docstring_summary, llm_summary, template_summary, summary, confidence,
                    source_model
                 )
                 SELECT
                    s.artefact_id, s.repo_id, s.content_id, s.semantic_features_input_hash,
                    s.docstring_summary, s.llm_summary, s.template_summary, s.summary,
                    s.confidence, s.source_model
                 FROM symbol_semantics_current s
                 JOIN current_file_state c
                   ON c.repo_id = s.repo_id
                  AND c.path = s.path
                  AND c.effective_source = 'head'
                  AND c.effective_content_id = s.content_id
                 WHERE s.repo_id = '{repo_id}'
                   {current_scope_predicate}
                 ON CONFLICT (artefact_id) DO UPDATE SET
                    repo_id = EXCLUDED.repo_id,
                    blob_sha = EXCLUDED.blob_sha,
                    semantic_features_input_hash = EXCLUDED.semantic_features_input_hash,
                    docstring_summary = EXCLUDED.docstring_summary,
                    llm_summary = EXCLUDED.llm_summary,
                    template_summary = EXCLUDED.template_summary,
                    summary = EXCLUDED.summary,
                    confidence = EXCLUDED.confidence,
                    source_model = EXCLUDED.source_model",
                current_scope_predicate = current_scope_predicate,
            ),
            format!(
                "INSERT INTO symbol_features (
                    artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                    normalized_name, normalized_signature, modifiers, identifier_tokens,
                    normalized_body_tokens, parent_kind, context_tokens
                 )
                 SELECT
                    f.artefact_id, f.repo_id, f.content_id, f.semantic_features_input_hash,
                    f.normalized_name, f.normalized_signature, f.modifiers,
                    f.identifier_tokens, f.normalized_body_tokens, f.parent_kind,
                    f.context_tokens
                 FROM symbol_features_current f
                 JOIN current_file_state c
                   ON c.repo_id = f.repo_id
                  AND c.path = f.path
                  AND c.effective_source = 'head'
                  AND c.effective_content_id = f.content_id
                 WHERE f.repo_id = '{repo_id}'
                   {current_scope_predicate}
                 ON CONFLICT (artefact_id) DO UPDATE SET
                    repo_id = EXCLUDED.repo_id,
                    blob_sha = EXCLUDED.blob_sha,
                    semantic_features_input_hash = EXCLUDED.semantic_features_input_hash,
                    normalized_name = EXCLUDED.normalized_name,
                    normalized_signature = EXCLUDED.normalized_signature,
                    modifiers = EXCLUDED.modifiers,
                    identifier_tokens = EXCLUDED.identifier_tokens,
                    normalized_body_tokens = EXCLUDED.normalized_body_tokens,
                    parent_kind = EXCLUDED.parent_kind,
                    context_tokens = EXCLUDED.context_tokens",
                current_scope_predicate = current_scope_predicate,
            ),
            format!(
                "INSERT INTO symbol_search_documents (
                    artefact_id, repo_id, blob_sha, path, symbol_id, signature_text,
                    summary_text, body_text, searchable_text
                 )
                 SELECT
                    d.artefact_id, d.repo_id, d.content_id, d.path, d.symbol_id,
                    d.signature_text, d.summary_text, d.body_text, d.searchable_text
                 FROM symbol_search_documents_current d
                 JOIN current_file_state c
                   ON c.repo_id = d.repo_id
                  AND c.path = d.path
                  AND c.effective_source = 'head'
                  AND c.effective_content_id = d.content_id
                 WHERE d.repo_id = '{repo_id}'
                   {current_scope_predicate}
                 ON CONFLICT (artefact_id) DO UPDATE SET
                    repo_id = EXCLUDED.repo_id,
                    blob_sha = EXCLUDED.blob_sha,
                    path = EXCLUDED.path,
                    symbol_id = EXCLUDED.symbol_id,
                    signature_text = EXCLUDED.signature_text,
                    summary_text = EXCLUDED.summary_text,
                    body_text = EXCLUDED.body_text,
                    searchable_text = EXCLUDED.searchable_text",
                current_scope_predicate = current_scope_predicate,
            ),
            format!(
                "INSERT INTO symbol_embeddings (
                    artefact_id, repo_id, blob_sha, representation_kind, setup_fingerprint,
                    provider, model, dimension, embedding_input_hash, embedding
                 )
                 SELECT
                    e.artefact_id, e.repo_id, e.content_id, e.representation_kind,
                    e.setup_fingerprint, e.provider, e.model, e.dimension,
                    e.embedding_input_hash, e.embedding
                 FROM symbol_embeddings_current e
                 JOIN current_file_state c
                   ON c.repo_id = e.repo_id
                  AND c.path = e.path
                  AND c.effective_source = 'head'
                  AND c.effective_content_id = e.content_id
                 WHERE e.repo_id = '{repo_id}'
                   {current_scope_predicate}
                 ON CONFLICT (artefact_id, representation_kind, setup_fingerprint) DO UPDATE SET
                    repo_id = EXCLUDED.repo_id,
                    blob_sha = EXCLUDED.blob_sha,
                    provider = EXCLUDED.provider,
                    model = EXCLUDED.model,
                    dimension = EXCLUDED.dimension,
                    embedding_input_hash = EXCLUDED.embedding_input_hash,
                    embedding = EXCLUDED.embedding",
                current_scope_predicate = current_scope_predicate,
            ),
            format!(
                "INSERT INTO symbol_clone_edges (
                    repo_id, source_symbol_id, source_artefact_id, target_symbol_id,
                    target_artefact_id, relation_kind, score, semantic_score,
                    lexical_score, structural_score, clone_input_hash, explanation_json
                 )
                 SELECT
                    ce.repo_id, ce.source_symbol_id, ce.source_artefact_id,
                    ce.target_symbol_id, ce.target_artefact_id, ce.relation_kind, ce.score,
                    ce.semantic_score, ce.lexical_score, ce.structural_score,
                    ce.clone_input_hash, ce.explanation_json
                 FROM symbol_clone_edges_current ce
                 JOIN artefacts_current src
                   ON src.repo_id = ce.repo_id
                  AND src.artefact_id = ce.source_artefact_id
                 JOIN current_file_state src_state
                   ON src_state.repo_id = src.repo_id
                  AND src_state.path = src.path
                  AND src_state.effective_source = 'head'
                  AND src_state.effective_content_id = src.content_id
                 JOIN artefacts_current tgt
                   ON tgt.repo_id = ce.repo_id
                  AND tgt.artefact_id = ce.target_artefact_id
                 JOIN current_file_state tgt_state
                   ON tgt_state.repo_id = tgt.repo_id
                  AND tgt_state.path = tgt.path
                  AND tgt_state.effective_source = 'head'
                  AND tgt_state.effective_content_id = tgt.content_id
                 WHERE ce.repo_id = '{repo_id}'
                   {clone_scope_predicate}
                 ON CONFLICT (repo_id, source_artefact_id, target_artefact_id) DO UPDATE SET
                    source_symbol_id = EXCLUDED.source_symbol_id,
                    source_artefact_id = EXCLUDED.source_artefact_id,
                    target_symbol_id = EXCLUDED.target_symbol_id,
                    target_artefact_id = EXCLUDED.target_artefact_id,
                    relation_kind = EXCLUDED.relation_kind,
                    score = EXCLUDED.score,
                    semantic_score = EXCLUDED.semantic_score,
                    lexical_score = EXCLUDED.lexical_score,
                    structural_score = EXCLUDED.structural_score,
                    clone_input_hash = EXCLUDED.clone_input_hash,
                    explanation_json = EXCLUDED.explanation_json",
                clone_scope_predicate = clone_scope_predicate,
            ),
        ])
        .await
}

fn sql_optional_path_scope_predicate_pg(column: &str, paths: &[String]) -> String {
    if paths.is_empty() {
        String::new()
    } else {
        format!("AND {column} IN ({})", sql_string_list_pg(paths))
    }
}

fn sql_clone_scope_predicate_pg(
    source_column: &str,
    target_column: &str,
    paths: &[String],
) -> String {
    if paths.is_empty() {
        String::new()
    } else {
        format!(
            "AND ({source_column} IN ({paths_sql}) OR {target_column} IN ({paths_sql}))",
            paths_sql = sql_string_list_pg(paths),
        )
    }
}
