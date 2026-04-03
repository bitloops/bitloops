use rusqlite::Connection;
use serde_json::json;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

async fn sqlite_relational_store_with_sync_schema(
    path: &Path,
) -> crate::host::devql::RelationalStorage {
    crate::host::devql::init_sqlite_schema(path)
        .await
        .expect("initialise sqlite relational schema");
    crate::host::devql::RelationalStorage::local_only(path.to_path_buf())
}

async fn seed_sync_repository_catalog_row(
    relational: &crate::host::devql::RelationalStorage,
    cfg: &crate::host::devql::DevqlConfig,
) {
    relational
        .exec(&format!(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
             VALUES ('{}', '{}', '{}', '{}', 'main') \
             ON CONFLICT(repo_id) DO UPDATE SET \
               provider = excluded.provider, \
               organization = excluded.organization, \
               name = excluded.name, \
               default_branch = excluded.default_branch",
            crate::host::devql::db_utils::esc_pg(&cfg.repo.repo_id),
            crate::host::devql::db_utils::esc_pg(&cfg.repo.provider),
            crate::host::devql::db_utils::esc_pg(&cfg.repo.organization),
            crate::host::devql::db_utils::esc_pg(&cfg.repo.name),
        ))
        .await
        .expect("seed sync repository catalog row");
}

