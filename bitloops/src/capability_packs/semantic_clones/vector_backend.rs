use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;

use crate::capability_packs::semantic_clones::embeddings;
use crate::host::devql::{RelationalPrimaryBackend, RelationalStorage, esc_pg, sql_string_list_pg};

const SQLITE_CURRENT_VEC_TABLE_PREFIX: &str = "semantic_embedding_current_vec_dim_";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SemanticVectorBackendKind {
    PostgresPgvector,
    SqliteVec,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SemanticNearestCandidate {
    pub artefact_id: String,
    pub distance: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SemanticVectorQuery<'a> {
    pub repo_id: &'a str,
    pub representation_kind: embeddings::EmbeddingRepresentationKind,
    pub setup_fingerprint: &'a str,
    pub dimension: usize,
    pub query_embedding: &'a [f32],
    pub limit: usize,
}

pub(crate) fn resolve_semantic_vector_backend(
    relational: &RelationalStorage,
) -> SemanticVectorBackendKind {
    match relational.primary_backend() {
        RelationalPrimaryBackend::Postgres => SemanticVectorBackendKind::PostgresPgvector,
        RelationalPrimaryBackend::Sqlite => SemanticVectorBackendKind::SqliteVec,
    }
}

pub(crate) async fn ensure_sqlite_current_vec_tables_for_existing_rows(
    relational: &RelationalStorage,
) -> Result<()> {
    for dimension in load_sqlite_current_vec_dimensions(relational).await? {
        ensure_sqlite_current_vec_table(relational, dimension).await?;
    }
    Ok(())
}

pub(crate) async fn sync_sqlite_current_symbol_embedding_row(
    relational: &RelationalStorage,
    path: &str,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<()> {
    let mut statements =
        build_sqlite_current_vec_table_init_statements(relational, row.dimension).await?;
    statements.extend(build_sqlite_current_vec_upsert_statements(path, row)?);
    relational
        .exec_serialized_batch_transactional(&statements)
        .await
}

pub(crate) async fn build_sqlite_current_vec_table_init_statements(
    relational: &RelationalStorage,
    dimension: usize,
) -> Result<Vec<String>> {
    let table_name = sqlite_current_vec_table_name(dimension);
    let exists_rows = relational
        .query_rows(&format!(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '{table_name}'",
            table_name = esc_pg(&table_name),
        ))
        .await?;
    if !exists_rows.is_empty() {
        return Ok(Vec::new());
    }

    Ok(vec![
        format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS {table_name} USING vec0( \
                 embedding float[{dimension}] distance_metric=cosine, \
                 repo_id TEXT partition key, \
                 representation_kind TEXT partition key, \
                 setup_fingerprint TEXT partition key, \
                 artefact_id TEXT, \
                 path TEXT \
             )",
        ),
        format!(
            "INSERT INTO {table_name} (embedding, repo_id, representation_kind, setup_fingerprint, artefact_id, path) \
             SELECT vec_f32(embedding), repo_id, representation_kind, setup_fingerprint, artefact_id, path \
             FROM symbol_embeddings_current \
             WHERE dimension = {dimension}",
        ),
    ])
}

pub(crate) fn build_sqlite_current_vec_upsert_statements(
    path: &str,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<Vec<String>> {
    let table_name = sqlite_current_vec_table_name(row.dimension);
    Ok(vec![
        format!(
            "DELETE FROM {table_name} \
             WHERE repo_id = '{repo_id}' \
               AND representation_kind = '{representation_kind}' \
               AND setup_fingerprint = '{setup_fingerprint}' \
               AND artefact_id = '{artefact_id}'",
            repo_id = esc_pg(&row.repo_id),
            representation_kind = esc_pg(&row.representation_kind.to_string()),
            setup_fingerprint = esc_pg(&row.setup_fingerprint),
            artefact_id = esc_pg(&row.artefact_id),
        ),
        format!(
            "INSERT INTO {table_name} (embedding, repo_id, representation_kind, setup_fingerprint, artefact_id, path) \
             VALUES (vec_f32('{embedding_json}'), '{repo_id}', '{representation_kind}', '{setup_fingerprint}', '{artefact_id}', '{path}')",
            embedding_json = vector_json_string(&row.embedding)?,
            repo_id = esc_pg(&row.repo_id),
            representation_kind = esc_pg(&row.representation_kind.to_string()),
            setup_fingerprint = esc_pg(&row.setup_fingerprint),
            artefact_id = esc_pg(&row.artefact_id),
            path = esc_pg(path),
        ),
    ])
}

pub(crate) async fn build_sqlite_stale_current_rows_for_path_delete_statements(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    keep_artefact_ids: &[String],
) -> Result<Vec<String>> {
    let extra_delete_clause = if keep_artefact_ids.is_empty() {
        "1 = 1".to_string()
    } else {
        format!(
            "artefact_id NOT IN ({})",
            sql_string_list_pg(keep_artefact_ids)
        )
    };
    load_sqlite_current_vec_table_names(relational)
        .await
        .map(|table_names| {
            table_names
                .into_iter()
                .map(|table_name| {
                    format!(
                        "DELETE FROM {table_name} \
                         WHERE repo_id = '{repo_id}' AND path = '{path}' AND {representation_predicate} AND ({extra_delete_clause})",
                        repo_id = esc_pg(repo_id),
                        path = esc_pg(path),
                        representation_predicate = sqlite_representation_kind_predicate(
                            "representation_kind",
                            representation_kind,
                        ),
                        extra_delete_clause = extra_delete_clause,
                    )
                })
                .collect()
        })
}

pub(crate) async fn delete_sqlite_stale_current_rows_for_path(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    keep_artefact_ids: &[String],
) -> Result<()> {
    let statements = build_sqlite_stale_current_rows_for_path_delete_statements(
        relational,
        repo_id,
        path,
        representation_kind,
        keep_artefact_ids,
    )
    .await?;
    if statements.is_empty() {
        return Ok(());
    }
    relational
        .exec_serialized_batch_transactional(&statements)
        .await
}

pub(crate) async fn clear_sqlite_current_rows_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: &[String],
) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    execute_sqlite_vec_delete_across_tables(
        relational,
        &format!(
            "repo_id = '{repo_id}' AND path IN ({paths})",
            repo_id = esc_pg(repo_id),
            paths = sql_string_list_pg(paths),
        ),
    )
    .await
}

