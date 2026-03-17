#[test]
fn build_postgres_semantic_neighbors_sql_uses_embedding_similarity() {
    let sql = build_postgres_semantic_neighbors_sql("repo-1", "artefact-1", 8);
    assert!(sql.contains("FROM symbol_embeddings src"));
    assert!(sql.contains("JOIN symbol_embeddings emb"));
    assert!(sql.contains("semantic_score"));
    assert!(sql.contains("LIMIT 8"));
}

#[tokio::test]
async fn init_sqlite_schema_creates_symbol_embeddings_table() {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("devql.sqlite");

    init_sqlite_schema(&db_path)
        .await
        .expect("initialise sqlite relational schema");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite db");
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'symbol_embeddings'",
        )
        .expect("prepare sqlite master query");
    let table_name: String = stmt
        .query_row([], |row| row.get(0))
        .expect("symbol_embeddings table");

    assert_eq!(table_name, "symbol_embeddings");
}
