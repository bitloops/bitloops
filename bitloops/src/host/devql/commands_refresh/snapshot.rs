use std::collections::BTreeSet;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::capability_packs::semantic_clones::embeddings::{
    EmbeddingRepresentationKind, SymbolEmbeddingRow,
};
use crate::capability_packs::semantic_clones::{
    SearchDocumentRow, build_postgres_symbol_embedding_persist_sql,
    build_search_document_persist_sql, build_sqlite_symbol_embedding_persist_sql,
};
use crate::config::resolve_store_backend_config_for_repo;
use crate::host::devql::{
    DevqlConfig, RelationalDialect, RelationalStorage, RelationalStorageRole, esc_pg,
    sql_string_list_pg,
};

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
    let statements = if relational.has_remote_shared_relational_authority() {
        build_shared_snapshot_statements_from_local_current(
            relational,
            &repo_id,
            &commit_sha,
            &current_scope_predicate,
            &clone_scope_predicate,
        )
        .await?
    } else {
        build_local_shared_snapshot_statements(
            &repo_id,
            &commit_sha,
            &current_scope_predicate,
            &clone_scope_predicate,
        )
    };
    if statements.is_empty() {
        return Ok(());
    }
    relational
        .exec_batch_transactional_for_role(RelationalStorageRole::SharedRelational, &statements)
        .await
        .context("snapshotting committed current rows into shared relational storage")?;
    Ok(())
}

fn build_local_shared_snapshot_statements(
    repo_id: &str,
    commit_sha: &str,
    current_scope_predicate: &str,
    clone_scope_predicate: &str,
) -> Vec<String> {
    let repo_id = esc_pg(repo_id);
    let commit_sha = esc_pg(commit_sha);
    vec![
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
    ]
}

