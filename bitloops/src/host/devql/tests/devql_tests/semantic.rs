use super::*;

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

#[tokio::test]
async fn init_relational_schema_creates_test_harness_tables() {
    let temp = tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(repo_root.join(".bitloops")).expect("create config dir");
    let db_path = repo_root.join("devql.sqlite");
    std::fs::write(
        repo_root.join(".bitloops/config.json"),
        format!(
            r#"{{
  "version": "1.0",
  "scope": "project",
  "settings": {{
    "stores": {{
      "relational": {{
        "provider": "sqlite",
        "sqlite_path": "{}"
      }}
    }}
  }}
}}"#,
            db_path.display()
        ),
    )
    .expect("write store config");

    let mut cfg = test_cfg();
    cfg.repo_root = repo_root;
    let relational = RelationalStorage::Sqlite {
        path: db_path.clone(),
    };
    init_relational_schema(&cfg, &relational)
        .await
        .expect("initialise sqlite relational schema");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite db");
    for table in [
        "test_suites",
        "test_scenarios",
        "test_links",
        "coverage_captures",
        "coverage_hits",
        "test_discovery_runs",
    ] {
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("query sqlite master");
        assert_eq!(table_count, 1, "expected sqlite table `{table}`");
    }
}
