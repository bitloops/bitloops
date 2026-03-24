use super::*;

pub(crate) fn commit_file(repo_root: &Path, filename: &str, content: &str) {
    fs::write(repo_root.join(filename), content).unwrap();
    git_ok(repo_root, &["add", filename]);
    git_ok(repo_root, &["commit", "-m", "test commit"]);
}

pub(crate) fn init_devql_schema(repo_root: &Path) -> PathBuf {
    let bitloops_dir = repo_root.join(".bitloops");
    fs::create_dir_all(&bitloops_dir).expect("create .bitloops directory");
    fs::write(
        bitloops_dir.join("config.json"),
        r#"{
  "version": "1.0",
  "scope": "project",
  "settings": {
    "stores": {
      "relational": {
        "sqlite_path": ".bitloops/stores/relational/post-commit-devql.db",
        "postgres_dsn": null
      },
      "event": {
        "duckdb_path": ".bitloops/stores/events/post-commit-events.duckdb",
        "clickhouse_url": null,
        "clickhouse_user": null,
        "clickhouse_password": null,
        "clickhouse_database": null
      },
      "blob": {
        "local_path": ".bitloops/stores/blobs/post-commit"
      }
    }
  }
}"#,
    )
    .expect("write repo-local store config for post-commit tests");

    let repo = crate::host::devql::resolve_repo_identity(repo_root).expect("resolve repo identity");
    let cfg = crate::host::devql::DevqlConfig::from_env(repo_root.to_path_buf(), repo)
        .expect("build devql cfg for post-commit test");
    let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime for devql init");
    runtime
        .block_on(crate::host::devql::run_init(&cfg))
        .expect("initialise DevQL schema for post-commit test");

    let sqlite_path = repo_root.join(".bitloops/stores/relational/post-commit-devql.db");
    let sqlite = rusqlite::Connection::open(&sqlite_path)
        .expect("open relational sqlite after DevQL init for post-commit test");
    let has_artefacts_current: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'artefacts_current'",
            [],
            |row| row.get(0),
        )
        .expect("query sqlite_master for artefacts_current table");
    assert_eq!(
        has_artefacts_current, 1,
        "post-commit test must initialise DevQL relational schema in the configured sqlite path"
    );

    sqlite_path
}
