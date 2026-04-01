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
    sqlite
        .execute_batch(
            r#"
CREATE TABLE IF NOT EXISTS repositories (
    repo_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    organization TEXT NOT NULL,
    name TEXT NOT NULL,
    default_branch TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);
"#,
        )
        .expect("ensure DevQL repository catalog exists for post-commit test");
    sqlite
        .execute_batch(crate::host::devql::checkpoint_schema_sql_sqlite())
        .expect("ensure checkpoint projection tables exist for post-commit test");
    sqlite
        .execute_batch(crate::host::devql::sync::schema::sync_schema_sql())
        .expect("ensure DevQL sync tables exist for post-commit test");
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
    let has_repo_sync_state: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'repo_sync_state'",
            [],
            |row| row.get(0),
        )
        .expect("query sqlite_master for repo_sync_state table");
    assert_eq!(
        has_repo_sync_state, 1,
        "post-commit test must initialise DevQL sync schema in the configured sqlite path"
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
    let postgres_line = postgres_dsn
        .map(|dsn| format!("postgres_dsn = {dsn:?}\n"))
        .unwrap_or_default();
    fs::write(
        repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        format!(
            "[stores.relational]\nsqlite_path = {sqlite_path:?}\n{postgres_line}\n[stores.event]\nduckdb_path = {duckdb_path:?}\n\n[stores.blob]\nlocal_path = {blob_local_path:?}\n",
            sqlite_path = sqlite_path.to_string_lossy(),
            duckdb_path = duckdb_path.to_string_lossy(),
            blob_local_path = blob_local_path.to_string_lossy(),
        ),
    )
    .expect("write repo-local store config for post-commit tests");
}
