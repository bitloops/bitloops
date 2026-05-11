use super::*;

use crate::host::checkpoints::strategy::manual_commit::{WriteCommittedOptions, write_committed};
use crate::test_support::process_state::enter_process_state;
use tempfile::TempDir;

fn write_local_devql_config(repo_root: &Path) {
    let daemon_state_root = repo_root.join(".bitloops-test-state");
    let sqlite_path = daemon_state_root
        .join("stores")
        .join("relational")
        .join("devql.sqlite");
    let duckdb_path = daemon_state_root
        .join("stores")
        .join("event")
        .join("events.duckdb");
    write_repo_daemon_config(
        repo_root,
        format!(
            r#"[stores.relational]
sqlite_path = {sqlite_path:?}

[stores.events]
duckdb_path = {duckdb_path:?}
"#,
        ),
    );
}

fn test_cfg_for_repo(repo_root: &Path) -> DevqlConfig {
    let mut cfg = test_cfg();
    cfg.daemon_config_root = repo_root.to_path_buf();
    cfg.repo_root = repo_root.to_path_buf();
    cfg.repo = resolve_repo_identity(repo_root).expect("resolve repo identity");
    cfg
}

fn checkpoint_write_options(checkpoint_id: &str, files_touched: &[&str]) -> WriteCommittedOptions {
    WriteCommittedOptions {
        checkpoint_id: checkpoint_id.to_string(),
        session_id: "session-1".to_string(),
        strategy: "manual-commit".to_string(),
        agent: "codex".to_string(),
        transcript: br#"{"checkpoint": true}"#.to_vec(),
        prompts: None,
        context: None,
        checkpoints_count: 1,
        files_touched: files_touched
            .iter()
            .map(|path| (*path).to_string())
            .collect(),
        token_usage_input: None,
        token_usage_output: None,
        token_usage_api_call_count: None,
        turn_id: String::new(),
        transcript_identifier_at_start: String::new(),
        checkpoint_transcript_start: 0,
        token_usage: None,
        initial_attribution: None,
        author_name: "Bitloops Test".to_string(),
        author_email: "bitloops-test@example.com".to_string(),
        summary: None,
        is_task: false,
        tool_use_id: String::new(),
        agent_id: String::new(),
        transcript_path: String::new(),
        subagent_transcript_path: String::new(),
    }
}

fn projection_row_count(sqlite_path: &Path) -> i64 {
    let conn = rusqlite::Connection::open(sqlite_path).expect("open sqlite");
    conn.query_row("SELECT COUNT(*) FROM checkpoint_files", [], |row| {
        row.get(0)
    })
    .expect("count projection rows")
}

#[tokio::test]
async fn checkpoint_file_snapshot_backfill_noops_for_repo_without_committed_checkpoints() {
    let repo = seed_git_repo();
    let home = TempDir::new().expect("home dir");
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[
            ("HOME", Some(home_path.as_str())),
            ("USERPROFILE", Some(home_path.as_str())),
        ],
    );
    write_local_devql_config(repo.path());

    let cfg = test_cfg_for_repo(repo.path());
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;
    let summary = execute_checkpoint_file_snapshot_backfill_with_relational(
        &cfg,
        &relational,
        &CheckpointFileSnapshotBackfillOptions::default(),
    )
    .await
    .expect("backfill should succeed");

    assert!(summary.success);
    assert_eq!(summary.checkpoints_scanned, 0);
    assert_eq!(summary.checkpoints_processed, 0);
    assert_eq!(summary.rows_projected, 0);
    assert_eq!(summary.unresolved_files, 0);
    assert_eq!(projection_row_count(&sqlite_path), 0);
}

