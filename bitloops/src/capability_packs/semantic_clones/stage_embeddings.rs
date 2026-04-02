//! Stage 2: symbol embedding rows (`symbol_embeddings`) for the semantic_clones pipeline.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::adapters::model_providers::embeddings::EmbeddingProvider;
use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::{
    RelationalStorage, esc_pg, postgres_exec, sql_string_list_pg, sqlite_exec_path_allow_create,
};

fn semantic_embeddings_postgres_schema_sql() -> &'static str {
    r#"
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS symbol_embeddings (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding vector NOT NULL,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_artefact_idx
ON symbol_embeddings (repo_id, artefact_id);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_model_idx
ON symbol_embeddings (repo_id, provider, model, dimension, blob_sha);
"#
}

fn semantic_embeddings_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_embeddings (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding TEXT NOT NULL,
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_artefact_idx
ON symbol_embeddings (repo_id, artefact_id);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_model_idx
ON symbol_embeddings (repo_id, provider, model, dimension, blob_sha);
"#
}

pub(crate) async fn init_sqlite_semantic_embeddings_schema(sqlite_path: &Path) -> Result<()> {
    sqlite_exec_path_allow_create(sqlite_path, semantic_embeddings_sqlite_schema_sql())
        .await
        .context("creating SQLite semantic embedding tables")?;
    Ok(())
}

pub(crate) async fn init_postgres_semantic_embeddings_schema(
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    postgres_exec(pg_client, semantic_embeddings_postgres_schema_sql())
        .await
        .context("creating Postgres semantic embedding tables")?;
    Ok(())
}

pub(crate) async fn upsert_symbol_embedding_rows(
    relational: &RelationalStorage,
    inputs: &[semantic::SemanticFeatureInput],
    embedding_provider: Arc<dyn EmbeddingProvider>,
) -> Result<embeddings::SymbolEmbeddingIngestionStats> {
    let mut stats = embeddings::SymbolEmbeddingIngestionStats::default();
    if inputs.is_empty() {
        return Ok(stats);
    }

    ensure_semantic_embeddings_schema(relational).await?;

    let artefact_ids = inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let summary_by_artefact_id = load_semantic_summary_map(relational, &artefact_ids).await?;
    let embedding_inputs =
        embeddings::build_symbol_embedding_inputs(inputs, &summary_by_artefact_id);

    for input in embedding_inputs {
        let next_input_hash =
            embeddings::build_symbol_embedding_input_hash(&input, embedding_provider.as_ref());
        let state = load_symbol_embedding_index_state(relational, &input.artefact_id).await?;
        if !embeddings::symbol_embeddings_require_reindex(&state, &next_input_hash) {
            stats.skipped += 1;
            continue;
        }

        let input = input.clone();
        let embedding_provider = Arc::clone(&embedding_provider);
        let row = tokio::task::spawn_blocking(move || {
            embeddings::build_symbol_embedding_row(&input, embedding_provider.as_ref())
        })
        .await
        .context("building semantic embedding row on blocking worker")??;
        persist_symbol_embedding_row(relational, &row).await?;
        stats.upserted += 1;
    }

    Ok(stats)
}

pub(crate) async fn ensure_semantic_embeddings_schema(
    relational: &RelationalStorage,
) -> Result<()> {
    init_sqlite_semantic_embeddings_schema(&relational.local.path).await?;
    if let Some(remote) = relational.remote.as_ref() {
        init_postgres_semantic_embeddings_schema(&remote.client).await?;
    }
    Ok(())
}

pub(crate) async fn clear_repo_symbol_embedding_rows(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let sql = format!(
        "DELETE FROM symbol_embeddings WHERE repo_id = '{}'",
        esc_pg(repo_id),
    );
    relational.exec(&sql).await
}

async fn load_symbol_embedding_index_state(
    relational: &RelationalStorage,
    artefact_id: &str,
) -> Result<embeddings::SymbolEmbeddingIndexState> {
    let rows = relational
        .query_rows(&build_symbol_embedding_index_state_sql(artefact_id))
        .await?;
    Ok(parse_symbol_embedding_index_state_rows(&rows))
}