fn sync_test_cfg() -> crate::host::devql::DevqlConfig {
    crate::host::devql::DevqlConfig {
        config_root: std::path::PathBuf::from("/tmp/repo"),
        repo_root: std::path::PathBuf::from("/tmp/repo"),
        repo: crate::host::devql::RepoIdentity {
            provider: "github".to_string(),
            organization: "bitloops".to_string(),
            name: "temp2".to_string(),
            identity: "github/bitloops/temp2".to_string(),
            repo_id: crate::host::devql::deterministic_uuid("repo://github/bitloops/temp2"),
        },
        pg_dsn: None,
        clickhouse_url: "http://localhost:8123".to_string(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: "default".to_string(),
        semantic_provider: None,
        semantic_model: None,
        semantic_api_key: None,
        semantic_base_url: None,
    }
}

fn sync_test_cfg_for_repo(repo_root: &Path) -> crate::host::devql::DevqlConfig {
    crate::host::devql::DevqlConfig {
        config_root: repo_root.to_path_buf(),
        repo_root: repo_root.to_path_buf(),
        repo: crate::host::devql::RepoIdentity {
            provider: "github".to_string(),
            organization: "bitloops".to_string(),
            name: "sync-task-10".to_string(),
            identity: "github/bitloops/sync-task-10".to_string(),
            repo_id: crate::host::devql::deterministic_uuid(&format!(
                "repo://{}",
                repo_root.display()
            )),
        },
        pg_dsn: None,
        clickhouse_url: "http://localhost:8123".to_string(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: "default".to_string(),
        semantic_provider: None,
        semantic_model: None,
        semantic_api_key: None,
        semantic_base_url: None,
    }
}

fn desired_file_state(
    path: &str,
    language: &str,
    content_id: &str,
) -> crate::host::devql::sync::types::DesiredFileState {
    crate::host::devql::sync::types::DesiredFileState {
        path: path.to_string(),
        language: language.to_string(),
        head_content_id: Some(content_id.to_string()),
        index_content_id: Some(content_id.to_string()),
        worktree_content_id: Some(content_id.to_string()),
        effective_content_id: content_id.to_string(),
        effective_source: crate::host::devql::sync::types::EffectiveSource::Head,
        exists_in_head: true,
        exists_in_index: true,
        exists_in_worktree: true,
    }
}

fn expected_symbol_id_by_fqn(
    items: &[crate::host::language_adapter::LanguageArtefact],
    path: &str,
) -> std::collections::HashMap<String, String> {
    let mut symbol_ids = std::collections::HashMap::from([(
        path.to_string(),
        crate::host::devql::file_symbol_id(path),
    )]);

    for item in items {
        let parent_symbol_id = item
            .parent_symbol_fqn
            .as_ref()
            .and_then(|fqn| symbol_ids.get(fqn))
            .map(String::as_str);
        let symbol_id =
            crate::host::devql::structural_symbol_id_for_artefact(item, parent_symbol_id);
        symbol_ids.insert(item.symbol_fqn.clone(), symbol_id);
    }

    symbol_ids
}

#[test]
fn sync_lock_acquires_and_releases() {
    let dir = tempdir().expect("temp dir");
    let lock_path = dir.path().join(".bitloops").join("sync.lock");

    let lock =
        crate::host::devql::sync::lock::SyncLock::acquire(dir.path()).expect("acquire sync lock");
    assert!(lock.is_held());
    assert!(lock_path.exists(), "lock file should exist while held");

    drop(lock);

    assert!(
        !lock_path.exists(),
        "lock file should be removed when lock is dropped"
    );

    let lock2 = crate::host::devql::sync::lock::SyncLock::acquire(dir.path())
        .expect("re-acquire sync lock");
    assert!(lock2.is_held());
}

#[test]
fn sync_lock_fails_fast_when_held() {
    let dir = tempdir().expect("temp dir");
    let _lock =
        crate::host::devql::sync::lock::SyncLock::acquire(dir.path()).expect("acquire sync lock");

    let result = crate::host::devql::sync::lock::SyncLock::try_acquire(dir.path());

    assert!(result.is_err(), "second acquisition should fail fast");
}

#[test]
fn sync_lock_partial_file_is_treated_as_held() {
    let dir = tempdir().expect("temp dir");
    let lock_dir = dir.path().join(".bitloops");
    let lock_path = lock_dir.join("sync.lock");
    fs::create_dir_all(&lock_dir).expect("create lock dir");
    fs::write(&lock_path, format!("{}\n", std::process::id())).expect("write partial lock");

    let result = crate::host::devql::sync::lock::SyncLock::try_acquire(dir.path());

    assert!(
        result.is_err(),
        "partial lock file should be treated as held"
    );
    assert_eq!(
        fs::read_to_string(&lock_path).expect("read partial lock"),
        format!("{}\n", std::process::id()),
        "partial lock file should not be cleared as stale"
    );
}

#[test]
fn sync_lock_malformed_file_is_treated_as_held() {
    let dir = tempdir().expect("temp dir");
    let lock_dir = dir.path().join(".bitloops");
    let lock_path = lock_dir.join("sync.lock");
    fs::create_dir_all(&lock_dir).expect("create lock dir");
    fs::write(&lock_path, "not-a-pid\nmalformed-token\n").expect("write malformed lock");

    let result = crate::host::devql::sync::lock::SyncLock::try_acquire(dir.path());

    assert!(
        result.is_err(),
        "malformed lock file should be treated as held"
    );
    assert_eq!(
        fs::read_to_string(&lock_path).expect("read malformed lock"),
        "not-a-pid\nmalformed-token\n",
        "malformed lock file should not be cleared as stale"
    );
}

#[tokio::test]
async fn repo_sync_state_write_helpers_track_lifecycle() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let cfg = sync_test_cfg();
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    crate::host::devql::sync::lock::write_sync_started(
        &relational,
        &cfg.repo.repo_id,
        cfg.repo_root.to_string_lossy().as_ref(),
        "full",
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("write started state");

    let started_rows = relational
        .query_rows(&format!(
            "SELECT repo_root, active_branch, head_commit_sha, head_tree_sha, parser_version, extractor_version, last_sync_started_at, last_sync_completed_at, last_sync_status, last_sync_reason \
FROM repo_sync_state WHERE repo_id = '{}'",
            crate::host::devql::db_utils::esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("query started sync state");
    let started = started_rows
        .first()
        .and_then(serde_json::Value::as_object)
        .expect("started row");
    assert_eq!(
        started.get("repo_root").and_then(|v| v.as_str()),
        Some("/tmp/repo")
    );
    assert_eq!(started.get("active_branch").and_then(|v| v.as_str()), None);
    assert_eq!(
        started.get("head_commit_sha").and_then(|v| v.as_str()),
        None
    );
    assert_eq!(started.get("head_tree_sha").and_then(|v| v.as_str()), None);
    assert_eq!(
        started.get("parser_version").and_then(|v| v.as_str()),
        Some("parser-v1")
    );
    assert_eq!(
        started.get("extractor_version").and_then(|v| v.as_str()),
        Some("extractor-v1")
    );
    assert!(
        started
            .get("last_sync_started_at")
            .and_then(|v| v.as_str())
            .is_some(),
        "started timestamp should be written"
    );
    assert_eq!(
        started
            .get("last_sync_completed_at")
            .and_then(|v| v.as_str()),
        None
    );
    assert_eq!(
        started.get("last_sync_status").and_then(|v| v.as_str()),
        Some("running")
    );
    assert_eq!(
        started.get("last_sync_reason").and_then(|v| v.as_str()),
        Some("full")
    );

    crate::host::devql::sync::lock::write_sync_completed(
        &relational,
        &cfg.repo.repo_id,
        Some("head-123"),
        Some("tree-456"),
        Some("main"),
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("write completed state");

    let completed_rows = relational
        .query_rows(&format!(
            "SELECT repo_root, active_branch, head_commit_sha, head_tree_sha, parser_version, extractor_version, last_sync_started_at, last_sync_completed_at, last_sync_status, last_sync_reason \
FROM repo_sync_state WHERE repo_id = '{}'",
            crate::host::devql::db_utils::esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("query completed sync state");
    let completed = completed_rows
        .first()
        .and_then(serde_json::Value::as_object)
        .expect("completed row");
    assert_eq!(
        completed.get("repo_root").and_then(|v| v.as_str()),
        Some("/tmp/repo")
    );
    assert_eq!(
        completed.get("active_branch").and_then(|v| v.as_str()),
        Some("main")
    );
    assert_eq!(
        completed.get("head_commit_sha").and_then(|v| v.as_str()),
        Some("head-123")
    );
    assert_eq!(
        completed.get("head_tree_sha").and_then(|v| v.as_str()),
        Some("tree-456")
    );
    assert_eq!(
        completed.get("last_sync_status").and_then(|v| v.as_str()),
        Some("completed")
    );
    assert!(
        completed
            .get("last_sync_completed_at")
            .and_then(|v| v.as_str())
            .is_some(),
        "completed timestamp should be written"
    );
    assert_eq!(
        completed.get("last_sync_reason").and_then(|v| v.as_str()),
        Some("full")
    );
}

#[tokio::test]
async fn repo_sync_state_write_failed_marks_repo_as_failed() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let cfg = sync_test_cfg();
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    crate::host::devql::sync::lock::write_sync_started(
        &relational,
        &cfg.repo.repo_id,
        cfg.repo_root.to_string_lossy().as_ref(),
        "repair",
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("write started state");

    crate::host::devql::sync::lock::write_sync_failed(&relational, &cfg.repo.repo_id)
        .await
        .expect("write failed state");

    let rows = relational
        .query_rows(&format!(
            "SELECT last_sync_status, last_sync_reason, last_sync_started_at FROM repo_sync_state WHERE repo_id = '{}'",
            crate::host::devql::db_utils::esc_pg(&cfg.repo.repo_id),
        ))
        .await
        .expect("query failed sync state");
    let row = rows
        .first()
        .and_then(serde_json::Value::as_object)
        .expect("failed row");
    assert_eq!(
        row.get("last_sync_status").and_then(|v| v.as_str()),
        Some("failed")
    );
    assert_eq!(
        row.get("last_sync_reason").and_then(|v| v.as_str()),
        Some("repair")
    );
    assert!(
        row.get("last_sync_started_at")
            .and_then(|v| v.as_str())
            .is_some(),
        "failed write should preserve started timestamp"
    );
}

#[tokio::test]
async fn repo_sync_state_write_completed_errors_without_started_row() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let cfg = sync_test_cfg();
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let err = crate::host::devql::sync::lock::write_sync_completed(
        &relational,
        &cfg.repo.repo_id,
        Some("head-123"),
        Some("tree-456"),
        Some("main"),
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect_err("missing repo_sync_state row should error");

    assert!(
        err.to_string().contains("repo_sync_state"),
        "error should explain the missing sync state row"
    );
}

#[tokio::test]
async fn repo_sync_state_write_failed_errors_without_started_row() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let cfg = sync_test_cfg();
    seed_sync_repository_catalog_row(&relational, &cfg).await;

    let err = crate::host::devql::sync::lock::write_sync_failed(&relational, &cfg.repo.repo_id)
        .await
        .expect_err("missing repo_sync_state row should error");

    assert!(
        err.to_string().contains("repo_sync_state"),
        "error should explain the missing sync state row"
    );
}

#[tokio::test]
async fn sync_schema_creates_all_tables() {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("devql.sqlite");
    let db = Connection::open(&db_path).expect("open sqlite db");
    db.execute_batch(
        r#"
CREATE TABLE current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    updated_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, path)
);
"#,
    )
    .expect("seed legacy current_file_state");
    drop(db);

    crate::host::devql::init_sqlite_schema(&db_path)
        .await
        .expect("initialise sqlite relational schema");

    let db = Connection::open(&db_path).expect("open sqlite db");

    for table in &[
        "repo_sync_state",
        "current_file_state",
        "artefacts_current",
        "artefact_edges_current",
        "content_cache",
        "content_cache_artefacts",
        "content_cache_edges",
    ] {
        let count: i64 = db
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{table}'"
                ),
                [],
                |row| row.get(0),
            )
            .expect("read sqlite_master");
        assert_eq!(count, 1, "table {table} should exist");
    }

    let legacy_sync_state_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sync_state'",
            [],
            |row| row.get(0),
        )
        .expect("read sqlite_master for sync_state");
    assert_eq!(
        legacy_sync_state_count, 1,
        "legacy sync_state table should still exist"
    );

    for column in &[
        "head_content_id",
        "index_content_id",
        "worktree_content_id",
        "effective_content_id",
        "effective_source",
        "parser_version",
        "extractor_version",
        "exists_in_head",
        "exists_in_index",
        "exists_in_worktree",
        "last_synced_at",
    ] {
        let column_count: i64 = db
            .query_row(
                "SELECT COUNT(*) \
                 FROM pragma_table_info('current_file_state') \
                 WHERE name = ?1",
                [column],
                |row| row.get(0),
            )
            .expect("read pragma_table_info");
        assert_eq!(
            column_count, 1,
            "column {column} should exist on current_file_state"
        );
    }

    for table in &[
        "repo_sync_state",
        "current_file_state",
        "artefacts_current",
        "artefact_edges_current",
    ] {
        let mut stmt = db
            .prepare(&format!("PRAGMA foreign_key_list({table})"))
            .expect("prepare foreign_key_list");
        let fk_rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .expect("query foreign_key_list")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect foreign keys");
        assert!(
            fk_rows
                .iter()
                .any(|(referenced_table, from_column, to_column, on_delete)| {
                    referenced_table == "repositories"
                        && from_column == "repo_id"
                        && to_column == "repo_id"
                        && on_delete.eq_ignore_ascii_case("CASCADE")
                }),
            "table {table} should reference repositories(repo_id) with ON DELETE CASCADE"
        );
    }
}

#[test]
fn sync_artefacts_current_migration_sql_recreates_current_state_tables() {
    let sql = crate::host::devql::sync::schema::sync_artefacts_current_migration_sql();

    assert!(sql.contains("DROP TABLE IF EXISTS artefacts_current;"));
    assert!(sql.contains("DROP TABLE IF EXISTS artefact_edges_current;"));

    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefacts_current ("));
    assert!(sql.contains("content_id TEXT NOT NULL"));
    assert!(sql.contains("modifiers TEXT NOT NULL DEFAULT '[]'"));
    assert!(sql.contains("PRIMARY KEY (repo_id, path, symbol_id)"));
    assert!(sql.contains("UNIQUE (repo_id, artefact_id)"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_path_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_kind_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_fqn_idx"));
    assert!(!sql.contains("branch TEXT"));
    assert!(!sql.contains("commit_sha"));
    assert!(!sql.contains("revision_kind"));
    assert!(!sql.contains("revision_id"));
    assert!(!sql.contains("temp_checkpoint_id"));
    assert!(!sql.contains("blob_sha"));

    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefact_edges_current ("));
    assert!(sql.contains("metadata TEXT NOT NULL DEFAULT '{}'"));
    assert!(sql.contains("PRIMARY KEY (repo_id, edge_id)"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_current_path_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_current_from_idx"));
    assert!(!sql.contains("artefact_edges_current_branch_from_idx"));
    assert!(!sql.contains("artefact_edges_current_branch_to_idx"));
    assert!(!sql.contains("JSONB"));
}

fn seed_workspace_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n",
    )
    .expect("write rust source");
    fs::write(dir.path().join("README.md"), "# ignored\n").expect("write readme");

    crate::test_support::git_fixtures::git_ok(dir.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(dir.path(), &["commit", "-m", "initial"]);
    dir
}

fn seed_full_sync_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    fs::create_dir_all(dir.path().join("web")).expect("create web dir");
    fs::create_dir_all(dir.path().join("scripts")).expect("create scripts dir");

    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n",
    )
    .expect("write rust source");
    fs::write(
        dir.path().join("web/app.ts"),
        "import { helper } from \"./util\";\n\nexport function run(): number {\n  return helper();\n}\n",
    )
    .expect("write TypeScript source");
    fs::write(
        dir.path().join("web/util.js"),
        "export function helper() {\n  return 7;\n}\n",
    )
    .expect("write JavaScript source");
    fs::write(
        dir.path().join("scripts/main.py"),
        "def helper() -> int:\n    return 1\n\n\ndef run() -> int:\n    return helper()\n",
    )
    .expect("write Python source");
    fs::write(dir.path().join("README.md"), "# ignored\n").expect("write readme");

    crate::test_support::git_fixtures::git_ok(dir.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(dir.path(), &["commit", "-m", "initial"]);
    dir
}

fn seed_supported_and_unsupported_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    fs::create_dir_all(dir.path().join("docs")).expect("create docs dir");

    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n",
    )
    .expect("write supported source file");
    fs::write(dir.path().join("docs/notes.foo"), "ignored content\n")
        .expect("write unsupported source file");

    crate::test_support::git_fixtures::git_ok(dir.path(), &["add", "."]);
    crate::test_support::git_fixtures::git_ok(dir.path(), &["commit", "-m", "initial"]);
    dir
}

#[test]
fn workspace_state_inspect_workspace_reads_head_tree() {
    let repo = seed_workspace_repo();

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect clean workspace");

    let head_sha = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", "HEAD"],
    )
    .expect("resolve HEAD");
    let head_blob = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", "HEAD:src/lib.rs"],
    )
    .expect("resolve HEAD blob");
    let head_tree_sha = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", "HEAD^{tree}"],
    )
    .expect("resolve HEAD tree");
    let active_branch = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["branch", "--show-current"],
    )
    .expect("resolve active branch");

    assert_eq!(state.head_commit_sha.as_deref(), Some(head_sha.as_str()));
    assert_eq!(state.head_tree_sha.as_deref(), Some(head_tree_sha.as_str()));
    assert_eq!(state.active_branch.as_deref(), Some(active_branch.as_str()));
    assert_eq!(state.head_tree.len(), 2);
    assert_eq!(state.head_tree.get("src/lib.rs"), Some(&head_blob));
    assert!(state.head_tree.contains_key("README.md"));
    assert!(state.staged_changes.is_empty());
    assert!(state.dirty_files.is_empty());
    assert!(state.untracked_files.is_empty());
}

