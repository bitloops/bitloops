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
    let home = TempDir::new().expect("home dir");
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[
            ("HOME", Some(home_path.as_str())),
            ("USERPROFILE", Some(home_path.as_str())),
            ("BITLOOPS_DEVQL_PG_DSN", None),
            ("BITLOOPS_DEVQL_CH_URL", None),
            ("BITLOOPS_DEVQL_CH_USER", None),
            ("BITLOOPS_DEVQL_CH_PASSWORD", None),
            ("BITLOOPS_DEVQL_CH_DATABASE", None),
        ],
    );
    let repo_root = temp.path().join("repo");
    let db_path = repo_root.join("devql.sqlite");
    write_repo_daemon_config(
        &repo_root,
        format!(
            "[stores.relational]\nsqlite_path = {path:?}\n",
            path = db_path.display()
        ),
    );

    let mut cfg = test_cfg();
    cfg.daemon_config_root = repo_root.clone();
    cfg.repo_root = repo_root;
    let relational = RelationalStorage::local_only(db_path.clone());
    init_relational_schema(&cfg, &relational)
        .await
        .expect("initialise sqlite relational schema");

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite db");
    for table in [
        "test_artefacts_current",
        "test_artefact_edges_current",
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

    for table in ["test_suites", "test_scenarios", "test_links"] {
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("query sqlite master");
        assert_eq!(
            table_count, 0,
            "did not expect legacy sqlite table `{table}`"
        );
    }
}
