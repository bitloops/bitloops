use anyhow::{Result, bail};

use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::{esc_pg, sql_string_list_pg};

pub(super) fn build_symbol_embedding_index_state_sql(
    artefact_id: &str,
    table: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup_fingerprint: &str,
) -> String {
    format!(
        "SELECT embedding_input_hash AS embedding_hash \
FROM {table} \
WHERE artefact_id = '{artefact_id}' AND representation_kind = '{representation_kind}' \
  AND setup_fingerprint = '{setup_fingerprint}'",
        table = table,
        artefact_id = esc_pg(artefact_id),
        representation_kind = esc_pg(&representation_kind.to_string()),
        setup_fingerprint = esc_pg(setup_fingerprint),
    )
}

pub(super) fn build_active_embedding_setup_lookup_sql(
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> String {
    format!(
        "SELECT representation_kind, provider, model, dimension, setup_fingerprint \
FROM semantic_clone_embedding_setup_state \
WHERE repo_id = '{repo_id}' AND {representation_predicate}",
        repo_id = esc_pg(repo_id),
        representation_predicate =
            representation_kind_sql_predicate("representation_kind", representation_kind),
    )
}

pub(super) fn build_current_repo_embedding_states_sql(
    repo_id: &str,
    representation_kind: Option<embeddings::EmbeddingRepresentationKind>,
) -> String {
    let representation_filter = representation_kind
        .map(|kind| {
            format!(
                "AND {}",
                representation_kind_sql_predicate("e.representation_kind", kind)
            )
        })
        .unwrap_or_default();
    format!(
        "SELECT representation_kind, provider, model, dimension, setup_fingerprint \
FROM ( \
    SELECT e.representation_kind AS representation_kind, e.provider AS provider, e.model AS model, e.dimension AS dimension, e.setup_fingerprint AS setup_fingerprint \
    FROM artefacts_current a \
    JOIN symbol_embeddings_current e ON e.repo_id = a.repo_id AND e.artefact_id = a.artefact_id AND e.content_id = a.content_id \
    WHERE a.repo_id = '{repo_id}' {representation_filter} \
    UNION \
    SELECT e.representation_kind AS representation_kind, e.provider AS provider, e.model AS model, e.dimension AS dimension, e.setup_fingerprint AS setup_fingerprint \
    FROM artefacts_current a \
    JOIN symbol_embeddings e ON e.repo_id = a.repo_id AND e.artefact_id = a.artefact_id \
    WHERE a.repo_id = '{repo_id}' {representation_filter} \
) setups \
ORDER BY representation_kind, provider, model, dimension, setup_fingerprint",
        repo_id = esc_pg(repo_id),
        representation_filter = representation_filter,
    )
}

pub(super) fn build_current_repo_semantic_clone_coverage_sql(
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup: &embeddings::EmbeddingSetup,
) -> String {
    format!(
        "SELECT \
            (SELECT COUNT(*) FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             WHERE a.repo_id = '{repo_id}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import') AS eligible_current_artefacts, \
            (SELECT COUNT(DISTINCT a.artefact_id) FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             JOIN symbol_semantics_current ss ON ss.repo_id = a.repo_id AND ss.artefact_id = a.artefact_id AND ss.content_id = a.content_id \
             JOIN symbol_features_current sf ON sf.repo_id = a.repo_id AND sf.artefact_id = a.artefact_id AND sf.content_id = a.content_id \
             JOIN symbol_embeddings_current e ON e.repo_id = a.repo_id AND e.artefact_id = a.artefact_id AND e.content_id = a.content_id \
             WHERE a.repo_id = '{repo_id}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
               AND {representation_predicate} \
               AND e.provider = '{provider}' \
               AND e.model = '{model}' \
               AND e.dimension = {dimension}) AS fully_indexed_current_artefacts",
        repo_id = esc_pg(repo_id),
        representation_predicate =
            representation_kind_sql_predicate("e.representation_kind", representation_kind),
        provider = esc_pg(&setup.provider),
        model = esc_pg(&setup.model),
        dimension = setup.dimension,
    )
}

pub(super) fn build_semantic_summary_lookup_sql(artefact_ids: &[String], table: &str) -> String {
    format!(
        "SELECT artefact_id, docstring_summary, llm_summary, template_summary, summary, source_model \
FROM {table} \
WHERE artefact_id IN ({})",
        sql_string_list_pg(artefact_ids),
        table = table,
    )
}

pub(crate) fn build_active_embedding_setup_persist_sql(
    repo_id: &str,
    active_state: &embeddings::ActiveEmbeddingRepresentationState,
) -> String {
    let setup = &active_state.setup;
    format!(
        "INSERT INTO semantic_clone_embedding_setup_state (repo_id, representation_kind, provider, model, dimension, setup_fingerprint) \
VALUES ('{repo_id}', '{representation_kind}', '{provider}', '{model}', {dimension}, '{setup_fingerprint}') \
ON CONFLICT (repo_id, representation_kind) DO UPDATE SET provider = excluded.provider, model = excluded.model, dimension = excluded.dimension, setup_fingerprint = excluded.setup_fingerprint, updated_at = CURRENT_TIMESTAMP",
        repo_id = esc_pg(repo_id),
        representation_kind = esc_pg(&active_state.representation_kind.to_string()),
        provider = esc_pg(&setup.provider),
        model = esc_pg(&setup.model),
        dimension = setup.dimension,
        setup_fingerprint = esc_pg(&setup.setup_fingerprint),
    )
}