#[test]
fn workspace_state_reports_dirty_files() {
    let repo = seed_workspace_repo();
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n",
    )
    .expect("rewrite rust source");

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect dirty workspace");

    assert!(state.staged_changes.is_empty());
    assert_eq!(state.dirty_files, vec!["src/lib.rs".to_string()]);
    assert!(state.untracked_files.is_empty());
    assert!(state.head_tree.contains_key("src/lib.rs"));
}

#[test]
fn workspace_state_staged_changes_report_index_diffs() {
    let repo = seed_workspace_repo();
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hey {name}\")\n}\n",
    )
    .expect("rewrite rust source");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "src/lib.rs"]);

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect staged workspace");

    let index_blob = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", ":src/lib.rs"],
    )
    .expect("resolve index blob");
    let staged = state
        .staged_changes
        .get("src/lib.rs")
        .expect("expected staged rust file");
    assert_eq!(
        staged,
        &crate::host::devql::sync::workspace_state::StagedChange::Modified(index_blob)
    );
    assert_eq!(state.staged_changes.len(), 1);
    assert!(state.dirty_files.is_empty());
    assert!(state.untracked_files.is_empty());
}

#[test]
fn workspace_state_reports_staged_deletes() {
    let repo = seed_workspace_repo();
    crate::test_support::git_fixtures::git_ok(repo.path(), &["rm", "src/lib.rs"]);

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect staged delete workspace");

    let staged = state
        .staged_changes
        .get("src/lib.rs")
        .expect("expected staged delete");
    assert_eq!(
        staged,
        &crate::host::devql::sync::workspace_state::StagedChange::Deleted
    );
    assert_eq!(state.staged_changes.len(), 1);
    assert!(state.dirty_files.is_empty());
    assert!(state.untracked_files.is_empty());
}

#[test]
fn workspace_state_reports_untracked_files() {
    let repo = seed_workspace_repo();
    fs::write(
        repo.path().join("src/new_file.rs"),
        "pub fn created() -> i32 {\n    7\n}\n",
    )
    .expect("write untracked rust source");

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect workspace with untracked file");

    assert!(state.staged_changes.is_empty());
    assert!(state.dirty_files.is_empty());
    assert_eq!(state.untracked_files, vec!["src/new_file.rs".to_string()]);
    assert!(!state.head_tree.contains_key("src/new_file.rs"));
}

#[test]
fn workspace_state_filter_limits_results_to_requested_paths() {
    let repo = seed_full_sync_repo();
    let requested_paths = std::collections::HashSet::from(["src/lib.rs".to_string()]);

    let state = crate::host::devql::sync::workspace_state::inspect_workspace_for_paths(
        repo.path(),
        Some(&requested_paths),
    )
    .expect("inspect filtered workspace");

    assert_eq!(state.head_tree.len(), 1);
    assert!(state.head_tree.contains_key("src/lib.rs"));
    assert!(
        state.staged_changes.keys().all(|path| path == "src/lib.rs"),
        "filtered staged changes should only include requested paths"
    );
    assert!(
        state.dirty_files.iter().all(|path| path == "src/lib.rs"),
        "filtered dirty files should only include requested paths"
    );
    assert!(
        state
            .untracked_files
            .iter()
            .all(|path| path == "src/lib.rs"),
        "filtered untracked files should only include requested paths"
    );
}

#[test]
fn workspace_state_unborn_head_reports_raw_workspace_state() {
    let repo = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn draft() -> bool {\n    true\n}\n",
    )
    .expect("write rust source");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "src/lib.rs"]);

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect unborn HEAD");

    let active_branch = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["branch", "--show-current"],
    )
    .expect("resolve active branch");

    assert_eq!(state.head_commit_sha, None);
    assert_eq!(state.head_tree_sha, None);
    assert_eq!(state.active_branch.as_deref(), Some(active_branch.as_str()));
    assert!(state.head_tree.is_empty());
    assert_eq!(state.staged_changes.len(), 1);
    assert_eq!(
        state
            .staged_changes
            .get("src/lib.rs")
            .expect("expected staged rust file"),
        &crate::host::devql::sync::workspace_state::StagedChange::Added(
            crate::host::checkpoints::strategy::manual_commit::run_git(
                repo.path(),
                &["rev-parse", ":src/lib.rs"],
            )
            .expect("resolve staged blob"),
        )
    );
    assert!(state.dirty_files.is_empty());
    assert!(state.untracked_files.is_empty());
}

#[tokio::test]
async fn content_cache_lookup_returns_none_on_cache_miss() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let cached = crate::host::devql::sync::content_cache::lookup_cached_content(
        &relational,
        "content-1",
        "rust",
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("lookup cache entry");

    assert_eq!(cached, None);
}

#[tokio::test]
async fn content_cache_store_then_lookup_roundtrips_payload() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let expected = crate::host::devql::sync::content_cache::CachedExtraction {
        content_id: "content-1".to_string(),
        language: "rust".to_string(),
        parser_version: "parser-v1".to_string(),
        extractor_version: "extractor-v1".to_string(),
        parse_status: "ok".to_string(),
        artefacts: vec![crate::host::devql::sync::content_cache::CachedArtefact {
            artifact_key: "file::src/lib.rs".to_string(),
            canonical_kind: Some("file".to_string()),
            language_kind: "file".to_string(),
            name: "src/lib.rs".to_string(),
            parent_artifact_key: None,
            start_line: 1,
            end_line: 3,
            start_byte: 0,
            end_byte: 48,
            signature: "pub fn greet(name: &str) -> String".to_string(),
            modifiers: vec!["pub".to_string()],
            docstring: Some("Greets a caller.".to_string()),
            metadata: json!({ "symbol_fqn": "src/lib.rs" }),
        }],
        edges: vec![crate::host::devql::sync::content_cache::CachedEdge {
            edge_key: "edge::call".to_string(),
            from_artifact_key: "file::src/lib.rs".to_string(),
            to_artifact_key: None,
            to_symbol_ref: Some("std::fmt::format".to_string()),
            edge_kind: "calls".to_string(),
            start_line: Some(2),
            end_line: Some(2),
            metadata: json!({ "call_form": "macro" }),
        }],
    };

    crate::host::devql::sync::content_cache::store_cached_content(&relational, &expected, "hot")
        .await
        .expect("store cache entry");

    let cached = crate::host::devql::sync::content_cache::lookup_cached_content(
        &relational,
        &expected.content_id,
        &expected.language,
        &expected.parser_version,
        &expected.extractor_version,
    )
    .await
    .expect("lookup stored cache entry")
    .expect("cache entry should exist");

    assert_eq!(cached.content_id, expected.content_id);
    assert_eq!(cached.language, expected.language);
    assert_eq!(cached.parse_status, expected.parse_status);
    assert_eq!(cached.artefacts, expected.artefacts);
    assert_eq!(cached.edges, expected.edges);
}