#[tokio::test]
async fn checkpoint_file_snapshot_backfill_is_rerunnable_for_checkpoint_file_rows() {
    let repo = seed_git_repo();
    let home = TempDir::new().expect("home dir");
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[
            ("HOME", Some(home_path.as_str())),
            ("USERPROFILE", Some(home_path.as_str())),
        ],
    );
    write_local_devql_config(repo.path());

    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn projection_backfill() -> &'static str {\n    \"ok\"\n}\n",
    )
    .expect("write source file");
    git_ok(repo.path(), &["add", "src/lib.rs"]);
    git_ok(repo.path(), &["commit", "-m", "add projection source"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let checkpoint_id = "a1b2c3d4e5f6";
    let file_path = "src/lib.rs";
    let cfg = test_cfg_for_repo(repo.path());
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    write_committed(
        repo.path(),
        checkpoint_write_options(checkpoint_id, &[file_path]),
    )
    .expect("write committed checkpoint");
    insert_commit_checkpoint_mapping(repo.path(), &head_sha, checkpoint_id);

    let first = execute_checkpoint_file_snapshot_backfill_with_relational(
        &cfg,
        &relational,
        &CheckpointFileSnapshotBackfillOptions::default(),
    )
    .await
    .expect("initial backfill should succeed");
    assert!(first.success);
    assert_eq!(first.checkpoints_scanned, 1);
    assert_eq!(first.checkpoints_processed, 1);
    assert_eq!(first.rows_projected, 0);
    assert_eq!(first.rows_already_present, 1);
    assert_eq!(first.unresolved_files, 0);
    assert_eq!(projection_row_count(&sqlite_path), 1);

    let second = execute_checkpoint_file_snapshot_backfill_with_relational(
        &cfg,
        &relational,
        &CheckpointFileSnapshotBackfillOptions::default(),
    )
    .await
    .expect("rerun should succeed");
    assert!(second.success);
    assert_eq!(second.checkpoints_scanned, 1);
    assert_eq!(second.rows_projected, 0);
    assert_eq!(second.rows_already_present, 1);
    assert_eq!(second.unresolved_files, 0);
    assert_eq!(projection_row_count(&sqlite_path), 1);
}

#[tokio::test]
async fn checkpoint_file_snapshot_backfill_deletes_stale_rows_when_checkpoint_is_fully_resolved() {
    let repo = seed_git_repo();
    let home = TempDir::new().expect("home dir");
    let home_path = home.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[
            ("HOME", Some(home_path.as_str())),
            ("USERPROFILE", Some(home_path.as_str())),
        ],
    );
    write_local_devql_config(repo.path());

    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::write(
        repo.path().join("src/repair.rs"),
        "pub fn repair_projection() -> i32 {\n    7\n}\n",
    )
    .expect("write source file");
    git_ok(repo.path(), &["add", "src/repair.rs"]);
    git_ok(repo.path(), &["commit", "-m", "add repair source"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let checkpoint_id = "f6e5d4c3b2a1";
    let file_path = "src/repair.rs";
    let cfg = test_cfg_for_repo(repo.path());
    let sqlite_path = checkpoint_sqlite_path(repo.path());
    let relational = sqlite_relational_store_with_schema(&sqlite_path).await;

    write_committed(
        repo.path(),
        checkpoint_write_options(checkpoint_id, &[file_path]),
    )
    .expect("write committed checkpoint");
    insert_commit_checkpoint_mapping(repo.path(), &head_sha, checkpoint_id);

    let blob_sha =
        git_blob_sha_at_commit(repo.path(), &head_sha, file_path).expect("resolve blob sha");

    {
        let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
        conn.execute(
            "INSERT INTO checkpoint_files (
                relation_id, repo_id, checkpoint_id, session_id, event_time, agent, branch, strategy,
                commit_sha, change_kind, path_before, path_after, blob_sha_before, blob_sha_after
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'modify', ?10, ?10, ?11, ?11)",
            rusqlite::params![
                "stale-relation",
                cfg.repo.repo_id.as_str(),
                checkpoint_id,
                "stale-session",
                "2026-03-27T00:00:00Z",
                "codex",
                "main",
                "manual-commit",
                head_sha.as_str(),
                file_path,
                "stale-blob"
            ],
        )
        .expect("insert stale checkpoint_files row");
    }

    let summary = execute_checkpoint_file_snapshot_backfill_with_relational(
        &cfg,
        &relational,
        &CheckpointFileSnapshotBackfillOptions::default(),
    )
    .await
    .expect("repair backfill should succeed");

    assert!(summary.success);
    assert_eq!(summary.checkpoints_scanned, 1);
    assert_eq!(summary.rows_projected, 0);
    assert_eq!(summary.rows_already_present, 1);
    assert_eq!(summary.stale_rows_detected, 1);
    assert_eq!(summary.stale_rows_deleted, 1);
    assert_eq!(summary.unresolved_files, 0);

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
    let blobs = conn
        .prepare(
            "SELECT blob_sha_after FROM checkpoint_files
             WHERE repo_id = ?1 AND checkpoint_id = ?2
             ORDER BY blob_sha_after ASC",
        )
        .expect("prepare blob query")
        .query_map(
            rusqlite::params![cfg.repo.repo_id.as_str(), checkpoint_id],
            |row| row.get::<_, String>(0),
        )
        .expect("query projection blobs")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect projection blobs");
    assert_eq!(blobs, vec![blob_sha]);
}
