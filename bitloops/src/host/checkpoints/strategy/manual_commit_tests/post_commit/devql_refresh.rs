use super::*;

use super::helpers::commit_file;

fn init_devql_schema(repo_root: &Path) -> PathBuf {
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

#[test]
pub(crate) fn post_commit_refreshes_devql_current_state_for_changed_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let devql_sqlite_path = init_devql_schema(dir.path());

    fs::create_dir_all(dir.path().join("src")).unwrap();
    commit_file(
        dir.path(),
        "src/post_commit.ts",
        "export function run(value: number) { return value + 1; }\n",
    );
    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let sqlite = rusqlite::Connection::open(devql_sqlite_path).unwrap();
    let branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    let indexed_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND commit_sha = ?2 AND branch = ?3 AND revision_kind = 'commit'",
            rusqlite::params!["src/post_commit.ts", head_sha.as_str(), branch.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        indexed_rows > 0,
        "post_commit should index changed files into artefacts_current"
    );

    let commit_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commits WHERE commit_sha = ?1",
            rusqlite::params![head_sha.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        commit_rows, 1,
        "post_commit should upsert commit metadata in DevQL commits table"
    );
}

#[test]
pub(crate) fn post_commit_refresh_removes_devql_current_state_for_deleted_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let devql_sqlite_path = init_devql_schema(dir.path());

    fs::create_dir_all(dir.path().join("src")).unwrap();
    commit_file(
        dir.path(),
        "src/remove_me.ts",
        "export const removeMe = () => 'remove';\n",
    );
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let sqlite_path = devql_sqlite_path;
    let sqlite = rusqlite::Connection::open(&sqlite_path).unwrap();
    let branch = run_git(dir.path(), &["branch", "--show-current"]).unwrap();
    let before_delete: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND branch = ?2",
            rusqlite::params!["src/remove_me.ts", branch.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(before_delete > 0, "expected indexed file before deletion");
    drop(sqlite);

    fs::remove_file(dir.path().join("src/remove_me.ts")).unwrap();
    git_ok(dir.path(), &["add", "-A"]);
    git_ok(dir.path(), &["commit", "-m", "delete file"]);

    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let sqlite = rusqlite::Connection::open(&sqlite_path).unwrap();
    let after_delete: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = ?1 AND branch = ?2",
            rusqlite::params!["src/remove_me.ts", branch.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        after_delete, 0,
        "post_commit should remove deleted file rows from branch-scoped current state"
    );
}