#[tokio::test]
async fn content_cache_lookup_respects_parser_and_extractor_versions() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let extraction = crate::host::devql::sync::content_cache::CachedExtraction {
        content_id: "content-versions".to_string(),
        language: "rust".to_string(),
        parser_version: "parser-a".to_string(),
        extractor_version: "extractor-a".to_string(),
        parse_status: "ok".to_string(),
        artefacts: vec![crate::host::devql::sync::content_cache::CachedArtefact {
            artifact_key: "fn::demo".to_string(),
            canonical_kind: Some("function".to_string()),
            language_kind: "function_item".to_string(),
            name: "demo".to_string(),
            parent_artifact_key: None,
            start_line: 1,
            end_line: 1,
            start_byte: 0,
            end_byte: 12,
            signature: "fn demo()".to_string(),
            modifiers: vec![],
            docstring: None,
            metadata: json!({}),
        }],
        edges: vec![],
    };

    crate::host::devql::sync::content_cache::store_cached_content(&relational, &extraction, "hot")
        .await
        .expect("store versioned cache entry");

    let version_a = crate::host::devql::sync::content_cache::lookup_cached_content(
        &relational,
        &extraction.content_id,
        &extraction.language,
        &extraction.parser_version,
        &extraction.extractor_version,
    )
    .await
    .expect("lookup version a");

    let version_b = crate::host::devql::sync::content_cache::lookup_cached_content(
        &relational,
        &extraction.content_id,
        &extraction.language,
        "parser-b",
        "extractor-b",
    )
    .await
    .expect("lookup version b");

    assert_eq!(version_a, Some(extraction));
    assert_eq!(version_b, None);
}

#[test]
fn sync_extraction_converts_typescript_content_to_cache_format() {
    let cfg = sync_test_cfg();
    let path = "src/sample.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());

    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract TypeScript content into cache format")
    .expect("TypeScript cache extraction should be supported");

    assert_eq!(extraction.content_id, content_id);
    assert_eq!(extraction.language, "typescript");
    assert_eq!(extraction.parser_version, "tree-sitter-ts@1");
    assert_eq!(extraction.extractor_version, "ts-language-pack@1");
    assert_eq!(extraction.parse_status, "ok");

    let repeated = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("repeat extract TypeScript content into cache format")
    .expect("repeated TypeScript cache extraction should be supported");
    assert_eq!(
        extraction, repeated,
        "cache extraction should be deterministic"
    );

    let file = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("file") && artefact.name == path
        })
        .expect("expected file artefact");
    let class = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.language_kind == "class_declaration" && artefact.name == "Service"
        })
        .expect("expected class artefact");
    let method = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("method") && artefact.name == "run"
        })
        .expect("expected method artefact");
    let helper = extraction
        .artefacts
        .iter()
        .find(|artefact| {
            artefact.canonical_kind.as_deref() == Some("function") && artefact.name == "localHelper"
        })
        .expect("expected local helper artefact");

    assert!(
        !file.artifact_key.is_empty(),
        "file artefact key should be deterministic and non-empty"
    );
    assert!(
        !class.artifact_key.is_empty(),
        "class artefact key should be deterministic and non-empty"
    );
    assert!(
        !method.artifact_key.is_empty(),
        "method artefact key should be deterministic and non-empty"
    );
    assert!(
        !helper.artifact_key.is_empty(),
        "helper artefact key should be deterministic and non-empty"
    );
    assert_eq!(
        class.parent_artifact_key.as_deref(),
        Some(file.artifact_key.as_str())
    );
    assert_eq!(
        method.parent_artifact_key.as_deref(),
        Some(class.artifact_key.as_str())
    );
    assert_eq!(
        helper.parent_artifact_key.as_deref(),
        Some(file.artifact_key.as_str())
    );

    let same_file_call = extraction
        .edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "calls"
                && edge.from_artifact_key == method.artifact_key
                && edge.to_artifact_key.as_deref() == Some(helper.artifact_key.as_str())
        })
        .expect("expected same-file call edge");
    assert!(
        !same_file_call.edge_key.is_empty(),
        "same-file edge key should be deterministic and non-empty"
    );
    assert_eq!(same_file_call.to_symbol_ref, None);

    let cross_file_call = extraction
        .edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "calls"
                && edge.from_artifact_key == method.artifact_key
                && edge.to_symbol_ref.as_deref() == Some("./remote::remoteFoo")
        })
        .expect("expected cross-file call edge");
    assert!(
        !cross_file_call.edge_key.is_empty(),
        "cross-file edge key should be deterministic and non-empty"
    );
    assert_eq!(cross_file_call.to_artifact_key, None);

    let import_edge = extraction
        .edges
        .iter()
        .find(|edge| {
            edge.edge_kind == "imports"
                && edge.from_artifact_key == file.artifact_key
                && edge.to_symbol_ref.as_deref() == Some("./remote")
        })
        .expect("expected file-level import edge");
    assert!(
        !import_edge.edge_key.is_empty(),
        "import edge key should be deterministic and non-empty"
    );
    assert_eq!(import_edge.to_artifact_key, None);
}

#[test]
fn sync_extraction_uses_path_agnostic_artifact_keys_for_same_content() {
    let cfg = sync_test_cfg();
    let content = r#"class Service {
  run(): number {
    return localHelper();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());

    let first = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        "src/sample.ts",
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract first TypeScript path")
    .expect("first TypeScript cache extraction should be supported");
    let second = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        "nested/other.ts",
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract second TypeScript path")
    .expect("second TypeScript cache extraction should be supported");

    let key_for = |extraction: &crate::host::devql::sync::content_cache::CachedExtraction,
                   name: &str,
                   language_kind: &str| {
        extraction
            .artefacts
            .iter()
            .find(|artefact| artefact.name == name && artefact.language_kind == language_kind)
            .map(|artefact| artefact.artifact_key.clone())
            .expect("expected artefact key")
    };

    assert_eq!(
        first
            .artefacts
            .iter()
            .find(|artefact| artefact.canonical_kind.as_deref() == Some("file"))
            .map(|artefact| artefact.artifact_key.clone()),
        second
            .artefacts
            .iter()
            .find(|artefact| artefact.canonical_kind.as_deref() == Some("file"))
            .map(|artefact| artefact.artifact_key.clone())
    );
    assert_eq!(
        key_for(&first, "Service", "class_declaration"),
        key_for(&second, "Service", "class_declaration")
    );
    assert_eq!(
        key_for(&first, "run", "method_definition"),
        key_for(&second, "run", "method_definition")
    );
    assert_eq!(
        key_for(&first, "localHelper", "function_declaration"),
        key_for(&second, "localHelper", "function_declaration")
    );

    let same_file_edge_key =
        |extraction: &crate::host::devql::sync::content_cache::CachedExtraction| {
            extraction
                .edges
                .iter()
                .find(|edge| edge.edge_kind == "calls" && edge.to_artifact_key.is_some())
                .map(|edge| edge.edge_key.clone())
                .expect("expected same-file edge key")
        };

    assert_eq!(same_file_edge_key(&first), same_file_edge_key(&second));
}

#[tokio::test]
async fn materialize_writes_artefacts_current_with_correct_symbol_id() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;
    let path = "src/sample.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = desired_file_state(path, "typescript", &content_id);
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract TypeScript content into cache format")
    .expect("TypeScript cache extraction should be supported");
    let rev = crate::host::devql::FileRevision {
        commit_sha: &content_id,
        revision: crate::host::devql::TemporalRevisionRef {
            kind: crate::host::devql::TemporalRevisionKind::Temporary,
            id: &content_id,
            temp_checkpoint_id: None,
        },
        commit_unix: 0,
        path,
        blob_sha: &content_id,
    };
    let (items, _, _) = crate::host::devql::extract_language_pack_artefacts_and_edges(
        &cfg,
        &rev,
        "typescript",
        content,
    )
    .expect("extract expected TypeScript artefacts")
    .expect("TypeScript artefacts should be supported");
    let expected_symbol_ids = expected_symbol_id_by_fqn(&items, path);

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached path");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let mut stmt = db
        .prepare(
            "SELECT symbol_fqn, symbol_id, artefact_id \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND path = ?2 \
             ORDER BY symbol_fqn",
        )
        .expect("prepare current artefacts query");
    let rows = stmt
        .query_map([cfg.repo.repo_id.as_str(), path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query current artefacts")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect current artefacts");

    let mut expected = expected_symbol_ids
        .into_iter()
        .map(|(symbol_fqn, symbol_id)| {
            let artefact_id = crate::host::devql::revision_artefact_id(
                &cfg.repo.repo_id,
                &content_id,
                &symbol_id,
            );
            (symbol_fqn, symbol_id, artefact_id)
        })
        .collect::<Vec<_>>();
    expected.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));

    assert_eq!(rows, expected);
}

