use super::*;

pub(crate) fn commit_file(repo_root: &Path, filename: &str, content: &str) {
    fs::write(repo_root.join(filename), content).unwrap();
    git_ok(repo_root, &["add", filename]);
    git_ok(repo_root, &["commit", "-m", "test commit"]);
}

pub(crate) fn init_devql_schema(repo_root: &Path) -> PathBuf {
    init_devql_schema_with_postgres_dsn(repo_root, None)
}

pub(crate) fn init_devql_schema_with_postgres_dsn(
    repo_root: &Path,
    postgres_dsn: Option<&str>,
) -> PathBuf {
    let bitloops_dir = repo_root.join(".bitloops");
    fs::create_dir_all(&bitloops_dir).expect("create .bitloops directory");
    write_post_commit_test_config(repo_root, None);

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

    if postgres_dsn.is_some() {
        write_post_commit_test_config(repo_root, postgres_dsn);
    }

    sqlite_path
}

fn write_post_commit_test_config(repo_root: &Path, postgres_dsn: Option<&str>) {
    let sqlite_path = repo_root.join(".bitloops/stores/relational/post-commit-devql.db");
    let duckdb_path = repo_root.join(".bitloops/stores/events/post-commit-events.duckdb");
    let blob_local_path = repo_root.join(".bitloops/stores/blobs/post-commit");
    let cfg = serde_json::json!({
        "version": "1.0",
        "scope": "project",
        "settings": {
            "stores": {
                "relational": {
                    "sqlite_path": sqlite_path,
                    "postgres_dsn": postgres_dsn
                },
                "event": {
                    "duckdb_path": duckdb_path,
                    "clickhouse_url": null,
                    "clickhouse_user": null,
                    "clickhouse_password": null,
                    "clickhouse_database": null
                },
                "blob": {
                    "local_path": blob_local_path
                }
            }
        }
    });

    fs::write(
        repo_root.join(".bitloops/config.json"),
        serde_json::to_vec_pretty(&cfg).expect("serialise post-commit test config"),
    )
    .expect("write repo-local store config for post-commit tests");
}