async fn build_shared_snapshot_statements_from_local_current(
    relational: &RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
    current_scope_predicate: &str,
    clone_scope_predicate: &str,
) -> Result<Vec<String>> {
    let shared_dialect = relational.dialect_for_role(RelationalStorageRole::SharedRelational);
    let repo_id_sql = esc_pg(repo_id);
    let commit_sha_sql = esc_pg(commit_sha);
    let mut statements = Vec::new();

    let file_state_rows = relational
        .query_rows(&format!(
            "SELECT c.repo_id, c.path, c.effective_content_id AS blob_sha \
             FROM current_file_state c \
             WHERE c.repo_id = '{repo_id}' \
               AND c.effective_source = 'head' \
               AND c.effective_content_id IS NOT NULL \
               {current_scope_predicate}",
            repo_id = repo_id_sql,
            current_scope_predicate = current_scope_predicate,
        ))
        .await?;
    for row in &file_state_rows {
        statements.push(build_file_state_upsert_sql(
            row_required_str(row, "repo_id")?,
            commit_sha,
            row_required_str(row, "path")?,
            row_required_str(row, "blob_sha")?,
        ));
    }

    let artefact_rows = relational
        .query_rows(&format!(
            "SELECT
                a.artefact_id,
                a.symbol_id,
                a.repo_id,
                a.language,
                a.extraction_fingerprint,
                a.canonical_kind,
                a.language_kind,
                a.symbol_fqn,
                a.signature,
                a.modifiers,
                a.docstring,
                a.content_id AS content_hash
             FROM artefacts_current a
             JOIN current_file_state c
               ON c.repo_id = a.repo_id
              AND c.path = a.path
              AND c.effective_source = 'head'
              AND c.effective_content_id = a.content_id
             WHERE a.repo_id = '{repo_id}'
               {current_scope_predicate}",
            repo_id = repo_id_sql,
            current_scope_predicate = current_scope_predicate,
        ))
        .await?;
    for row in &artefact_rows {
        statements.push(build_shared_artefact_upsert_sql(shared_dialect, row)?);
    }

    let artefact_snapshot_rows = relational
        .query_rows(&format!(
            "SELECT
                a.repo_id,
                a.content_id AS blob_sha,
                a.path,
                a.artefact_id,
                a.parent_artefact_id,
                a.start_line,
                a.end_line,
                a.start_byte,
                a.end_byte
             FROM artefacts_current a
             JOIN current_file_state c
               ON c.repo_id = a.repo_id
              AND c.path = a.path
              AND c.effective_source = 'head'
              AND c.effective_content_id = a.content_id
             WHERE a.repo_id = '{repo_id}'
               {current_scope_predicate}",
            repo_id = repo_id_sql,
            current_scope_predicate = current_scope_predicate,
        ))
        .await?;
    for row in &artefact_snapshot_rows {
        statements.push(build_shared_artefact_snapshot_upsert_sql(row)?);
    }

    let semantic_rows = relational
        .query_rows(&format!(
            "SELECT
                s.artefact_id,
                s.repo_id,
                s.content_id AS blob_sha,
                s.semantic_features_input_hash,
                s.docstring_summary,
                s.llm_summary,
                s.template_summary,
                s.summary,
                s.confidence,
                s.source_model
             FROM symbol_semantics_current s
             JOIN current_file_state c
               ON c.repo_id = s.repo_id
              AND c.path = s.path
              AND c.effective_source = 'head'
              AND c.effective_content_id = s.content_id
             WHERE s.repo_id = '{repo_id}'
               {current_scope_predicate}",
            repo_id = repo_id_sql,
            current_scope_predicate = current_scope_predicate,
        ))
        .await?;
    for row in &semantic_rows {
        statements.push(build_shared_symbol_semantics_upsert_sql(row)?);
    }

    let feature_rows = relational
        .query_rows(&format!(
            "SELECT
                f.artefact_id,
                f.repo_id,
                f.content_id AS blob_sha,
                f.semantic_features_input_hash,
                f.normalized_name,
                f.normalized_signature,
                f.modifiers,
                f.identifier_tokens,
                f.normalized_body_tokens,
                f.parent_kind,
                f.context_tokens
             FROM symbol_features_current f
             JOIN current_file_state c
               ON c.repo_id = f.repo_id
              AND c.path = f.path
              AND c.effective_source = 'head'
              AND c.effective_content_id = f.content_id
             WHERE f.repo_id = '{repo_id}'
               {current_scope_predicate}",
            repo_id = repo_id_sql,
            current_scope_predicate = current_scope_predicate,
        ))
        .await?;
    for row in &feature_rows {
        statements.push(build_shared_symbol_features_upsert_sql(
            shared_dialect,
            row,
        )?);
    }

    let search_document_rows = relational
        .query_rows(&format!(
            "SELECT
                d.artefact_id,
                d.repo_id,
                d.content_id AS blob_sha,
                d.path,
                d.symbol_id,
                d.signature_text,
                d.summary_text,
                d.body_text,
                d.searchable_text
             FROM symbol_search_documents_current d
             JOIN current_file_state c
               ON c.repo_id = d.repo_id
              AND c.path = d.path
              AND c.effective_source = 'head'
              AND c.effective_content_id = d.content_id
             WHERE d.repo_id = '{repo_id}'
               {current_scope_predicate}",
            repo_id = repo_id_sql,
            current_scope_predicate = current_scope_predicate,
        ))
        .await?;
    for row in &search_document_rows {
        statements.push(build_shared_search_document_upsert_sql(
            shared_dialect,
            row,
        )?);
    }

    let embedding_rows = relational
        .query_rows(&format!(
            "SELECT
                e.artefact_id,
                e.repo_id,
                e.content_id AS blob_sha,
                e.representation_kind,
                e.setup_fingerprint,
                e.provider,
                e.model,
                e.dimension,
                e.embedding_input_hash,
                e.embedding
             FROM symbol_embeddings_current e
             JOIN current_file_state c
               ON c.repo_id = e.repo_id
              AND c.path = e.path
              AND c.effective_source = 'head'
              AND c.effective_content_id = e.content_id
             WHERE e.repo_id = '{repo_id}'
               {current_scope_predicate}",
            repo_id = repo_id_sql,
            current_scope_predicate = current_scope_predicate,
        ))
        .await?;
    for row in &embedding_rows {
        statements.push(build_shared_symbol_embedding_upsert_sql(
            shared_dialect,
            row,
        )?);
    }

    let clone_edge_rows = relational
        .query_rows(&format!(
            "SELECT
                ce.repo_id,
                ce.source_symbol_id,
                ce.source_artefact_id,
                ce.target_symbol_id,
                ce.target_artefact_id,
                ce.relation_kind,
                ce.score,
                ce.semantic_score,
                ce.lexical_score,
                ce.structural_score,
                ce.clone_input_hash,
                ce.explanation_json
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
               {clone_scope_predicate}",
            repo_id = repo_id_sql,
            clone_scope_predicate = clone_scope_predicate,
        ))
        .await?;
    for row in &clone_edge_rows {
        statements.push(build_shared_symbol_clone_edge_upsert_sql(
            shared_dialect,
            row,
        )?);
    }

    let _ = commit_sha_sql;
    Ok(statements)
}