#[tokio::test]
async fn materialize_then_re_materialize_is_idempotent() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;
    let path = "src/sample.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = desired_file_state(path, "typescript", &content_id);
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract TypeScript content into cache format")
    .expect("TypeScript cache extraction should be supported");
    let rev = crate::host::devql::FileRevision {
        commit_sha: &content_id,
        revision: crate::host::devql::TemporalRevisionRef {
            kind: crate::host::devql::TemporalRevisionKind::Temporary,
            id: &content_id,
            temp_checkpoint_id: None,
        },
        commit_unix: 0,
        path,
        blob_sha: &content_id,
    };
    let (items, _, _) = crate::host::devql::extract_language_pack_artefacts_and_edges(
        &cfg,
        &rev,
        "typescript",
        content,
    )
    .expect("extract expected TypeScript artefacts")
    .expect("TypeScript artefacts should be supported");
    let expected_symbol_ids = expected_symbol_id_by_fqn(&items, path);
    let helper_symbol_id = expected_symbol_ids
        .get(&format!("{path}::localHelper"))
        .cloned()
        .expect("expected localHelper symbol id");
    let helper_artefact_id =
        crate::host::devql::revision_artefact_id(&cfg.repo.repo_id, &content_id, &helper_symbol_id);

    let load_artefacts = |db: &Connection| {
        let mut stmt = db
            .prepare(
                "SELECT symbol_fqn, symbol_id, artefact_id, parent_symbol_id, parent_artefact_id \
                 FROM artefacts_current \
                 WHERE repo_id = ?1 AND path = ?2 \
                 ORDER BY symbol_fqn",
            )
            .expect("prepare artefacts_current query");
        stmt.query_map([cfg.repo.repo_id.as_str(), path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .expect("query artefacts_current rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect artefacts_current rows")
    };
    let load_edges = |db: &Connection| {
        let mut stmt = db
            .prepare(
                "SELECT edge_id, from_symbol_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind \
                 FROM artefact_edges_current \
                 WHERE repo_id = ?1 AND path = ?2 \
                 ORDER BY edge_id",
            )
            .expect("prepare artefact_edges_current query");
        stmt.query_map([cfg.repo.repo_id.as_str(), path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .expect("query artefact_edges_current rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect artefact_edges_current rows")
    };
    let load_current_state = |db: &Connection| {
        db.query_row(
            "SELECT language, head_content_id, index_content_id, worktree_content_id, effective_content_id, effective_source, parser_version, extractor_version, exists_in_head, exists_in_index, exists_in_worktree \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .expect("load current_file_state row")
    };

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached path first time");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let first_artefacts = load_artefacts(&db);
    let first_edges = load_edges(&db);
    let first_state = load_current_state(&db);

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached path second time");

    let second_artefacts = load_artefacts(&db);
    let second_edges = load_edges(&db);
    let second_state = load_current_state(&db);

    assert_eq!(first_artefacts, second_artefacts);
    assert_eq!(first_edges, second_edges);
    assert_eq!(first_state, second_state);
    assert_eq!(first_artefacts.len(), extraction.artefacts.len());
    assert_eq!(first_edges.len(), extraction.edges.len());
    assert_eq!(
        first_state,
        (
            "typescript".to_string(),
            Some(content_id.clone()),
            Some(content_id.clone()),
            Some(content_id.clone()),
            content_id.clone(),
            "head".to_string(),
            "tree-sitter-ts@1".to_string(),
            "ts-language-pack@1".to_string(),
            1,
            1,
            1,
        )
    );
    assert!(
        first_edges.iter().any(|edge| {
            edge.2.as_deref() == Some(helper_symbol_id.as_str())
                && edge.3.as_deref() == Some(helper_artefact_id.as_str())
                && edge.5 == "calls"
        }),
        "same-file call edge should resolve through cached artifact_key mapping"
    );
}

#[tokio::test]
async fn materialize_reuses_cached_extraction_at_new_path_with_path_sensitive_identity() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;
    let original_path = "src/sample.ts";
    let materialized_path = "nested/other.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        original_path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract original TypeScript content into cache format")
    .expect("original TypeScript cache extraction should be supported");
    let desired = desired_file_state(materialized_path, "typescript", &content_id);
    let rev = crate::host::devql::FileRevision {
        commit_sha: &content_id,
        revision: crate::host::devql::TemporalRevisionRef {
            kind: crate::host::devql::TemporalRevisionKind::Temporary,
            id: &content_id,
            temp_checkpoint_id: None,
        },
        commit_unix: 0,
        path: materialized_path,
        blob_sha: &content_id,
    };
    let (items, _, _) = crate::host::devql::extract_language_pack_artefacts_and_edges(
        &cfg,
        &rev,
        "typescript",
        content,
    )
    .expect("extract expected TypeScript artefacts for new path")
    .expect("TypeScript artefacts for new path should be supported");
    let expected_symbol_ids = expected_symbol_id_by_fqn(&items, materialized_path);

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached extraction at new path");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let mut stmt = db
        .prepare(
            "SELECT symbol_fqn, symbol_id, artefact_id \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND path = ?2 \
             ORDER BY symbol_fqn",
        )
        .expect("prepare current artefacts query");
    let rows = stmt
        .query_map([cfg.repo.repo_id.as_str(), materialized_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query current artefacts")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect current artefacts");

    let mut expected = expected_symbol_ids
        .into_iter()
        .map(|(symbol_fqn, symbol_id)| {
            let artefact_id = crate::host::devql::revision_artefact_id(
                &cfg.repo.repo_id,
                &content_id,
                &symbol_id,
            );
            (symbol_fqn, symbol_id, artefact_id)
        })
        .collect::<Vec<_>>();
    expected.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));

    assert!(
        rows.iter()
            .all(|(symbol_fqn, _, _)| symbol_fqn.starts_with(materialized_path)),
        "all symbol_fqn values should be re-derived from the materialization path"
    );
    assert!(
        rows.iter()
            .all(|(symbol_fqn, _, _)| !symbol_fqn.starts_with(original_path)),
        "stored symbol_fqn values should not retain the cached source path"
    );
    assert_eq!(rows, expected);
}

#[tokio::test]
async fn remove_path_deletes_all_rows() {
    let cfg = sync_test_cfg();
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    seed_sync_repository_catalog_row(&relational, &cfg).await;
    let path = "src/sample.ts";
    let content = r#"import { remoteFoo } from "./remote";

class Service {
  run(): number {
    return localHelper() + remoteFoo();
  }
}

function localHelper(): number {
  return 1;
}
"#;
    let content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(content.as_bytes());
    let desired = desired_file_state(path, "typescript", &content_id);
    let extraction = crate::host::devql::sync::extraction::extract_to_cache_format(
        &cfg,
        path,
        &content_id,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
        content,
    )
    .expect("extract TypeScript content into cache format")
    .expect("TypeScript cache extraction should be supported");

    crate::host::devql::sync::materializer::materialize_path(
        &cfg,
        &relational,
        &desired,
        &extraction,
        "tree-sitter-ts@1",
        "ts-language-pack@1",
    )
    .await
    .expect("materialize cached path");
    crate::host::devql::sync::materializer::remove_path(&cfg, &relational, path)
        .await
        .expect("remove materialized path");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let artefact_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count artefacts_current rows");
    let edge_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count artefact_edges_current rows");
    let current_file_state_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count current_file_state rows");

    assert_eq!(artefact_count, 0);
    assert_eq!(edge_count, 0);
    assert_eq!(current_file_state_count, 0);
}

#[tokio::test]
async fn full_sync_indexes_all_supported_files() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let summary = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync");

    assert!(summary.success, "full sync should report success");
    assert_eq!(summary.paths_added, 4);
    assert_eq!(summary.paths_changed, 0);
    assert_eq!(summary.paths_removed, 0);
    assert_eq!(summary.paths_unchanged, 0);

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let mut stmt = db
        .prepare(
            "SELECT path, language, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 \
             ORDER BY path",
        )
        .expect("prepare current_file_state query");
    let rows = stmt
        .query_map([cfg.repo.repo_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query current_file_state rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect current_file_state rows");

    assert_eq!(
        rows,
        vec![
            (
                "scripts/main.py".to_string(),
                "python".to_string(),
                "head".to_string()
            ),
            (
                "src/lib.rs".to_string(),
                "rust".to_string(),
                "head".to_string()
            ),
            (
                "web/app.ts".to_string(),
                "typescript".to_string(),
                "head".to_string()
            ),
            (
                "web/util.js".to_string(),
                "javascript".to_string(),
                "head".to_string()
            ),
        ]
    );

    let cache_count: i64 = db
        .query_row("SELECT COUNT(*) FROM content_cache", [], |row| row.get(0))
        .expect("count content_cache rows");
    let artefact_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefacts_current rows");
    let edge_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefact_edges_current rows");
    let sync_state: (String, String, Option<String>, Option<String>) = db
        .query_row(
            "SELECT last_sync_status, last_sync_reason, active_branch, head_commit_sha \
             FROM repo_sync_state WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read repo_sync_state row");
    let repository_row: (String, String, String) = db
        .query_row(
            "SELECT provider, organization, name FROM repositories WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read repositories row");

    assert_eq!(cache_count, 4, "supported files should be cached once each");
    assert!(
        artefact_count >= 8,
        "full sync should materialize file and symbol artefacts"
    );
    assert!(
        edge_count >= 2,
        "full sync should materialize dependency edges for supported files"
    );
    assert_eq!(sync_state.0, "completed");
    assert_eq!(sync_state.1, "full");
    assert_eq!(sync_state.2.as_deref(), Some("main"));
    assert_eq!(
        repository_row,
        (
            cfg.repo.provider.clone(),
            cfg.repo.organization.clone(),
            cfg.repo.name.clone()
        )
    );
    assert!(
        sync_state.3.is_some(),
        "completed sync should persist the resolved HEAD commit"
    );
}

#[tokio::test]
async fn sync_validate_reports_clean_state_then_detects_stale_rows() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync before validation");

    let clean = crate::host::devql::execute_sync_validation(&cfg, &relational)
        .await
        .expect("execute sync validation for clean state");
    let clean_report = clean.validation.expect("validation report");
    assert!(
        clean_report.valid,
        "freshly synced state should validate clean"
    );
    assert!(
        clean_report.files_with_drift.is_empty(),
        "clean validation should not report per-file drift"
    );

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    db.execute(
        "INSERT INTO artefacts_current (
            repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind,
            language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
            start_byte, end_byte, signature, modifiers, docstring, updated_at
         )
         SELECT
            repo_id, path, content_id, 'sync-validation-stale-symbol', 'sync-validation-stale-artefact',
            language, canonical_kind, language_kind, symbol_fqn || '::stale', parent_symbol_id,
            parent_artefact_id, start_line, end_line, start_byte, end_byte, signature,
            modifiers, docstring, datetime('now')
         FROM artefacts_current
         WHERE repo_id = ?1
         LIMIT 1",
        [cfg.repo.repo_id.as_str()],
    )
    .expect("insert stale artefact row");

    let drift = crate::host::devql::execute_sync_validation(&cfg, &relational)
        .await
        .expect("execute sync validation with stale rows");
    let drift_report = drift.validation.expect("validation report");
    assert!(
        !drift_report.valid,
        "validation should fail when stale rows exist"
    );
    assert!(
        drift_report.stale_artefacts >= 1,
        "stale artefacts should be counted in the validation report"
    );
    assert!(
        drift_report
            .files_with_drift
            .iter()
            .any(|file| file.stale_artefacts >= 1),
        "stale artefact drift should be attributed to at least one file"
    );
}