#[cfg(test)]
pub(super) fn build_postgres_symbol_embedding_persist_sql(
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<String> {
    let embedding_expr = sql_vector_string(&row.embedding)?;
    Ok(format!(
        "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, representation_kind, setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{representation_kind}', '{setup_fingerprint}', '{provider}', '{model}', {dimension}, '{embedding_input_hash}', {embedding}) \
ON CONFLICT (artefact_id, representation_kind, setup_fingerprint) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, provider = EXCLUDED.provider, model = EXCLUDED.model, dimension = EXCLUDED.dimension, embedding_input_hash = EXCLUDED.embedding_input_hash, embedding = EXCLUDED.embedding, generated_at = now()",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        blob_sha = esc_pg(&row.blob_sha),
        representation_kind = esc_pg(&row.representation_kind.to_string()),
        setup_fingerprint = esc_pg(&row.setup_fingerprint),
        provider = esc_pg(&row.provider),
        model = esc_pg(&row.model),
        dimension = row.dimension,
        embedding_input_hash = esc_pg(&row.embedding_input_hash),
        embedding = embedding_expr,
    ))
}

pub(crate) fn build_sqlite_symbol_embedding_persist_sql(
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<String> {
    let embedding_json = sql_json_string(&row.embedding)?;
    Ok(format!(
        "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, representation_kind, setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{representation_kind}', '{setup_fingerprint}', '{provider}', '{model}', {dimension}, '{embedding_input_hash}', '{embedding}') \
ON CONFLICT (artefact_id, representation_kind, setup_fingerprint) DO UPDATE SET repo_id = excluded.repo_id, blob_sha = excluded.blob_sha, provider = excluded.provider, model = excluded.model, dimension = excluded.dimension, embedding_input_hash = excluded.embedding_input_hash, embedding = excluded.embedding, generated_at = CURRENT_TIMESTAMP",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        blob_sha = esc_pg(&row.blob_sha),
        representation_kind = esc_pg(&row.representation_kind.to_string()),
        setup_fingerprint = esc_pg(&row.setup_fingerprint),
        provider = esc_pg(&row.provider),
        model = esc_pg(&row.model),
        dimension = row.dimension,
        embedding_input_hash = esc_pg(&row.embedding_input_hash),
        embedding = embedding_json,
    ))
}

pub(crate) fn build_current_symbol_embedding_persist_sql(
    input: &semantic::SemanticFeatureInput,
    path: &str,
    content_id: &str,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<String> {
    let embedding_json = sql_json_string(&row.embedding)?;
    let symbol_id_sql = input
        .symbol_id
        .as_deref()
        .map(|value| format!("'{}'", esc_pg(value)))
        .unwrap_or_else(|| "NULL".to_string());
    Ok(format!(
        "INSERT INTO symbol_embeddings_current (artefact_id, repo_id, path, content_id, symbol_id, representation_kind, setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding) \
VALUES ('{artefact_id}', '{repo_id}', '{path}', '{content_id}', {symbol_id}, '{representation_kind}', '{setup_fingerprint}', '{provider}', '{model}', {dimension}, '{embedding_input_hash}', '{embedding}') \
ON CONFLICT (artefact_id, representation_kind, setup_fingerprint) DO UPDATE SET repo_id = excluded.repo_id, path = excluded.path, content_id = excluded.content_id, symbol_id = excluded.symbol_id, provider = excluded.provider, model = excluded.model, dimension = excluded.dimension, embedding_input_hash = excluded.embedding_input_hash, embedding = excluded.embedding, generated_at = CURRENT_TIMESTAMP",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        path = esc_pg(path),
        content_id = esc_pg(content_id),
        symbol_id = symbol_id_sql,
        representation_kind = esc_pg(&row.representation_kind.to_string()),
        setup_fingerprint = esc_pg(&row.setup_fingerprint),
        provider = esc_pg(&row.provider),
        model = esc_pg(&row.model),
        dimension = row.dimension,
        embedding_input_hash = esc_pg(&row.embedding_input_hash),
        embedding = embedding_json,
    ))
}

pub(crate) fn build_embedding_setup_persist_sql(setup: &embeddings::EmbeddingSetup) -> String {
    format!(
        "INSERT INTO semantic_embedding_setups (setup_fingerprint, provider, model, dimension) \
VALUES ('{setup_fingerprint}', '{provider}', '{model}', {dimension}) \
ON CONFLICT (setup_fingerprint) DO UPDATE SET provider = excluded.provider, model = excluded.model, dimension = excluded.dimension",
        setup_fingerprint = esc_pg(&setup.setup_fingerprint),
        provider = esc_pg(&setup.provider),
        model = esc_pg(&setup.model),
        dimension = setup.dimension,
    )
}

pub(super) fn representation_kind_sql_predicate(
    column: &str,
    kind: embeddings::EmbeddingRepresentationKind,
) -> String {
    let values = kind
        .storage_values()
        .iter()
        .map(|value| format!("'{}'", esc_pg(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{column} IN ({values})")
}

#[cfg(test)]
pub(super) fn sql_vector_string(values: &[f32]) -> Result<String> {
    let json = sql_json_string(values)?;
    Ok(format!("'{json}'::vector"))
}

pub(super) fn sql_json_string(values: &[f32]) -> Result<String> {
    if values.is_empty() {
        bail!("cannot persist empty embedding vector");
    }

    for value in values {
        if !value.is_finite() {
            bail!("cannot persist embedding vector containing non-finite values");
        }
    }

    Ok(esc_pg(&serde_json::to_string(values)?))
}
