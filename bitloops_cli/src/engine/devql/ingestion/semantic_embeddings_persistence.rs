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
    generated_at DATETIME DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_artefact_idx
ON symbol_embeddings (repo_id, artefact_id);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_model_idx
ON symbol_embeddings (repo_id, provider, model, dimension, blob_sha);
"#
}

async fn init_postgres_semantic_embeddings_schema(
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    postgres_exec(pg_client, semantic_embeddings_postgres_schema_sql())
        .await
        .context("creating Postgres semantic embedding tables")?;
    Ok(())
}

async fn upsert_symbol_embedding_rows(
    relational: &RelationalStorage,
    inputs: &[semantic::SemanticFeatureInput],
    embedding_provider: Arc<dyn EmbeddingProvider>,
) -> Result<semantic_embeddings::SymbolEmbeddingIngestionStats> {
    let mut stats = semantic_embeddings::SymbolEmbeddingIngestionStats::default();
    if inputs.is_empty() {
        return Ok(stats);
    }

    let artefact_ids = inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let summary_by_artefact_id = load_semantic_summary_map(relational, &artefact_ids).await?;
    let embedding_inputs =
        semantic_embeddings::build_symbol_embedding_inputs(inputs, &summary_by_artefact_id);

    for input in embedding_inputs {
        let next_input_hash = semantic_embeddings::build_symbol_embedding_input_hash(
            &input,
            embedding_provider.as_ref(),
        );
        let state = load_symbol_embedding_index_state(relational, &input.artefact_id).await?;
        if !semantic_embeddings::symbol_embeddings_require_reindex(&state, &next_input_hash) {
            stats.skipped += 1;
            continue;
        }

        let input = input.clone();
        let embedding_provider = Arc::clone(&embedding_provider);
        let row = tokio::task::spawn_blocking(move || {
            semantic_embeddings::build_symbol_embedding_row(&input, embedding_provider.as_ref())
        })
        .await
        .context("building semantic embedding row on blocking worker")??;
        persist_symbol_embedding_row(relational, &row).await?;
        stats.upserted += 1;
    }

    Ok(stats)
}

async fn load_symbol_embedding_index_state(
    relational: &RelationalStorage,
    artefact_id: &str,
) -> Result<semantic_embeddings::SymbolEmbeddingIndexState> {
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
    row: &semantic_embeddings::SymbolEmbeddingRow,
) -> Result<()> {
    relational.exec(&build_symbol_embedding_persist_sql(row)?).await
}

async fn load_symbol_embedding_source_metadata(
    relational: &RelationalStorage,
    repo_id: &str,
    artefact_id: &str,
) -> Result<Option<Value>> {
    let rows = relational
        .query_rows(&build_symbol_embedding_source_metadata_sql(repo_id, artefact_id))
        .await?;
    Ok(rows.into_iter().next())
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
) -> semantic_embeddings::SymbolEmbeddingIndexState {
    let Some(row) = rows.first() else {
        return semantic_embeddings::SymbolEmbeddingIndexState::default();
    };

    semantic_embeddings::SymbolEmbeddingIndexState {
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

fn build_symbol_embedding_persist_sql(
    row: &semantic_embeddings::SymbolEmbeddingRow,
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

fn build_symbol_embedding_source_metadata_sql(repo_id: &str, artefact_id: &str) -> String {
    format!(
        "SELECT artefact_id, provider, model, dimension \
FROM symbol_embeddings \
WHERE repo_id = '{repo_id}' AND artefact_id = '{artefact_id}' \
LIMIT 1",
        repo_id = esc_pg(repo_id),
        artefact_id = esc_pg(artefact_id),
    )
}

fn build_postgres_semantic_neighbors_sql(
    repo_id: &str,
    source_artefact_id: &str,
    limit: usize,
) -> String {
    format!(
        "SELECT \
target.artefact_id, \
target.path, \
target.canonical_kind, \
target.language_kind, \
target.language, \
target.start_line, \
target.end_line, \
target.start_byte, \
target.end_byte, \
target.signature, \
target.modifiers, \
target.docstring, \
target.blob_sha, \
target.symbol_fqn, \
target.content_hash, \
target.updated_at AS created_at, \
sem.summary, \
emb.provider AS embedding_provider, \
emb.model AS embedding_model, \
emb.dimension AS embedding_dimension, \
1 - (emb.embedding <=> src.embedding) AS semantic_score \
FROM symbol_embeddings src \
JOIN symbol_embeddings emb \
  ON emb.repo_id = src.repo_id \
 AND emb.provider = src.provider \
 AND emb.model = src.model \
 AND emb.dimension = src.dimension \
 AND emb.artefact_id <> src.artefact_id \
JOIN artefacts_current target \
  ON target.repo_id = emb.repo_id \
 AND target.artefact_id = emb.artefact_id \
LEFT JOIN symbol_semantics sem \
  ON sem.artefact_id = target.artefact_id \
WHERE src.repo_id = '{repo_id}' \
  AND src.artefact_id = '{source_artefact_id}' \
ORDER BY emb.embedding <=> src.embedding, target.path, target.start_line \
LIMIT {limit}",
        repo_id = esc_pg(repo_id),
        source_artefact_id = esc_pg(source_artefact_id),
        limit = limit.max(1),
    )
}

fn sql_vector_string(values: &[f32]) -> Result<String> {
    if values.is_empty() {
        bail!("cannot persist empty embedding vector");
    }

    for value in values {
        if !value.is_finite() {
            bail!("cannot persist embedding vector containing non-finite values");
        }
    }

    Ok(format!(
        "'{}'::vector",
        esc_pg(&serde_json::to_string(values)?)
    ))
}

#[cfg(test)]
mod semantic_embedding_persistence_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn semantic_embedding_schema_includes_vector_table() {
        let schema = semantic_embeddings_postgres_schema_sql();
        assert!(schema.contains("CREATE EXTENSION IF NOT EXISTS vector"));
        assert!(schema.contains("CREATE TABLE IF NOT EXISTS symbol_embeddings"));
        assert!(schema.contains("embedding vector"));
    }

    #[test]
    fn semantic_embedding_state_parser_defaults_and_reads_hash() {
        let empty = parse_symbol_embedding_index_state_rows(&[]);
        assert_eq!(
            empty,
            semantic_embeddings::SymbolEmbeddingIndexState::default()
        );

        let rows = vec![json!({ "embedding_hash": "hash-1" })];
        let parsed = parse_symbol_embedding_index_state_rows(&rows);
        assert_eq!(parsed.embedding_hash.as_deref(), Some("hash-1"));
    }

    #[test]
    fn semantic_embedding_persist_sql_contains_vector_literal() {
        let sql = build_symbol_embedding_persist_sql(&semantic_embeddings::SymbolEmbeddingRow {
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
    fn semantic_neighbors_sql_orders_by_cosine_distance() {
        let sql = build_postgres_semantic_neighbors_sql("repo-1", "artefact-1", 10);
        assert!(sql.contains("1 - (emb.embedding <=> src.embedding) AS semantic_score"));
        assert!(sql.contains("ORDER BY emb.embedding <=> src.embedding"));
        assert!(sql.contains("LIMIT 10"));
    }
}