#[tokio::test]
async fn auto_sync_uses_full_reason_and_git_backed_retention_for_clean_head_files() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let summary = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Auto,
    )
    .await
    .expect("execute auto sync");

    assert_eq!(summary.mode, "full");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let sync_reason: String = db
        .query_row(
            "SELECT last_sync_reason FROM repo_sync_state WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("read repo_sync_state reason");
    let retention_classes = {
        let mut stmt = db
            .prepare(
                "SELECT DISTINCT retention_class \
                 FROM content_cache \
                 ORDER BY retention_class",
            )
            .expect("prepare retention class query");
        stmt.query_map([], |row| row.get::<_, String>(0))
            .expect("query retention classes")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect retention classes")
    };

    assert_eq!(sync_reason, "full");
    assert_eq!(retention_classes, vec!["git_backed".to_string()]);
}

#[tokio::test]
async fn auto_sync_marks_dirty_worktree_content_as_worktree_only_retention() {
    let repo = seed_full_sync_repo();
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n",
    )
    .expect("rewrite rust source in worktree");

    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let summary = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Auto,
    )
    .await
    .expect("execute auto sync with dirty worktree");

    assert_eq!(summary.mode, "full");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let effective_source: String = db
        .query_row(
            "SELECT effective_source FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/lib.rs"],
            |row| row.get(0),
        )
        .expect("read current_file_state effective source");
    let retention_class: String = db
        .query_row(
            "SELECT retention_class \
             FROM content_cache c \
             JOIN current_file_state s \
               ON s.effective_content_id = c.content_id \
              AND s.language = c.language \
             WHERE s.repo_id = ?1 AND s.path = ?2",
            [cfg.repo.repo_id.as_str(), "src/lib.rs"],
            |row| row.get(0),
        )
        .expect("read content_cache retention class");

    assert_eq!(effective_source, "worktree");
    assert_eq!(retention_class, "worktree_only");
}

#[tokio::test]
async fn execute_sync_preserves_original_error_when_failed_status_write_fails() {
    let temp = tempdir().expect("temp dir");
    let sqlite_path = temp.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let mut cfg = sync_test_cfg_for_repo(temp.path());
    cfg.repo_root = temp.path().join("missing-repo");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    db.execute_batch(
        r#"
CREATE TRIGGER fail_sync_failed_status
BEFORE UPDATE OF last_sync_status ON repo_sync_state
WHEN NEW.last_sync_status = 'failed'
BEGIN
    SELECT RAISE(FAIL, 'forced write_sync_failed failure');
END;
"#,
    )
    .expect("create failing repo_sync_state trigger");

    let err = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect_err("sync should fail when repo_root is not a git workspace");
    let message = format!("{err:#}");

    assert!(
        message.contains("inspecting workspace for DevQL sync"),
        "returned error should preserve the original inner sync failure: {message}"
    );
    assert!(
        !message.contains("forced write_sync_failed failure"),
        "write_sync_failed failure must not mask the original sync failure: {message}"
    );
}

#[tokio::test]
async fn sync_twice_with_no_changes_is_noop() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let first = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute initial full sync");
    assert_eq!(first.paths_added, 4);
    assert_eq!(first.paths_changed, 0);
    assert_eq!(first.paths_removed, 0);
    assert_eq!(first.paths_unchanged, 0);

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let artefacts_before: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefacts before second sync");

    let second = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute second full sync");

    let artefacts_after: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count artefacts after second sync");

    assert_eq!(second.paths_unchanged, 4);
    assert_eq!(second.paths_added, 0);
    assert_eq!(second.paths_changed, 0);
    assert_eq!(second.paths_removed, 0);
    assert_eq!(second.cache_hits, 0);
    assert_eq!(second.cache_misses, 0);
    assert_eq!(artefacts_before, artefacts_after);
}

#[tokio::test]
async fn sync_detects_file_edit() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let original_content = "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n";
    let edited_content = "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n";
    let original_blob =
        crate::host::devql::sync::content_identity::compute_blob_oid(original_content.as_bytes());

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute initial full sync");

    fs::write(repo.path().join("src/lib.rs"), edited_content).expect("edit tracked source file");

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after edit");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let current_state: (String, String) = db
        .query_row(
            "SELECT effective_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/lib.rs"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current_file_state for edited path");
    let artefact_content_ids = {
        let mut stmt = db
            .prepare(
                "SELECT content_id FROM artefacts_current \
                 WHERE repo_id = ?1 AND path = ?2 \
                 ORDER BY symbol_id",
            )
            .expect("prepare artefacts_current content query");
        stmt.query_map([cfg.repo.repo_id.as_str(), "src/lib.rs"], |row| {
            row.get::<_, String>(0)
        })
        .expect("query artefacts_current content ids")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect artefacts_current content ids")
    };
    let old_content_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2 AND content_id = ?3",
            [cfg.repo.repo_id.as_str(), "src/lib.rs", original_blob.as_str()],
            |row| row.get(0),
        )
        .expect("count artefacts with previous content id");

    let edited_blob = crate::host::devql::sync::content_identity::compute_blob_oid(
        &fs::read(repo.path().join("src/lib.rs")).expect("read edited worktree file"),
    );

    assert_eq!(result.paths_changed, 1);
    assert_eq!(result.paths_added, 0);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(result.paths_unchanged, 3);
    assert_eq!(current_state.0, edited_blob);
    assert_eq!(current_state.1, "worktree");
    assert!(!artefact_content_ids.is_empty());
    assert!(
        artefact_content_ids
            .iter()
            .all(|content_id| content_id == &edited_blob),
        "all materialized rows for src/lib.rs should reflect the edited content"
    );
    assert_eq!(
        old_content_count, 0,
        "no artefacts_current rows should remain for the previous content id"
    );
}

