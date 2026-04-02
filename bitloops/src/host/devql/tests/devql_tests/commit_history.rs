use super::*;

fn write_local_devql_config(repo_root: &Path) {
    write_repo_daemon_config(
        repo_root,
        r#"[stores.relational]
sqlite_path = ".bitloops/stores/devql.sqlite"

[stores.events]
duckdb_path = ".bitloops/stores/events.duckdb"

[semantic]
provider = "disabled"
"#,
    );
}

fn cfg_for_repo(repo_root: &Path) -> DevqlConfig {
    let repo = resolve_repo_identity(repo_root).expect("resolve repo identity");
    DevqlConfig::from_env(repo_root.to_path_buf(), repo).expect("build devql cfg from repo")
}

fn sqlite_path_for_repo(repo_root: &Path) -> std::path::PathBuf {
    crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config")
        .relational
        .resolve_sqlite_db_path_for_repo(repo_root)
        .expect("resolve sqlite path")
}

fn duckdb_path_for_repo(repo_root: &Path) -> std::path::PathBuf {
    crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config")
        .events
        .resolve_duckdb_db_path_for_repo(repo_root)
}

fn sync_state_value(conn: &rusqlite::Connection, repo_id: &str, key: &str) -> Option<String> {
    use rusqlite::OptionalExtension;

    conn.query_row(
        "SELECT state_value FROM sync_state WHERE repo_id = ?1 AND state_key = ?2",
        rusqlite::params![repo_id, key],
        |row| row.get(0),
    )
    .optional()
    .expect("read sync_state value")
}

#[test]
fn select_missing_branch_commit_segment_prefers_branch_watermark_when_it_is_an_ancestor() {
    let repo = seed_git_repo();
    let sqlite_path = repo.path().join("relational.sqlite");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let relational = runtime.block_on(sqlite_relational_store_with_schema(&sqlite_path));
    let cfg = cfg_for_repo(repo.path());

    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(repo.path().join("src/lib.rs"), "pub fn one() -> i32 { 1 }\n")
        .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add lib"]);
    let first_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\n",
    )
    .expect("update lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "expand lib"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    runtime
        .block_on(relational.exec(&format!(
            "INSERT INTO sync_state (repo_id, state_key, state_value, updated_at) \
             VALUES ('{}', '{}', '{}', datetime('now'))",
            esc_pg(&cfg.repo.repo_id),
            esc_pg(&historical_branch_watermark_key("main")),
            esc_pg(&first_sha),
        )))
        .expect("seed historical watermark");

    let commits = runtime
        .block_on(select_missing_branch_commit_segment(
            repo.path(),
            &relational,
            &cfg.repo.repo_id,
            Some("main"),
            &head_sha,
        ))
        .expect("select commit range");

    assert_eq!(commits, vec![head_sha]);
}

#[test]
fn select_missing_branch_commit_segment_falls_back_to_nearest_reachable_completed_ledger_commit() {
    let repo = seed_git_repo();
    let sqlite_path = repo.path().join("relational.sqlite");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let relational = runtime.block_on(sqlite_relational_store_with_schema(&sqlite_path));
    let cfg = cfg_for_repo(repo.path());

    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(repo.path().join("src/lib.rs"), "pub fn one() -> i32 { 1 }\n")
        .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add lib"]);
    let first_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\n",
    )
    .expect("update lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "expand lib"]);
    let second_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\npub fn three() -> i32 { 3 }\n",
    )
    .expect("update lib.rs again");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "expand lib again"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    runtime
        .block_on(relational.exec(&format!(
            "INSERT INTO commit_ingest_ledger (
                repo_id, commit_sha, history_status, checkpoint_status, checkpoint_id, last_error, updated_at
            ) VALUES (
                '{}', '{}', 'completed', 'not_applicable', NULL, NULL, datetime('now')
            )",
            esc_pg(&cfg.repo.repo_id),
            esc_pg(&second_sha),
        )))
        .expect("seed completed ledger row");

    let commits = runtime
        .block_on(select_missing_branch_commit_segment(
            repo.path(),
            &relational,
            &cfg.repo.repo_id,
            Some("main"),
            &head_sha,
        ))
        .expect("select commit range");

    assert_eq!(commits, vec![head_sha]);
    assert_ne!(first_sha, second_sha);
}

