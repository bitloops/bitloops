#[test]
fn build_postgres_semantic_neighbors_sql_uses_embedding_similarity() {
    let sql = build_postgres_semantic_neighbors_sql("repo-1", "artefact-1", 8);
    assert!(sql.contains("FROM symbol_embeddings src"));
    assert!(sql.contains("JOIN symbol_embeddings emb"));
    assert!(sql.contains("semantic_score"));
    assert!(sql.contains("LIMIT 8"));
}