#[tokio::test]
async fn dirty_then_commit_unchanged_is_cache_hit() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/lib.rs";
    let edited_content = "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n";

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute initial full sync");

    fs::write(repo.path().join(path), edited_content).expect("edit tracked source file");

    let dirty_sync = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after dirty edit");

    assert_eq!(dirty_sync.cache_hits, 0);
    assert_eq!(dirty_sync.cache_misses, 1);

    crate::host::checkpoints::strategy::manual_commit::run_git(repo.path(), &["add", path])
        .expect("stage edited source file");
    crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["commit", "-m", "commit edited source"],
    )
    .expect("commit edited source file");

    let committed_sync = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after committing unchanged content");

    assert_eq!(committed_sync.cache_hits, 1);
    assert_eq!(committed_sync.cache_misses, 0);

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let current_state: (String, String) = db
        .query_row(
            "SELECT effective_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current_file_state after commit");

    assert_eq!(
        current_state.0,
        crate::host::devql::sync::content_identity::compute_blob_oid(edited_content.as_bytes())
    );
    assert_eq!(current_state.1, "head");

    let retention_class: String = db
        .query_row(
            "SELECT retention_class \
             FROM content_cache \
             WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
            [
                current_state.0.as_str(),
                "rust",
                committed_sync.parser_version.as_str(),
                committed_sync.extractor_version.as_str(),
            ],
            |row| row.get(0),
        )
        .expect("read content_cache retention class");
    assert_eq!(retention_class, "git_backed");
}

#[tokio::test]
async fn cached_parse_error_hit_updates_manifest_and_clears_materialized_rows() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/lib.rs";
    let broken_content = "fn broken( {\n";

    let baseline = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute baseline full sync");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let baseline_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count baseline rows for parse-error path");
    assert!(
        baseline_rows > 0,
        "baseline sync should materialize at least one row for parse-error path"
    );

    fs::write(repo.path().join(path), broken_content).expect("write broken source content");
    let broken_content_id =
        crate::host::devql::sync::content_identity::compute_blob_oid(broken_content.as_bytes());
    let parse_error_payload = crate::host::devql::sync::content_cache::CachedExtraction {
        content_id: broken_content_id.clone(),
        language: "rust".to_string(),
        parser_version: baseline.parser_version.clone(),
        extractor_version: baseline.extractor_version.clone(),
        parse_status: crate::host::devql::sync::extraction::PARSE_STATUS_PARSE_ERROR.to_string(),
        artefacts: vec![],
        edges: vec![],
    };
    crate::host::devql::sync::content_cache::store_cached_content(
        &relational,
        &parse_error_payload,
        "worktree_only",
    )
    .await
    .expect("store parse-error cache payload");

    let summary = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync with cached parse-error payload");

    assert_eq!(summary.paths_changed, 1);
    assert_eq!(summary.cache_hits, 1);
    assert_eq!(summary.cache_misses, 0);
    assert_eq!(summary.parse_errors, 1);

    let rows_after: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| row.get(0),
        )
        .expect("count rows after parse-error materialization");
    assert_eq!(
        rows_after, 0,
        "parse-error payload should clear materialized rows for the path"
    );

    let current_state: (String, String) = db
        .query_row(
            "SELECT effective_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current_file_state after parse-error sync");
    assert_eq!(current_state.0, broken_content_id);
    assert_eq!(current_state.1, "worktree");

    let parse_status: String = db
        .query_row(
            "SELECT parse_status FROM content_cache \
             WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
            [
                current_state.0.as_str(),
                "rust",
                summary.parser_version.as_str(),
                summary.extractor_version.as_str(),
            ],
            |row| row.get(0),
        )
        .expect("read parse_status for parse-error cache row");
    assert_eq!(
        parse_status,
        crate::host::devql::sync::extraction::PARSE_STATUS_PARSE_ERROR
    );
}

#[tokio::test]
async fn branch_switch_reuses_cache() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/lib.rs";
    let feature_content =
        "pub fn greet(name: &str) -> String {\n    format!(\"feature {name}\")\n}\n";

    let initial_sync = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute initial full sync on main");

    assert_eq!(initial_sync.cache_hits, 0);
    assert_eq!(initial_sync.cache_misses, initial_sync.paths_added);
    assert!(initial_sync.cache_misses > 0);

    crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["checkout", "-b", "feature/cache-reuse"],
    )
    .expect("create feature branch");
    fs::write(repo.path().join(path), feature_content).expect("edit tracked source file");
    crate::host::checkpoints::strategy::manual_commit::run_git(repo.path(), &["add", path])
        .expect("stage feature change");
    crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["commit", "-m", "feature branch change"],
    )
    .expect("commit feature branch change");

    let feature_sync = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync on feature branch");

    assert_eq!(feature_sync.cache_hits, 0);
    assert_eq!(feature_sync.cache_misses, 1);

    crate::host::checkpoints::strategy::manual_commit::run_git(repo.path(), &["checkout", "main"])
        .expect("checkout main branch");

    let main_sync = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after returning to main");

    assert_eq!(main_sync.cache_hits, 1);
    assert_eq!(main_sync.cache_misses, 0);

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let current_state: (String, String) = db
        .query_row(
            "SELECT effective_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current_file_state after branch switch");

    assert_eq!(
        current_state.0,
        crate::host::devql::sync::content_identity::compute_blob_oid(
            "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n".as_bytes()
        )
    );
    assert_eq!(current_state.1, "head");
}

#[tokio::test]
async fn staged_content_takes_precedence_over_head() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/lib.rs";
    let staged_content =
        "pub fn greet(name: &str) -> String {\n    format!(\"staged {name}\")\n}\n";

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute baseline full sync");

    let baseline_supported_paths: i64 = Connection::open(&sqlite_path)
        .expect("open sqlite db")
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count baseline supported paths");

    fs::write(repo.path().join(path), staged_content).expect("edit tracked source file");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", path]);

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync with staged content");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let current_state: (String, Option<String>, String, Option<String>) = db
        .query_row(
            "SELECT effective_content_id, index_content_id, effective_source, head_content_id \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read current_file_state for staged path");
    let staged_blob =
        crate::host::devql::sync::content_identity::compute_blob_oid(staged_content.as_bytes());

    assert_eq!(result.paths_changed, 1);
    assert_eq!(result.paths_added, 0);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(
        result.paths_changed + result.paths_unchanged,
        baseline_supported_paths as usize
    );
    assert_eq!(current_state.0, staged_blob);
    assert_eq!(current_state.1.as_deref(), Some(staged_blob.as_str()));
    assert_eq!(current_state.2, "index");
    assert_ne!(current_state.3.as_deref(), Some(staged_blob.as_str()));
}

#[tokio::test]
async fn unborn_head_syncs_from_index_and_worktree() {
    let repo = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn draft() -> bool {\n    true\n}\n",
    )
    .expect("write supported source file");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "src/lib.rs"]);
    let staged_blob = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", ":src/lib.rs"],
    )
    .expect("resolve staged blob");

    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync for unborn HEAD repo");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let current_paths = {
        let mut stmt = db
            .prepare(
                "SELECT path \
                 FROM current_file_state \
                 WHERE repo_id = ?1 \
                 ORDER BY path",
            )
            .expect("prepare current_file_state path query");
        stmt.query_map([cfg.repo.repo_id.as_str()], |row| row.get::<_, String>(0))
            .expect("query current_file_state paths")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect current_file_state paths")
    };
    let current_state: (String, Option<String>, Option<String>, Option<String>, String) = db
        .query_row(
            "SELECT effective_content_id, index_content_id, worktree_content_id, head_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "src/lib.rs"],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("read current_file_state for unborn HEAD path");

    assert!(result.success, "unborn-head full sync should succeed");
    assert!(result.paths_added >= 1);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(result.paths_changed, 0);
    assert_eq!(current_paths, vec!["src/lib.rs".to_string()]);
    assert_eq!(current_state.0, staged_blob);
    assert_eq!(current_state.1.as_deref(), Some(staged_blob.as_str()));
    assert_eq!(current_state.2.as_deref(), Some(staged_blob.as_str()));
    assert_eq!(current_state.3, None);
    assert_eq!(current_state.4, "index");
}