#[tokio::test]
async fn execute_ingest_materialises_unmapped_commit_history_without_current_state_mutation() {
    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String { format!(\"hi {name}\") }\n",
    )
    .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add lib"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history unmapped test")
        .await
        .expect("initialise local devql store for unmapped commit history test");
    let summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute ingest for unmapped commits");
    assert!(
        summary.success,
        "ingest summary should report success for unmapped commit history"
    );
    assert_eq!(summary.commits_processed, 2);
    assert_eq!(summary.checkpoint_companions_processed, 0);

    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let sqlite = rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");

    let file_state_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count file_state rows");
    assert!(file_state_count > 0, "expected historical file_state rows");

    let artefact_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count historical artefacts");
    assert!(artefact_count > 0, "expected historical artefact rows");

    let current_artefact_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count current artefacts");
    assert_eq!(
        current_artefact_count, 0,
        "historical ingest must not mutate artefacts_current"
    );

    let current_file_state_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM current_file_state WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count current_file_state rows");
    assert_eq!(
        current_file_state_count, 0,
        "historical ingest must not mutate current_file_state"
    );

    let repo_sync_state_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM repo_sync_state WHERE repo_id = ?1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count repo_sync_state rows");
    assert_eq!(
        repo_sync_state_count, 0,
        "historical ingest must not mutate repo_sync_state"
    );

    let ledger_row: (String, String) = sqlite
        .query_row(
            "SELECT history_status, checkpoint_status
             FROM commit_ingest_ledger
             WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read commit ingest ledger row");
    assert_eq!(ledger_row.0, "completed");
    assert_eq!(ledger_row.1, "not_applicable");

    let watermark = sync_state_value(
        &sqlite,
        cfg.repo.repo_id.as_str(),
        &historical_branch_watermark_key("main"),
    )
    .expect("expected branch historical watermark");
    assert_eq!(watermark, head_sha);

    let checkpoint_projection_rows: i64 = sqlite
        .query_row("SELECT COUNT(*) FROM checkpoint_file_snapshots", [], |row| {
            row.get(0)
        })
        .expect("count checkpoint projections");
    assert_eq!(
        checkpoint_projection_rows, 0,
        "unmapped commits must not synthesize checkpoint projection rows"
    );

    let duckdb = duckdb::Connection::open(duckdb_path_for_repo(repo.path())).expect("open duckdb");
    let checkpoint_event_rows: i64 = duckdb
        .query_row("SELECT COUNT(*) FROM checkpoint_events", [], |row| row.get(0))
        .expect("count checkpoint events");
    assert_eq!(
        checkpoint_event_rows, 0,
        "unmapped commits must not synthesize checkpoint events"
    );
}

#[tokio::test]
async fn execute_ingest_runs_checkpoint_companion_work_once_for_mapped_commits() {
    use crate::host::checkpoints::strategy::manual_commit::{WriteCommittedOptions, write_committed};

    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "export const answer = () => 42;\n",
    )
    .expect("write lib.rs");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add mapped file"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history mapped test")
        .await
        .expect("initialise local devql store for mapped checkpoint test");

    let checkpoint_id = "aabbccddeeff";
    write_committed(
        repo.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "session-1".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "codex".to_string(),
            transcript: br#"{"checkpoint":true}"#.to_vec(),
            prompts: None,
            context: None,
            checkpoints_count: 1,
            files_touched: vec!["src/lib.rs".to_string()],
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
        },
    )
    .expect("write committed checkpoint");
    insert_commit_checkpoint_mapping(repo.path(), &head_sha, checkpoint_id);

    let first_summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("execute ingest for mapped commit");
    assert!(
        first_summary.commits_processed >= 1,
        "initial catch-up may include earlier reachable commits"
    );
    assert_eq!(first_summary.checkpoint_companions_processed, 1);
    let replay_summary = execute_ingest_with_observer(&cfg, false, 500, None, None)
        .await
        .expect("replay ingest for mapped commit");
    assert_eq!(replay_summary.commits_processed, 0);
    assert_eq!(replay_summary.checkpoint_companions_processed, 0);

    let sqlite = rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let checkpoint_projection_rows: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM checkpoint_file_snapshots WHERE checkpoint_id = ?1 AND commit_sha = ?2",
            rusqlite::params![checkpoint_id, head_sha.as_str()],
            |row| row.get(0),
        )
        .expect("count checkpoint projections");
    assert_eq!(checkpoint_projection_rows, 1);

    let ledger_row: (String, String) = sqlite
        .query_row(
            "SELECT history_status, checkpoint_status
             FROM commit_ingest_ledger
             WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read commit ingest ledger row");
    assert_eq!(ledger_row.0, "completed");
    assert_eq!(ledger_row.1, "completed");

    let duckdb = duckdb::Connection::open(duckdb_path_for_repo(repo.path())).expect("open duckdb");
    let checkpoint_event_rows: i64 = duckdb
        .query_row(
            "SELECT COUNT(*) FROM checkpoint_events WHERE checkpoint_id = ?",
            [checkpoint_id],
            |row| row.get(0),
        )
        .expect("count checkpoint events");
    assert_eq!(checkpoint_event_rows, 1);
}

#[tokio::test]
async fn execute_ingest_hidden_max_commits_cap_limits_commit_replay_without_public_api_changes() {
    use rusqlite::OptionalExtension;

    let repo = seed_git_repo();
    write_local_devql_config(repo.path());
    std::fs::create_dir_all(repo.path().join("src")).expect("create src");
    std::fs::write(repo.path().join("src/lib.rs"), "pub fn one() -> i32 { 1 }\n")
        .expect("write first revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add one"]);

    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn one() -> i32 { 1 }\npub fn two() -> i32 { 2 }\n",
    )
    .expect("write second revision");
    git_ok(repo.path(), &["add", "."]);
    git_ok(repo.path(), &["commit", "-m", "add two"]);
    let head_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);

    let cfg = cfg_for_repo(repo.path());
    execute_init_schema(&cfg, "commit-history hidden max commits test")
        .await
        .expect("initialise local devql store for hidden max commits test");

    let summary = execute_ingest_with_observer(&cfg, false, 1, None, None)
        .await
        .expect("execute ingest with hidden max commit cap");
    assert_eq!(summary.commits_processed, 1);

    let sqlite = rusqlite::Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let ingested_commit_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM commit_ingest_ledger WHERE repo_id = ?1 AND history_status = 'completed'",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get(0),
        )
        .expect("count completed ledger rows");
    let head_ledger: Option<String> = sqlite
        .query_row(
            "SELECT history_status FROM commit_ingest_ledger WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), head_sha.as_str()],
            |row| row.get(0),
        )
        .optional()
        .expect("read head ledger row");
    assert_eq!(ingested_commit_count, 1);
    assert!(head_ledger.is_none(), "head commit should remain pending");
}