fn build_file_state_upsert_sql(
    repo_id: &str,
    commit_sha: &str,
    path: &str,
    blob_sha: &str,
) -> String {
    format!(
        "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES ('{repo_id}', '{commit_sha}', '{path}', '{blob_sha}') \
         ON CONFLICT (repo_id, commit_sha, path) DO UPDATE SET blob_sha = EXCLUDED.blob_sha",
        repo_id = esc_pg(repo_id),
        commit_sha = esc_pg(commit_sha),
        path = esc_pg(path),
        blob_sha = esc_pg(blob_sha),
    )
}

fn build_shared_artefact_upsert_sql(dialect: RelationalDialect, row: &Value) -> Result<String> {
    Ok(format!(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, language, extraction_fingerprint, canonical_kind,
            language_kind, symbol_fqn, signature, modifiers, docstring, content_hash
         ) VALUES (
            '{artefact_id}', {symbol_id}, '{repo_id}', '{language}', {extraction_fingerprint},
            {canonical_kind}, '{language_kind}', {symbol_fqn}, {signature}, {modifiers},
            {docstring}, '{content_hash}'
         )
         ON CONFLICT (artefact_id) DO UPDATE SET
            symbol_id = EXCLUDED.symbol_id,
            repo_id = EXCLUDED.repo_id,
            language = EXCLUDED.language,
            extraction_fingerprint = EXCLUDED.extraction_fingerprint,
            canonical_kind = EXCLUDED.canonical_kind,
            language_kind = EXCLUDED.language_kind,
            symbol_fqn = EXCLUDED.symbol_fqn,
            signature = EXCLUDED.signature,
            modifiers = EXCLUDED.modifiers,
            docstring = EXCLUDED.docstring,
            content_hash = EXCLUDED.content_hash",
        artefact_id = esc_pg(row_required_str(row, "artefact_id")?),
        symbol_id = sql_nullable_text(row_optional_str(row, "symbol_id")),
        repo_id = esc_pg(row_required_str(row, "repo_id")?),
        language = esc_pg(row_required_str(row, "language")?),
        extraction_fingerprint = sql_nullable_text(row_optional_str(row, "extraction_fingerprint")),
        canonical_kind = sql_nullable_text(row_optional_str(row, "canonical_kind")),
        language_kind = esc_pg(row_required_str(row, "language_kind")?),
        symbol_fqn = sql_nullable_text(row_optional_str(row, "symbol_fqn")),
        signature = sql_nullable_text(row_optional_str(row, "signature")),
        modifiers = sql_json_text_literal(dialect, &row_json_text(row, "modifiers", "[]")?),
        docstring = sql_nullable_text(row_optional_str(row, "docstring")),
        content_hash = esc_pg(row_required_str(row, "content_hash")?),
    ))
}