#[tokio::test]
async fn unsupported_file_ignored_supported_file_added() {
    let repo = seed_supported_and_unsupported_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let current_paths = {
        let mut stmt = db
            .prepare(
                "SELECT path \
                 FROM current_file_state \
                 WHERE repo_id = ?1 \
                 ORDER BY path",
            )
            .expect("prepare current_file_state path query");
        stmt.query_map([cfg.repo.repo_id.as_str()], |row| row.get::<_, String>(0))
            .expect("query current_file_state paths")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect current_file_state paths")
    };
    let unsupported_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "docs/notes.foo"],
            |row| row.get(0),
        )
        .expect("count unsupported current_file_state rows");

    assert!(
        result.success,
        "sync should succeed with ignored unsupported files"
    );
    assert_eq!(result.paths_added, 1);
    assert_eq!(result.paths_changed, 0);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(result.paths_unchanged, 0);
    assert_eq!(current_paths, vec!["src/lib.rs".to_string()]);
    assert_eq!(unsupported_rows, 0);
}

#[tokio::test]
async fn path_scoped_sync_only_updates_specified_paths() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let scoped_path = "src/lib.rs";
    let unscoped_path = "web/app.ts";
    let scoped_content =
        "pub fn greet(name: &str) -> String {\n    format!(\"scoped {name}\")\n}\n";
    let unscoped_content = "import { helper } from \"./util\";\n\nexport function run(): number {\n  return helper() + 1;\n}\n";

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute baseline full sync");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let load_artefacts = |db: &Connection, path: &str| {
        let mut stmt = db
            .prepare(
                "SELECT content_id, symbol_fqn, symbol_id, artefact_id \
                 FROM artefacts_current \
                 WHERE repo_id = ?1 AND path = ?2 \
                 ORDER BY symbol_fqn",
            )
            .expect("prepare artefacts_current query");
        stmt.query_map([cfg.repo.repo_id.as_str(), path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .expect("query artefacts_current rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect artefacts_current rows")
    };
    let load_current_state = |db: &Connection, path: &str| {
        db.query_row(
            "SELECT language, head_content_id, index_content_id, worktree_content_id, effective_content_id, effective_source, parser_version, extractor_version, exists_in_head, exists_in_index, exists_in_worktree \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .expect("load current_file_state row")
    };

    let baseline_scoped_artefacts = load_artefacts(&db, scoped_path);
    let baseline_unscoped_artefacts = load_artefacts(&db, unscoped_path);
    let baseline_scoped_state = load_current_state(&db, scoped_path);
    let baseline_unscoped_state = load_current_state(&db, unscoped_path);

    fs::write(repo.path().join(scoped_path), scoped_content).expect("edit scoped file");
    fs::write(repo.path().join(unscoped_path), unscoped_content).expect("edit unscoped file");

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Paths(vec![scoped_path.to_string()]),
    )
    .await
    .expect("execute path-scoped sync");

    let scoped_blob =
        crate::host::devql::sync::content_identity::compute_blob_oid(scoped_content.as_bytes());
    let scoped_state = load_current_state(&db, scoped_path);
    let unscoped_state = load_current_state(&db, unscoped_path);
    let scoped_artefacts = load_artefacts(&db, scoped_path);
    let unscoped_artefacts = load_artefacts(&db, unscoped_path);

    assert_eq!(result.paths_changed, 1);
    assert_eq!(result.paths_added, 0);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(result.paths_unchanged, 0);
    assert_eq!(result.cache_hits, 0);
    assert_eq!(result.cache_misses, 1);
    assert_eq!(scoped_state.4, scoped_blob);
    assert_eq!(scoped_state.5, "worktree");
    assert_eq!(unscoped_state, baseline_unscoped_state);
    assert_eq!(unscoped_artefacts, baseline_unscoped_artefacts);
    assert_eq!(baseline_scoped_artefacts.len(), scoped_artefacts.len());
    assert_ne!(scoped_artefacts, baseline_scoped_artefacts);
    assert_eq!(
        unscoped_state.4, baseline_unscoped_state.4,
        "unscoped path should keep the previously materialized content id"
    );
    assert_eq!(
        baseline_scoped_state.4,
        crate::host::devql::sync::content_identity::compute_blob_oid(
            "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n".as_bytes()
        ),
        "baseline scoped state should still reflect the original materialization"
    );
    assert!(
        scoped_artefacts.iter().all(|row| row.0 == scoped_blob),
        "scoped artefacts should reflect the edited content"
    );
}

#[tokio::test]
async fn repair_mode_reprocesses_all_paths_using_cache_when_available() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;
    let path = "src/lib.rs";

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute baseline full sync");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let baseline_state: (String, String, String) = db
        .query_row(
            "SELECT effective_content_id, effective_source, parser_version \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read baseline current_file_state row");
    let baseline_versions: (String, String) = db
        .query_row(
            "SELECT parser_version, extractor_version \
             FROM repo_sync_state \
             WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read baseline sync versions");
    let expected_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1",
            [cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count baseline supported paths");
    let baseline_retention_class: String = db
        .query_row(
            "SELECT retention_class \
             FROM content_cache \
             WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
            [
                baseline_state.0.as_str(),
                "rust",
                baseline_versions.0.as_str(),
                baseline_versions.1.as_str(),
            ],
            |row| row.get(0),
        )
        .expect("read baseline retention class");

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Repair,
    )
    .await
    .expect("execute repair sync");

    let repaired_state: (String, String, String) = db
        .query_row(
            "SELECT effective_content_id, effective_source, parser_version \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read repaired current_file_state row");
    let retention_class: String = db
        .query_row(
            "SELECT retention_class \
             FROM content_cache \
             WHERE content_id = ?1 AND language = ?2 AND parser_version = ?3 AND extractor_version = ?4",
            [
                repaired_state.0.as_str(),
                "rust",
                result.parser_version.as_str(),
                result.extractor_version.as_str(),
            ],
            |row| row.get(0),
        )
        .expect("read repaired retention class");

    assert_eq!(result.paths_changed as i64, expected_count);
    assert_eq!(result.paths_added, 0);
    assert_eq!(result.paths_removed, 0);
    assert_eq!(result.paths_unchanged, 0);
    assert_eq!(result.cache_hits as i64, expected_count);
    assert_eq!(result.cache_misses, 0);
    assert_eq!(repaired_state, baseline_state);
    assert_eq!(retention_class, baseline_retention_class);
}

#[tokio::test]
async fn execute_sync_with_stats_reports_batched_sqlite_writes() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    let (summary, stats) = crate::host::devql::execute_sync_with_stats(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute full sync with stats");

    assert!(summary.paths_added > 0);
    assert_eq!(stats.prepare_worker_count, summary.paths_added.min(8));
    assert!(stats.sqlite_commits > 0);
    assert!(
        stats.sqlite_commits < summary.paths_added.saturating_mul(2),
        "batched writer should use fewer commits than per-file cache+materialise writes"
    );
    assert!(
        !stats.workspace_inspection.is_zero(),
        "workspace inspection timing should be recorded"
    );
    assert!(
        !stats.desired_manifest_build.is_zero(),
        "manifest timing should be recorded"
    );
    assert!(
        stats.sqlite_rows_written > 0,
        "writer stats should record SQLite row mutations"
    );
}

#[tokio::test]
async fn sync_removes_deleted_file() {
    let repo = seed_full_sync_repo();
    let cfg = sync_test_cfg_for_repo(repo.path());
    let sqlite_path = repo.path().join("devql.sqlite");
    let relational = sqlite_relational_store_with_sync_schema(&sqlite_path).await;

    crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute initial full sync");

    fs::remove_file(repo.path().join("web/app.ts")).expect("delete tracked source file");

    let result = crate::host::devql::execute_sync(
        &cfg,
        &relational,
        crate::host::devql::sync::types::SyncMode::Full,
    )
    .await
    .expect("execute sync after delete");

    let db = Connection::open(&sqlite_path).expect("open sqlite db");
    let artefact_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "web/app.ts"],
            |row| row.get(0),
        )
        .expect("count artefacts for deleted path");
    let current_state_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "web/app.ts"],
            |row| row.get(0),
        )
        .expect("count current_file_state for deleted path");
    let edge_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artefact_edges_current WHERE repo_id = ?1 AND path = ?2",
            [cfg.repo.repo_id.as_str(), "web/app.ts"],
            |row| row.get(0),
        )
        .expect("count artefact_edges_current for deleted path");

    assert_eq!(result.paths_removed, 1);
    assert_eq!(result.paths_added, 0);
    assert_eq!(result.paths_changed, 0);
    assert_eq!(result.paths_unchanged, 3);
    assert_eq!(artefact_count, 0);
    assert_eq!(edge_count, 0);
    assert_eq!(current_state_count, 0);
}