pub(crate) async fn clear_sqlite_repo_rows(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    execute_sqlite_vec_delete_across_tables(relational, &format!("repo_id = '{}'", esc_pg(repo_id)))
        .await
}

pub(crate) async fn clear_sqlite_repo_rows_for_representation(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<()> {
    execute_sqlite_vec_delete_across_tables(
        relational,
        &format!(
            "repo_id = '{repo_id}' AND {representation_predicate}",
            repo_id = esc_pg(repo_id),
            representation_predicate =
                sqlite_representation_kind_predicate("representation_kind", representation_kind),
        ),
    )
    .await
}

pub(crate) async fn nearest_current_candidates(
    relational: &RelationalStorage,
    query: SemanticVectorQuery<'_>,
) -> Result<Vec<SemanticNearestCandidate>> {
    match resolve_semantic_vector_backend(relational) {
        SemanticVectorBackendKind::SqliteVec => {
            ensure_sqlite_current_vec_table(relational, query.dimension).await?;
            load_sqlite_nearest_current_candidates(relational, query).await
        }
        SemanticVectorBackendKind::PostgresPgvector => {
            load_postgres_nearest_current_candidates(relational, query).await
        }
    }
}

pub(crate) async fn ensure_postgres_pgvector_indexes_for_dimension(
    relational: &RelationalStorage,
    dimension: usize,
) -> Result<()> {
    if resolve_semantic_vector_backend(relational) != SemanticVectorBackendKind::PostgresPgvector {
        return Ok(());
    }
    let statements = vec![
        build_postgres_pgvector_partial_index_sql("symbol_embeddings", dimension),
        build_postgres_pgvector_partial_index_sql("symbol_embeddings_current", dimension),
    ];
    relational
        .exec_remote_batch_transactional(&statements)
        .await
}

pub(crate) fn sqlite_current_vec_table_name(dimension: usize) -> String {
    format!("{SQLITE_CURRENT_VEC_TABLE_PREFIX}{dimension}")
}

pub(crate) fn postgres_current_pgvector_index_name(dimension: usize) -> String {
    format!("symbol_embeddings_current_embedding_cosine_d{dimension}_idx")
}

pub(crate) fn postgres_historical_pgvector_index_name(dimension: usize) -> String {
    format!("symbol_embeddings_embedding_cosine_d{dimension}_idx")
}

pub(crate) fn build_postgres_pgvector_partial_index_sql(table: &str, dimension: usize) -> String {
    let index_name = if table == "symbol_embeddings_current" {
        postgres_current_pgvector_index_name(dimension)
    } else {
        postgres_historical_pgvector_index_name(dimension)
    };
    format!(
        "CREATE INDEX IF NOT EXISTS {index_name} \
         ON {table} USING hnsw ((embedding::vector({dimension})) vector_cosine_ops) \
         WHERE dimension = {dimension}",
    )
}

pub(crate) fn build_postgres_nearest_current_candidates_sql(
    query: SemanticVectorQuery<'_>,
) -> Result<String> {
    let query_vector = format!("'{}'::vector", vector_json_string(query.query_embedding)?);
    Ok(format!(
        "SELECT artefact_id, (embedding::vector({dimension}) <=> {query_vector}) AS distance \
         FROM symbol_embeddings_current \
         WHERE repo_id = '{repo_id}' \
           AND {representation_predicate} \
           AND setup_fingerprint = '{setup_fingerprint}' \
           AND dimension = {dimension} \
         ORDER BY embedding::vector({dimension}) <=> {query_vector}, artefact_id \
         LIMIT {limit}",
        dimension = query.dimension,
        query_vector = query_vector,
        repo_id = esc_pg(query.repo_id),
        representation_predicate =
            sqlite_representation_kind_predicate("representation_kind", query.representation_kind),
        setup_fingerprint = esc_pg(query.setup_fingerprint),
        limit = query.limit.max(1),
    ))
}

async fn ensure_sqlite_current_vec_table(
    relational: &RelationalStorage,
    dimension: usize,
) -> Result<()> {
    let statements = build_sqlite_current_vec_table_init_statements(relational, dimension).await?;
    if statements.is_empty() {
        return Ok(());
    }
    relational
        .exec_serialized_batch_transactional(&statements)
        .await
}

async fn load_sqlite_current_vec_dimensions(relational: &RelationalStorage) -> Result<Vec<usize>> {
    let rows = relational
        .query_rows(
            "SELECT DISTINCT dimension FROM symbol_embeddings_current WHERE dimension > 0 ORDER BY dimension",
        )
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            row.get("dimension")
                .and_then(Value::as_u64)
                .or_else(|| {
                    row.get("dimension")
                        .and_then(Value::as_i64)
                        .map(|value| value.max(0) as u64)
                })
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0)
        })
        .collect())
}