fn build_shared_artefact_snapshot_upsert_sql(row: &Value) -> Result<String> {
    Ok(format!(
        "INSERT INTO artefact_snapshots (
            repo_id, blob_sha, path, artefact_id, parent_artefact_id, start_line, end_line,
            start_byte, end_byte
         ) VALUES (
            '{repo_id}', '{blob_sha}', '{path}', '{artefact_id}', {parent_artefact_id},
            {start_line}, {end_line}, {start_byte}, {end_byte}
         )
         ON CONFLICT (repo_id, blob_sha, artefact_id) DO UPDATE SET
            path = EXCLUDED.path,
            parent_artefact_id = EXCLUDED.parent_artefact_id,
            start_line = EXCLUDED.start_line,
            end_line = EXCLUDED.end_line,
            start_byte = EXCLUDED.start_byte,
            end_byte = EXCLUDED.end_byte",
        repo_id = esc_pg(row_required_str(row, "repo_id")?),
        blob_sha = esc_pg(row_required_str(row, "blob_sha")?),
        path = esc_pg(row_required_str(row, "path")?),
        artefact_id = esc_pg(row_required_str(row, "artefact_id")?),
        parent_artefact_id = sql_nullable_text(row_optional_str(row, "parent_artefact_id")),
        start_line = row_required_i64(row, "start_line")?,
        end_line = row_required_i64(row, "end_line")?,
        start_byte = row_required_i64(row, "start_byte")?,
        end_byte = row_required_i64(row, "end_byte")?,
    ))
}

fn build_shared_symbol_semantics_upsert_sql(row: &Value) -> Result<String> {
    Ok(format!(
        "INSERT INTO symbol_semantics (
            artefact_id, repo_id, blob_sha, semantic_features_input_hash, docstring_summary,
            llm_summary, template_summary, summary, confidence, source_model
         ) VALUES (
            '{artefact_id}', '{repo_id}', '{blob_sha}', '{semantic_features_input_hash}',
            {docstring_summary}, {llm_summary}, '{template_summary}', '{summary}', {confidence},
            {source_model}
         )
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
        artefact_id = esc_pg(row_required_str(row, "artefact_id")?),
        repo_id = esc_pg(row_required_str(row, "repo_id")?),
        blob_sha = esc_pg(row_required_str(row, "blob_sha")?),
        semantic_features_input_hash =
            esc_pg(row_required_str(row, "semantic_features_input_hash")?),
        docstring_summary = sql_nullable_text(row_optional_str(row, "docstring_summary")),
        llm_summary = sql_nullable_text(row_optional_str(row, "llm_summary")),
        template_summary = esc_pg(row_required_str(row, "template_summary")?),
        summary = esc_pg(row_required_str(row, "summary")?),
        confidence = sql_nullable_f32(row_optional_f32(row, "confidence")),
        source_model = sql_nullable_text(row_optional_str(row, "source_model")),
    ))
}

fn build_shared_symbol_features_upsert_sql(
    dialect: RelationalDialect,
    row: &Value,
) -> Result<String> {
    Ok(format!(
        "INSERT INTO symbol_features (
            artefact_id, repo_id, blob_sha, semantic_features_input_hash, normalized_name,
            normalized_signature, modifiers, identifier_tokens, normalized_body_tokens,
            parent_kind, context_tokens
         ) VALUES (
            '{artefact_id}', '{repo_id}', '{blob_sha}', '{semantic_features_input_hash}',
            '{normalized_name}', {normalized_signature}, {modifiers}, {identifier_tokens},
            {normalized_body_tokens}, {parent_kind}, {context_tokens}
         )
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
        artefact_id = esc_pg(row_required_str(row, "artefact_id")?),
        repo_id = esc_pg(row_required_str(row, "repo_id")?),
        blob_sha = esc_pg(row_required_str(row, "blob_sha")?),
        semantic_features_input_hash =
            esc_pg(row_required_str(row, "semantic_features_input_hash")?),
        normalized_name = esc_pg(row_required_str(row, "normalized_name")?),
        normalized_signature = sql_nullable_text(row_optional_str(row, "normalized_signature")),
        modifiers = sql_json_text_literal(dialect, &row_json_text(row, "modifiers", "[]")?),
        identifier_tokens =
            sql_json_text_literal(dialect, &row_json_text(row, "identifier_tokens", "[]")?),
        normalized_body_tokens = sql_json_text_literal(
            dialect,
            &row_json_text(row, "normalized_body_tokens", "[]")?,
        ),
        parent_kind = sql_nullable_text(row_optional_str(row, "parent_kind")),
        context_tokens =
            sql_json_text_literal(dialect, &row_json_text(row, "context_tokens", "[]")?),
    ))
}