async fn load_semantic_summary_map(
    relational: &RelationalStorage,
    artefact_ids: &[String],
) -> Result<HashMap<String, String>> {
    if artefact_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = relational
        .query_rows(&build_semantic_summary_lookup_sql(artefact_ids))
        .await?;
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(artefact_id) = row.get("artefact_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(summary) = row.get("summary").and_then(Value::as_str) else {
            continue;
        };
        if !summary.trim().is_empty() {
            out.insert(artefact_id.to_string(), summary.to_string());
        }
    }
    Ok(out)
}

async fn persist_symbol_embedding_row(
    relational: &RelationalStorage,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<()> {
    let sql = build_sqlite_symbol_embedding_persist_sql(row)?;
    relational.exec(&sql).await
}

fn build_symbol_embedding_index_state_sql(artefact_id: &str) -> String {
    format!(
        "SELECT embedding_input_hash AS embedding_hash \
FROM symbol_embeddings \
WHERE artefact_id = '{artefact_id}'",
        artefact_id = esc_pg(artefact_id),
    )
}

fn parse_symbol_embedding_index_state_rows(
    rows: &[Value],
) -> embeddings::SymbolEmbeddingIndexState {
    let Some(row) = rows.first() else {
        return embeddings::SymbolEmbeddingIndexState::default();
    };

    embeddings::SymbolEmbeddingIndexState {
        embedding_hash: row
            .get("embedding_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn build_semantic_summary_lookup_sql(artefact_ids: &[String]) -> String {
    format!(
        "SELECT artefact_id, summary \
FROM symbol_semantics \
WHERE artefact_id IN ({})",
        sql_string_list_pg(artefact_ids)
    )
}

#[cfg(test)]
fn build_postgres_symbol_embedding_persist_sql(
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<String> {
    let embedding_expr = sql_vector_string(&row.embedding)?;
    Ok(format!(
        "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, provider, model, dimension, embedding_input_hash, embedding) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{provider}', '{model}', {dimension}, '{embedding_input_hash}', {embedding}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, provider = EXCLUDED.provider, model = EXCLUDED.model, dimension = EXCLUDED.dimension, embedding_input_hash = EXCLUDED.embedding_input_hash, embedding = EXCLUDED.embedding, generated_at = now()",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        blob_sha = esc_pg(&row.blob_sha),
        provider = esc_pg(&row.provider),
        model = esc_pg(&row.model),
        dimension = row.dimension,
        embedding_input_hash = esc_pg(&row.embedding_input_hash),
        embedding = embedding_expr,
    ))
}

fn build_sqlite_symbol_embedding_persist_sql(
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<String> {
    let embedding_json = sql_json_string(&row.embedding)?;
    Ok(format!(
        "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, provider, model, dimension, embedding_input_hash, embedding) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{provider}', '{model}', {dimension}, '{embedding_input_hash}', '{embedding}') \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = excluded.repo_id, blob_sha = excluded.blob_sha, provider = excluded.provider, model = excluded.model, dimension = excluded.dimension, embedding_input_hash = excluded.embedding_input_hash, embedding = excluded.embedding, generated_at = CURRENT_TIMESTAMP",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        blob_sha = esc_pg(&row.blob_sha),
        provider = esc_pg(&row.provider),
        model = esc_pg(&row.model),
        dimension = row.dimension,
        embedding_input_hash = esc_pg(&row.embedding_input_hash),
        embedding = embedding_json,
    ))
}

#[cfg(test)]
fn sql_vector_string(values: &[f32]) -> Result<String> {
    let json = sql_json_string(values)?;
    Ok(format!("'{json}'::vector"))
}

fn sql_json_string(values: &[f32]) -> Result<String> {
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

#[cfg(test)]
mod semantic_embedding_persistence_tests {
    use super::*;
    use crate::host::devql::sqlite_query_rows_path;
    use serde_json::json;
    use tempfile::tempdir;

    async fn sqlite_relational_with_schema(sql: &str) -> RelationalStorage {
        let temp = tempdir().expect("temp dir");
        let db_path = temp.path().join("semantic-embeddings.sqlite");
        sqlite_exec_path_allow_create(&db_path, sql)
            .await
            .expect("create sqlite schema");
        std::mem::forget(temp);
        RelationalStorage::local_only(db_path)
    }

    #[test]
    fn semantic_embedding_schema_includes_vector_table() {
        let schema = semantic_embeddings_postgres_schema_sql();
        assert!(schema.contains("CREATE EXTENSION IF NOT EXISTS vector"));
        assert!(schema.contains("CREATE TABLE IF NOT EXISTS symbol_embeddings"));
        assert!(schema.contains("embedding vector"));
    }

    #[test]
    fn semantic_embedding_sqlite_schema_uses_text_storage() {
        let schema = semantic_embeddings_sqlite_schema_sql();
        assert!(schema.contains("CREATE TABLE IF NOT EXISTS symbol_embeddings"));
        assert!(schema.contains("embedding TEXT NOT NULL"));
        assert!(schema.contains("generated_at DATETIME DEFAULT CURRENT_TIMESTAMP"));
    }

    #[test]
    fn semantic_embedding_state_parser_defaults_and_reads_hash() {
        let empty = parse_symbol_embedding_index_state_rows(&[]);
        assert_eq!(empty, embeddings::SymbolEmbeddingIndexState::default());

        let rows = vec![json!({ "embedding_hash": "hash-1" })];
        let parsed = parse_symbol_embedding_index_state_rows(&rows);
        assert_eq!(parsed.embedding_hash.as_deref(), Some("hash-1"));
    }

    #[test]
    fn semantic_embedding_postgres_persist_sql_contains_vector_literal() {
        let sql = build_postgres_symbol_embedding_persist_sql(&embeddings::SymbolEmbeddingRow {
            artefact_id: "artefact-1".to_string(),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            provider: "voyage".to_string(),
            model: "voyage-code-3".to_string(),
            dimension: 3,
            embedding_input_hash: "hash-1".to_string(),
            embedding: vec![0.1, -0.2, 0.3],
        })
        .expect("persist sql");
        assert!(sql.contains("INSERT INTO symbol_embeddings"));
        assert!(sql.contains("'[0.1,-0.2,0.3]'::vector"));
    }

    #[test]
    fn semantic_embedding_sqlite_persist_sql_contains_json_literal() {
        let sql = build_sqlite_symbol_embedding_persist_sql(&embeddings::SymbolEmbeddingRow {
            artefact_id: "artefact-1".to_string(),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            provider: "local".to_string(),
            model: "jinaai/jina-embeddings-v2-base-code".to_string(),
            dimension: 3,
            embedding_input_hash: "hash-1".to_string(),
            embedding: vec![0.1, -0.2, 0.3],
        })
        .expect("persist sql");
        assert!(sql.contains("INSERT INTO symbol_embeddings"));
        assert!(sql.contains("'[0.1,-0.2,0.3]'"));
        assert!(!sql.contains("::vector"));
        assert!(sql.contains("generated_at = CURRENT_TIMESTAMP"));
    }

    #[test]
    fn semantic_embedding_vector_sql_contains_vector_cast() {
        let sql = sql_vector_string(&[0.1, -0.2, 0.3]).expect("vector sql");
        assert_eq!(sql, "'[0.1,-0.2,0.3]'::vector");
    }

    #[test]
    fn semantic_embedding_json_sql_contains_json_literal() {
        let sql = sql_json_string(&[0.1, -0.2, 0.3]).expect("json sql");
        assert_eq!(sql, "[0.1,-0.2,0.3]");
    }

    #[test]
    fn semantic_embedding_vector_sql_rejects_empty_or_non_finite_vectors() {
        let empty_err = sql_vector_string(&[]).expect_err("empty vectors must fail");
        assert!(empty_err.to_string().contains("empty embedding vector"));

        let invalid_err =
            sql_vector_string(&[0.1, f32::NAN]).expect_err("non-finite vectors must fail");
        assert!(invalid_err.to_string().contains("non-finite values"));
    }

    #[test]
    fn semantic_embedding_json_sql_rejects_empty_or_non_finite_vectors() {
        let empty_err = sql_json_string(&[]).expect_err("empty vectors must fail");
        assert!(empty_err.to_string().contains("empty embedding vector"));

        let invalid_err =
            sql_json_string(&[0.1, f32::NAN]).expect_err("non-finite vectors must fail");
        assert!(invalid_err.to_string().contains("non-finite values"));
    }

    #[test]
    fn semantic_embedding_index_state_sql_filters_by_artefact_id() {
        let sql = build_symbol_embedding_index_state_sql("artefact-'1");
        assert!(sql.contains("FROM symbol_embeddings"));
        assert!(sql.contains("WHERE artefact_id = 'artefact-''1'"));
    }

    #[test]
    fn semantic_embedding_summary_lookup_sql_uses_all_ids() {
        let sql = build_semantic_summary_lookup_sql(&[
            "artefact-1".to_string(),
            "artefact-2".to_string(),
        ]);
        assert!(sql.contains("FROM symbol_semantics"));
        assert!(sql.contains("'artefact-1'"));
        assert!(sql.contains("'artefact-2'"));
    }

    #[tokio::test]
    async fn semantic_embedding_loads_index_state_from_relational_storage() {
        let relational = sqlite_relational_with_schema(
            "CREATE TABLE symbol_embeddings (
                artefact_id TEXT PRIMARY KEY,
                repo_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                dimension INTEGER NOT NULL,
                embedding_input_hash TEXT NOT NULL
            );
            INSERT INTO symbol_embeddings (
                artefact_id, repo_id, provider, model, dimension, embedding_input_hash
            ) VALUES (
                'artefact-1', 'repo-1', 'voyage', 'voyage-code-3', 1024, 'hash-1'
            );",
        )
        .await;

        let state = load_symbol_embedding_index_state(&relational, "artefact-1")
            .await
            .expect("load embedding state");

        assert_eq!(state.embedding_hash.as_deref(), Some("hash-1"));
    }

    #[tokio::test]
    async fn semantic_embedding_loads_summary_map_from_relational_storage() {
        let relational = sqlite_relational_with_schema(
            "CREATE TABLE symbol_semantics (
                artefact_id TEXT PRIMARY KEY,
                summary TEXT
            );
            INSERT INTO symbol_semantics (artefact_id, summary) VALUES
                ('artefact-1', 'summarizes function 1'),
                ('artefact-2', ''),
                ('artefact-3', 'summarizes function 3');",
        )
        .await;

        let summary_map = load_semantic_summary_map(
            &relational,
            &[
                "artefact-1".to_string(),
                "artefact-2".to_string(),
                "artefact-3".to_string(),
            ],
        )
        .await
        .expect("load summary map");

        assert_eq!(
            summary_map.get("artefact-1").map(String::as_str),
            Some("summarizes function 1")
        );
        assert_eq!(
            summary_map.get("artefact-3").map(String::as_str),
            Some("summarizes function 3")
        );
        assert!(!summary_map.contains_key("artefact-2"));
    }

    #[tokio::test]
    async fn semantic_embedding_schema_ensure_creates_sqlite_table() {
        let temp = tempdir().expect("temp dir");
        let db_path = temp.path().join("semantic-embeddings.sqlite");
        let relational = RelationalStorage::local_only(db_path.clone());

        ensure_semantic_embeddings_schema(&relational)
            .await
            .expect("ensure sqlite embedding schema");

        let rows = sqlite_query_rows_path(
            &db_path,
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'symbol_embeddings'",
        )
        .await
        .expect("query sqlite master");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("name"), Some(&json!("symbol_embeddings")));
    }
}