async fn execute_sqlite_vec_delete_across_tables(
    relational: &RelationalStorage,
    predicate: &str,
) -> Result<()> {
    let mut statements = Vec::new();
    for table_name in load_sqlite_current_vec_table_names(relational).await? {
        statements.push(format!("DELETE FROM {table_name} WHERE {predicate}"));
    }
    if statements.is_empty() {
        return Ok(());
    }
    relational
        .exec_serialized_batch_transactional(&statements)
        .await
}

async fn load_sqlite_current_vec_table_names(
    relational: &RelationalStorage,
) -> Result<Vec<String>> {
    let prefix = esc_pg(SQLITE_CURRENT_VEC_TABLE_PREFIX);
    let rows = relational
        .query_rows(&format!(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' AND substr(name, 1, {prefix_len}) = '{prefix}' \
             ORDER BY name",
            prefix = prefix,
            prefix_len = SQLITE_CURRENT_VEC_TABLE_PREFIX.len(),
        ))
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.get("name").and_then(Value::as_str).map(str::to_string))
        .collect())
}

async fn load_sqlite_nearest_current_candidates(
    relational: &RelationalStorage,
    query: SemanticVectorQuery<'_>,
) -> Result<Vec<SemanticNearestCandidate>> {
    let table_name = sqlite_current_vec_table_name(query.dimension);
    let embedding_json = vector_json_string(query.query_embedding)?;
    let mut best_by_artefact = HashMap::<String, f64>::new();
    for storage_value in query.representation_kind.storage_values() {
        let sql = format!(
            "SELECT artefact_id, distance \
             FROM {table_name} \
             WHERE embedding MATCH '{embedding_json}' \
               AND k = {limit} \
               AND repo_id = '{repo_id}' \
               AND representation_kind = '{representation_kind}' \
               AND setup_fingerprint = '{setup_fingerprint}' \
             ORDER BY distance, artefact_id",
            table_name = table_name,
            embedding_json = embedding_json,
            limit = query.limit.max(1),
            repo_id = esc_pg(query.repo_id),
            representation_kind = esc_pg(storage_value),
            setup_fingerprint = esc_pg(query.setup_fingerprint),
        );
        for candidate in parse_nearest_candidates(relational.query_rows(&sql).await?) {
            best_by_artefact
                .entry(candidate.artefact_id)
                .and_modify(|distance| {
                    if candidate.distance < *distance {
                        *distance = candidate.distance;
                    }
                })
                .or_insert(candidate.distance);
        }
    }

    let mut candidates = best_by_artefact
        .into_iter()
        .map(|(artefact_id, distance)| SemanticNearestCandidate {
            artefact_id,
            distance,
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.distance
            .partial_cmp(&right.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.artefact_id.cmp(&right.artefact_id))
    });
    candidates.truncate(query.limit.max(1));
    Ok(candidates)
}