fn build_shared_search_document_upsert_sql(
    dialect: RelationalDialect,
    row: &Value,
) -> Result<String> {
    let search_row = SearchDocumentRow {
        artefact_id: row_required_str(row, "artefact_id")?.to_string(),
        repo_id: row_required_str(row, "repo_id")?.to_string(),
        blob_sha: row_required_str(row, "blob_sha")?.to_string(),
        path: row_required_str(row, "path")?.to_string(),
        symbol_id: row_optional_str(row, "symbol_id").map(str::to_string),
        signature_text: row_optional_str(row, "signature_text").map(str::to_string),
        summary_text: row_optional_str(row, "summary_text").map(str::to_string),
        body_text: row_required_str(row, "body_text")?.to_string(),
        searchable_text: row_required_str(row, "searchable_text")?.to_string(),
    };
    Ok(build_search_document_persist_sql(&search_row, dialect))
}

fn build_shared_symbol_embedding_upsert_sql(
    dialect: RelationalDialect,
    row: &Value,
) -> Result<String> {
    let embedding_row = SymbolEmbeddingRow {
        artefact_id: row_required_str(row, "artefact_id")?.to_string(),
        repo_id: row_required_str(row, "repo_id")?.to_string(),
        blob_sha: row_required_str(row, "blob_sha")?.to_string(),
        representation_kind: parse_representation_kind(row_required_str(
            row,
            "representation_kind",
        )?),
        setup_fingerprint: row_required_str(row, "setup_fingerprint")?.to_string(),
        provider: row_required_str(row, "provider")?.to_string(),
        model: row_required_str(row, "model")?.to_string(),
        dimension: row_required_usize(row, "dimension")?,
        embedding_input_hash: row_required_str(row, "embedding_input_hash")?.to_string(),
        embedding: row_required_embedding(row, "embedding")?,
    };
    match dialect {
        RelationalDialect::Postgres => build_postgres_symbol_embedding_persist_sql(&embedding_row),
        RelationalDialect::Sqlite => build_sqlite_symbol_embedding_persist_sql(&embedding_row),
    }
}

fn build_shared_symbol_clone_edge_upsert_sql(
    dialect: RelationalDialect,
    row: &Value,
) -> Result<String> {
    Ok(format!(
        "INSERT INTO symbol_clone_edges (
            repo_id, source_symbol_id, source_artefact_id, target_symbol_id,
            target_artefact_id, relation_kind, score, semantic_score, lexical_score,
            structural_score, clone_input_hash, explanation_json
         ) VALUES (
            '{repo_id}', '{source_symbol_id}', '{source_artefact_id}', '{target_symbol_id}',
            '{target_artefact_id}', '{relation_kind}', {score}, {semantic_score},
            {lexical_score}, {structural_score}, '{clone_input_hash}', {explanation_json}
         )
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
        repo_id = esc_pg(row_required_str(row, "repo_id")?),
        source_symbol_id = esc_pg(row_required_str(row, "source_symbol_id")?),
        source_artefact_id = esc_pg(row_required_str(row, "source_artefact_id")?),
        target_symbol_id = esc_pg(row_required_str(row, "target_symbol_id")?),
        target_artefact_id = esc_pg(row_required_str(row, "target_artefact_id")?),
        relation_kind = esc_pg(row_required_str(row, "relation_kind")?),
        score = row_required_f64(row, "score")?,
        semantic_score = row_required_f64(row, "semantic_score")?,
        lexical_score = row_required_f64(row, "lexical_score")?,
        structural_score = row_required_f64(row, "structural_score")?,
        clone_input_hash = esc_pg(row_required_str(row, "clone_input_hash")?),
        explanation_json =
            sql_json_text_literal(dialect, &row_json_text(row, "explanation_json", "{}")?),
    ))
}