async fn load_postgres_nearest_current_candidates(
    relational: &RelationalStorage,
    query: SemanticVectorQuery<'_>,
) -> Result<Vec<SemanticNearestCandidate>> {
    let sql = build_postgres_nearest_current_candidates_sql(query)?;
    let rows = relational.query_rows_primary(&sql).await?;
    Ok(parse_nearest_candidates(rows))
}

fn parse_nearest_candidates(rows: Vec<Value>) -> Vec<SemanticNearestCandidate> {
    rows.into_iter()
        .filter_map(|row| {
            let artefact_id = row.get("artefact_id").and_then(Value::as_str)?.to_string();
            let distance = row
                .get("distance")
                .and_then(Value::as_f64)
                .or_else(|| {
                    row.get("distance")
                        .and_then(Value::as_i64)
                        .map(|value| value as f64)
                })
                .or_else(|| {
                    row.get("distance")
                        .and_then(Value::as_str)
                        .and_then(|value| value.parse::<f64>().ok())
                })?;
            Some(SemanticNearestCandidate {
                artefact_id,
                distance,
            })
        })
        .collect()
}

fn sqlite_representation_kind_predicate(
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

fn vector_json_string(values: &[f32]) -> Result<String> {
    anyhow::ensure!(!values.is_empty(), "cannot persist empty embedding vector");
    for value in values {
        anyhow::ensure!(
            value.is_finite(),
            "cannot persist embedding vector containing non-finite values"
        );
    }
    Ok(esc_pg(&serde_json::to_string(values)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_table_name_is_dimension_scoped() {
        assert_eq!(
            sqlite_current_vec_table_name(1024),
            "semantic_embedding_current_vec_dim_1024"
        );
    }

    #[test]
    fn postgres_index_names_are_dimension_scoped() {
        assert_eq!(
            postgres_current_pgvector_index_name(768),
            "symbol_embeddings_current_embedding_cosine_d768_idx"
        );
        assert_eq!(
            postgres_historical_pgvector_index_name(768),
            "symbol_embeddings_embedding_cosine_d768_idx"
        );
    }

    #[test]
    fn postgres_partial_index_sql_uses_expression_cast() {
        let sql = build_postgres_pgvector_partial_index_sql("symbol_embeddings_current", 768);
        assert!(sql.contains("USING hnsw"));
        assert!(sql.contains("embedding::vector(768)"));
        assert!(sql.contains("WHERE dimension = 768"));
    }

    #[test]
    fn postgres_nearest_sql_filters_repo_representation_setup_and_dimension() {
        let sql = build_postgres_nearest_current_candidates_sql(SemanticVectorQuery {
            repo_id: "repo-1",
            representation_kind: embeddings::EmbeddingRepresentationKind::Code,
            setup_fingerprint: "provider=voyage|model=voyage-code-3|dimension=3",
            dimension: 3,
            query_embedding: &[0.1, 0.2, 0.3],
            limit: 25,
        })
        .expect("build postgres nearest sql");
        assert!(sql.contains("FROM symbol_embeddings_current"));
        assert!(sql.contains("repo_id = 'repo-1'"));
        assert!(sql.contains("representation_kind IN ('code', 'baseline', 'enriched')"));
        assert!(
            sql.contains("setup_fingerprint = 'provider=voyage|model=voyage-code-3|dimension=3'")
        );
        assert!(sql.contains("dimension = 3"));
        assert!(sql.contains("::vector"));
        assert!(sql.contains("LIMIT 25"));
    }
}