fn row_required_str<'a>(row: &'a Value, key: &str) -> Result<&'a str> {
    row.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("snapshot row is missing required string column `{key}`"))
}

fn row_optional_str<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn row_required_i64(row: &Value, key: &str) -> Result<i64> {
    row.get(key)
        .and_then(Value::as_i64)
        .or_else(|| {
            row.get(key)
                .and_then(Value::as_u64)
                .and_then(|value| i64::try_from(value).ok())
        })
        .or_else(|| {
            row.get(key)
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<i64>().ok())
        })
        .ok_or_else(|| anyhow!("snapshot row is missing required integer column `{key}`"))
}

fn row_required_usize(row: &Value, key: &str) -> Result<usize> {
    usize::try_from(row_required_i64(row, key)?)
        .map_err(|_| anyhow!("snapshot row column `{key}` contains a negative value"))
}

fn row_optional_f32(row: &Value, key: &str) -> Option<f32> {
    row.get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .or_else(|| {
            row.get(key)
                .and_then(Value::as_i64)
                .map(|value| value as f32)
        })
        .or_else(|| {
            row.get(key)
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<f32>().ok())
        })
}

fn row_required_f64(row: &Value, key: &str) -> Result<f64> {
    row.get(key)
        .and_then(Value::as_f64)
        .or_else(|| {
            row.get(key)
                .and_then(Value::as_i64)
                .map(|value| value as f64)
        })
        .or_else(|| {
            row.get(key)
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<f64>().ok())
        })
        .ok_or_else(|| anyhow!("snapshot row is missing required numeric column `{key}`"))
}

fn row_json_text(row: &Value, key: &str, default: &str) -> Result<String> {
    Ok(match row.get(key) {
        Some(Value::String(raw)) if !raw.is_empty() => raw.clone(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => row
            .get(key)
            .map(Value::to_string)
            .unwrap_or_else(|| default.to_string()),
        Some(Value::Null) | None => default.to_string(),
        Some(other) => other.to_string(),
    })
}

fn row_required_embedding(row: &Value, key: &str) -> Result<Vec<f32>> {
    match row.get(key) {
        Some(Value::String(raw)) => serde_json::from_str::<Vec<f32>>(raw)
            .with_context(|| format!("parsing embedding JSON from snapshot column `{key}`")),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_f64()
                    .map(|number| number as f32)
                    .or_else(|| value.as_i64().map(|number| number as f32))
                    .ok_or_else(|| anyhow!("embedding value in column `{key}` is not numeric"))
            })
            .collect(),
        _ => Err(anyhow!(
            "snapshot row is missing required embedding column `{key}`"
        )),
    }
}

fn sql_nullable_text(value: Option<&str>) -> String {
    value
        .map(|text| format!("'{}'", esc_pg(text)))
        .unwrap_or_else(|| "NULL".to_string())
}

fn sql_nullable_f32(value: Option<f32>) -> String {
    value
        .map(|number| number.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

fn sql_json_text_literal(dialect: RelationalDialect, raw_json: &str) -> String {
    let escaped = esc_pg(raw_json);
    match dialect {
        RelationalDialect::Postgres => format!("'{escaped}'::jsonb"),
        RelationalDialect::Sqlite => format!("'{escaped}'"),
    }
}

fn parse_representation_kind(raw: &str) -> EmbeddingRepresentationKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "summary" => EmbeddingRepresentationKind::Summary,
        "identity" | "locator" => EmbeddingRepresentationKind::Identity,
        _ => EmbeddingRepresentationKind::Code,
    }
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
