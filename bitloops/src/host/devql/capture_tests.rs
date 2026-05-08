use std::fs;

use super::*;
use crate::host::devql::file_symbol_id;
use crate::test_support::git_fixtures::{git_ok, init_test_repo, write_test_daemon_config};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn deterministic_uuid(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = hex::encode(hasher.finalize());
    let hex = &digest[..32];
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn seed_repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    init_test_repo(
        dir.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    write_test_daemon_config(dir.path());
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"watch-capture-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn first() -> i32 {\n    1\n}\n",
    )
    .expect("write initial file");
    git_ok(dir.path(), &["add", "."]);
    git_ok(dir.path(), &["commit", "-m", "initial"]);
    dir
}

fn devql_sqlite_path(repo_root: &std::path::Path) -> std::path::PathBuf {
    let backend = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config");
    let sqlite_path = backend
        .relational
        .sqlite_path
        .as_deref()
        .expect("test daemon config should set sqlite_path");
    crate::config::resolve_sqlite_db_path_for_repo(repo_root, Some(sqlite_path))
        .expect("resolve configured sqlite path")
}

fn workspace_revision_count(repo_root: &std::path::Path, repo_id: &str) -> i64 {
    let db_path = devql_sqlite_path(repo_root);
    if !db_path.exists() {
        return 0;
    }

    let conn = Connection::open(db_path).expect("open sqlite");
    let table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'workspace_revisions'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if !table_exists {
        return 0;
    }

    conn.query_row(
        "SELECT COUNT(*) FROM workspace_revisions WHERE repo_id = ?1",
        [repo_id],
        |row| row.get(0),
    )
    .expect("count workspace_revisions")
}

#[test]
fn capture_updates_current_devql_state_for_modified_file() {
    let dir = seed_repo();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn second() -> i32 {\n    2\n}\n",
    )
    .expect("update file");
    let working_blob_sha = git_ok(dir.path(), &["hash-object", "src/lib.rs"]);

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    capture_temporary_checkpoint_batch(&cfg, &[dir.path().join("src/lib.rs")])
        .expect("capture temporary checkpoint");

    let db_path = devql_sqlite_path(dir.path());
    let conn = Connection::open(db_path).expect("open sqlite");

    let workspace_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_revisions WHERE repo_id = ?1",
            [&cfg.repo.repo_id],
            |row| row.get(0),
        )
        .expect("count workspace_revisions");
    assert_eq!(workspace_rows, 1);

    let current_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = 'src/lib.rs' AND symbol_id = ?1",
            [file_symbol_id("src/lib.rs")],
            |row| row.get(0),
        )
        .expect("count current file rows");
    assert_eq!(current_rows, 1);

    let artefact_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = 'src/lib.rs'",
            [],
            |row| row.get(0),
        )
        .expect("count current artefact rows");
    assert!(artefact_rows >= 2, "expected file + function artefacts");

    let materialized_rows: Vec<(String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT symbol_fqn, content_id, language \
                 FROM artefacts_current \
                 WHERE path = 'src/lib.rs' \
                 ORDER BY symbol_fqn",
            )
            .expect("prepare sync-shaped artefacts_current query");
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("query sync-shaped artefacts_current rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect sync-shaped artefacts_current rows")
    };
    assert!(
        materialized_rows
            .iter()
            .all(|(_, content_id, language)| content_id == &working_blob_sha && language == "rust"),
        "watch capture should delegate to sync and materialize rows for the edited blob"
    );
    assert!(
        materialized_rows
            .iter()
            .any(|(symbol_fqn, _, _)| symbol_fqn == "src/lib.rs"),
        "file row should be materialized with the sync-shaped schema"
    );

    let current_state: (String, String) = conn
        .query_row(
            "SELECT effective_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [&cfg.repo.repo_id, "src/lib.rs"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current file state row");
    assert_eq!(current_state.0, working_blob_sha);
    assert_eq!(current_state.1, "worktree");
}

#[test]
fn capture_ignores_directory_events_and_still_updates_file_state() {
    let dir = seed_repo();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn third() -> i32 {\n    3\n}\n",
    )
    .expect("update file");

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    capture_temporary_checkpoint_batch(
        &cfg,
        &[dir.path().join("src"), dir.path().join("src/lib.rs")],
    )
    .expect("capture with mixed dir and file events");

    let db_path = devql_sqlite_path(dir.path());
    let conn = Connection::open(db_path).expect("open sqlite");
    let current_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = 'src/lib.rs' AND symbol_id = ?1",
            [file_symbol_id("src/lib.rs")],
            |row| row.get(0),
        )
        .expect("count current file rows");
    assert_eq!(current_rows, 1);
}

#[test]
fn capture_rewrites_stale_path_metadata_even_when_blob_is_unchanged() {
    let dir = seed_repo();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn second() -> i32 {\n    2\n}\n",
    )
    .expect("update file");
    let working_blob_sha = git_ok(dir.path(), &["hash-object", "src/lib.rs"]);

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    let db_path = devql_sqlite_path(dir.path());
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(db_path.clone()).expect("connect sqlite");
    sqlite
        .initialise_devql_schema()
        .expect("initialise devql schema");

    let committed_unix = git_ok(dir.path(), &["show", "-s", "--format=%ct", "HEAD"])
        .parse::<i64>()
        .expect("parse commit unix timestamp");
    let file_symbol = file_symbol_id("src/lib.rs");
    let file_artefact_id = deterministic_uuid(&format!(
        "{}|{}|{}",
        cfg.repo.repo_id, working_blob_sha, file_symbol
    ));

    sqlite
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
                 VALUES (?1, ?2, ?3, ?4, 'main') \
                 ON CONFLICT(repo_id) DO UPDATE SET \
                   provider = excluded.provider, \
                   organization = excluded.organization, \
                   name = excluded.name, \
                   default_branch = excluded.default_branch",
                rusqlite::params![
                    cfg.repo.repo_id.as_str(),
                    cfg.repo.provider.as_str(),
                    cfg.repo.organization.as_str(),
                    cfg.repo.name.as_str(),
                ],
            )?;
            conn.execute(
                "INSERT INTO artefacts_current (
                    repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind,
                    language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line,
                    end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at
                ) VALUES (?1, 'src/lib.rs', ?2, ?3, ?4, 'rust', 'file', 'file', 'src/lib.rs::stale',
                    NULL, NULL, 1, 3, 0, 29, NULL, '[]', NULL, datetime(?5, 'unixepoch'))",
                rusqlite::params![
                    cfg.repo.repo_id,
                    working_blob_sha,
                    file_symbol,
                    file_artefact_id,
                    committed_unix,
                ],
            )?;
            Ok(())
        })
        .expect("seed stale path metadata");

    capture_temporary_checkpoint_batch(&cfg, &[dir.path().join("src/lib.rs")])
        .expect("capture temporary checkpoint over unchanged blob");

    let conn = Connection::open(db_path).expect("open sqlite");
    let materialized_rows: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT symbol_fqn, content_id \
                 FROM artefacts_current \
                 WHERE path = 'src/lib.rs' \
                 ORDER BY symbol_fqn",
            )
            .expect("prepare sync-shaped artefacts_current query");
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query sync-shaped artefacts_current rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect sync-shaped artefacts_current rows")
    };
    assert!(
        materialized_rows
            .iter()
            .all(
                |(symbol_fqn, content_id)| symbol_fqn.starts_with("src/lib.rs")
                    && content_id == &working_blob_sha
            ),
        "capture should rewrite stale sync rows for the edited path"
    );
    assert!(
        materialized_rows
            .iter()
            .any(|(symbol_fqn, _)| symbol_fqn == "src/lib.rs"),
        "file row should be re-materialized using the sync-shaped schema"
    );
    assert!(
        !materialized_rows
            .iter()
            .any(|(symbol_fqn, _)| symbol_fqn == "src/lib.rs::stale"),
        "stale path metadata should not survive capture"
    );

    let current_state: (String, String) = conn
        .query_row(
            "SELECT effective_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [&cfg.repo.repo_id, "src/lib.rs"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current file state row");
    assert_eq!(current_state.0, working_blob_sha);
    assert_eq!(current_state.1, "worktree");
}

#[test]
fn capture_only_revises_affected_symbols_for_follow_up_temp_changes() {
    let dir = seed_repo();
    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    let target = dir.path().join("src/lib.rs");

    fs::write(
        &target,
        "pub fn first() -> i32 {\n    10\n}\n\npub fn second() -> i32 {\n    2\n}\n",
    )
    .expect("write first temp version");
    capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("capture first temp batch");

    fs::write(
        &target,
        "pub fn first() -> i32 {\n    11\n}\n\npub fn second() -> i32 {\n    2\n}\n",
    )
    .expect("write second temp version");
    let current_blob_sha = git_ok(dir.path(), &["hash-object", "src/lib.rs"]);
    capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("capture second temp batch");

    let db_path = devql_sqlite_path(dir.path());
    let conn = Connection::open(db_path).expect("open sqlite");
    let materialized_rows: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT symbol_fqn, content_id \
                 FROM artefacts_current \
                 WHERE path = 'src/lib.rs' \
                 ORDER BY symbol_fqn",
            )
            .expect("prepare sync-shaped artefacts_current query");
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query sync-shaped artefacts_current rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect sync-shaped artefacts_current rows")
    };

    assert!(
        materialized_rows
            .iter()
            .all(
                |(symbol_fqn, content_id)| symbol_fqn.starts_with("src/lib.rs")
                    && content_id == &current_blob_sha
            ),
        "follow-up temp changes should rematerialize the sync-shaped rows for the edited blob"
    );
    assert!(
        materialized_rows
            .iter()
            .any(|(symbol_fqn, _)| symbol_fqn == "src/lib.rs::first"),
        "first symbol should be materialized"
    );
    assert!(
        materialized_rows
            .iter()
            .any(|(symbol_fqn, _)| symbol_fqn == "src/lib.rs::second"),
        "second symbol should be materialized"
    );
}

#[test]
fn capture_skips_no_content_change_events() {
    let dir = seed_repo();

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    capture_temporary_checkpoint_batch(&cfg, &[dir.path().join("src/lib.rs")])
        .expect("capture no-op batch");

    let db_path = devql_sqlite_path(dir.path());
    if !db_path.exists() {
        return;
    }
    let conn = Connection::open(db_path).expect("open sqlite");
    let workspace_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_revisions WHERE repo_id = ?1",
            [&cfg.repo.repo_id],
            |row| row.get(0),
        )
        .expect("count workspace_revisions");
    assert_eq!(
        workspace_rows, 0,
        "no-content-change capture should not persist workspace_revisions rows"
    );
}

#[test]
fn capture_skips_duplicate_tree_hash_batches() {
    let dir = seed_repo();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn second() -> i32 {\n    2\n}\n",
    )
    .expect("update file");

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    let target = dir.path().join("src/lib.rs");
    capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("capture first batch");
    capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("capture duplicate batch");

    let db_path = devql_sqlite_path(dir.path());
    let conn = Connection::open(db_path).expect("open sqlite");
    let workspace_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_revisions WHERE repo_id = ?1",
            [&cfg.repo.repo_id],
            |row| row.get(0),
        )
        .expect("count workspace_revisions");
    assert_eq!(
        workspace_rows, 1,
        "duplicate tree hash should be deduplicated"
    );
}

#[test]
fn capture_prepares_batch_when_delete_restores_head_tree_after_dirty_revision() {
    let dir = seed_repo();
    let target = dir.path().join("src/dirty_reset.rs");
    fs::write(&target, "pub fn dirty_reset() {}\n").expect("write dirty source");

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("capture dirty source");

    fs::remove_file(&target).expect("delete dirty source");
    let prepared = prepare_capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("prepare delete capture");

    let prepared = prepared.expect("delete returning to HEAD should still produce a batch");
    assert_eq!(prepared.modified, Vec::<String>::new());
    assert_eq!(prepared.deleted, vec!["src/dirty_reset.rs".to_string()]);
}

#[test]
fn capture_prepares_batch_when_clean_removes_untracked_dirty_revision() {
    let dir = seed_repo();
    let target = dir.path().join("src/untracked_clean.rs");
    fs::write(&target, "pub fn untracked_clean() {}\n").expect("write untracked source");

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("capture untracked source");

    fs::remove_file(&target).expect("delete untracked source");
    let prepared = prepare_capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("prepare clean capture");

    let prepared = prepared.expect("clean returning to HEAD should still produce a batch");
    assert_eq!(prepared.modified, Vec::<String>::new());
    assert_eq!(prepared.deleted, vec!["src/untracked_clean.rs".to_string()]);
}

#[test]
fn capture_marks_revisited_tree_hash_as_latest_after_full_capture() {
    let dir = seed_repo();
    let target = dir.path().join("src/revisit.rs");
    let content = "pub fn revisit() {}\n";
    fs::write(&target, content).expect("write revisit source");

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("capture first revisit source");

    fs::remove_file(&target).expect("delete revisit source");
    capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("capture revisit cleanup");

    fs::write(&target, content).expect("recreate revisit source");
    capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("capture revisited source");

    let prepared = prepare_capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("prepare duplicate revisited source");
    assert!(
        prepared.is_none(),
        "revisited tree hash should become latest and dedupe immediate duplicate events"
    );
}

#[test]
fn prepare_noop_head_tree_does_not_create_sqlite_store() {
    let dir = seed_repo();
    let db_path = devql_sqlite_path(dir.path());
    assert!(
        !db_path.exists(),
        "seeded clean repo should not start with a DevQL SQLite store"
    );

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    let prepared =
        prepare_capture_temporary_checkpoint_batch(&cfg, &[dir.path().join("src/lib.rs")])
            .expect("prepare clean no-op capture");

    assert!(
        prepared.is_none(),
        "unchanged HEAD file should not produce a batch"
    );
    assert!(
        !db_path.exists(),
        "clean no-op prepare should not create the DevQL SQLite store"
    );
}

#[test]
fn prepare_capture_batch_does_not_consume_tree_hash_before_sync_handoff_succeeds() {
    let dir = seed_repo();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn second() -> i32 {\n    2\n}\n",
    )
    .expect("update file");

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    let target = dir.path().join("src/lib.rs");

    let first = prepare_capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("prepare first capture batch");
    assert!(
        first.is_some(),
        "updated file should produce a capture batch before handoff"
    );

    let db_path = devql_sqlite_path(dir.path());
    let conn = Connection::open(db_path).expect("open sqlite");
    let workspace_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_revisions WHERE repo_id = ?1",
            [&cfg.repo.repo_id],
            |row| row.get(0),
        )
        .expect("count workspace_revisions after prepare-only step");
    assert_eq!(
        workspace_rows, 0,
        "prepare-only capture must not persist workspace_revisions before sync handoff succeeds"
    );

    let second = prepare_capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("prepare retry capture batch");
    assert!(
        second.is_some(),
        "retry should still produce a capture batch when the prior handoff never succeeded"
    );
}

#[test]
fn capture_sync_disabled_does_not_persist_workspace_revision() {
    let dir = seed_repo();
    crate::config::settings::set_devql_producer_settings(
        &dir.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        false,
        true,
    )
    .expect("disable producer sync");
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn sync_disabled() -> i32 {\n    2\n}\n",
    )
    .expect("update file");

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    let target = dir.path().join("src/lib.rs");
    capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("capture temporary checkpoint with sync disabled");

    assert_eq!(
        workspace_revision_count(dir.path(), &cfg.repo.repo_id),
        0,
        "policy-skipped sync must not persist workspace_revisions rows"
    );

    crate::config::settings::set_devql_producer_settings(
        &dir.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        true,
        true,
    )
    .expect("enable producer sync");
    let prepared = prepare_capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("prepare capture after re-enabling sync");
    assert!(
        prepared.is_some(),
        "same tree hash must remain eligible after policy-skipped sync"
    );

    capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
        .expect("capture temporary checkpoint after re-enabling sync");
    assert_eq!(
        workspace_revision_count(dir.path(), &cfg.repo.repo_id),
        1,
        "completed sync should persist one workspace_revisions row"
    );
}

#[test]
fn capture_with_handle_updates_and_deletes_current_file_row() {
    let dir = seed_repo();
    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("create test runtime");
    let target = dir.path().join("src/lib.rs");

    fs::write(&target, "pub fn second() -> i32 {\n    2\n}\n").expect("update file");
    capture_temporary_checkpoint_batch_with_handle(
        &cfg,
        std::slice::from_ref(&target),
        runtime.handle(),
    )
    .expect("capture updated file using runtime handle");

    fs::remove_file(&target).expect("delete file");
    capture_temporary_checkpoint_batch_with_handle(
        &cfg,
        std::slice::from_ref(&target),
        runtime.handle(),
    )
    .expect("capture deleted file using runtime handle");

    let db_path = devql_sqlite_path(dir.path());
    let conn = Connection::open(db_path).expect("open sqlite");
    let workspace_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_revisions WHERE repo_id = ?1",
            [&cfg.repo.repo_id],
            |row| row.get(0),
        )
        .expect("count workspace_revisions");
    assert_eq!(
        workspace_rows, 2,
        "update and delete should produce two workspace_revisions rows"
    );

    let current_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE path = 'src/lib.rs' AND symbol_id = ?1",
            [file_symbol_id("src/lib.rs")],
            |row| row.get(0),
        )
        .expect("count current file rows");
    assert_eq!(
        current_rows, 0,
        "deleted file should be removed from current file row"
    );
}

#[test]
fn capture_with_handle_skips_duplicate_tree_hash_batches() {
    let dir = seed_repo();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn second() -> i32 {\n    2\n}\n",
    )
    .expect("update file");

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("create test runtime");
    let target = dir.path().join("src/lib.rs");
    capture_temporary_checkpoint_batch_with_handle(
        &cfg,
        std::slice::from_ref(&target),
        runtime.handle(),
    )
    .expect("capture first batch with runtime handle");
    capture_temporary_checkpoint_batch_with_handle(
        &cfg,
        std::slice::from_ref(&target),
        runtime.handle(),
    )
    .expect("capture duplicate batch with runtime handle");

    let db_path = devql_sqlite_path(dir.path());
    let conn = Connection::open(db_path).expect("open sqlite");
    let workspace_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_revisions WHERE repo_id = ?1",
            [&cfg.repo.repo_id],
            |row| row.get(0),
        )
        .expect("count workspace_revisions");
    assert_eq!(
        workspace_rows, 1,
        "duplicate tree hash should be deduplicated for handle capture path"
    );
}

#[test]
fn capture_workspace_revision_batches_delegate_to_sync_and_dedupe_tree_hashes() {
    // Verify that watcher capture delegates to sync-shaped materialization
    // and that workspace_revisions still dedupe identical tree hashes.
    let dir = seed_repo();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn linked() -> i32 {\n    7\n}\n",
    )
    .expect("update file");

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    capture_temporary_checkpoint_batch(&cfg, &[dir.path().join("src/lib.rs")])
        .expect("capture batch");

    let db_path = devql_sqlite_path(dir.path());
    let conn = Connection::open(db_path).expect("open sqlite");

    let workspace_id: i64 = conn
        .query_row(
            "SELECT id FROM workspace_revisions WHERE repo_id = ?1 ORDER BY id DESC LIMIT 1",
            [&cfg.repo.repo_id],
            |row| row.get(0),
        )
        .expect("fetch workspace_revisions id");

    let workspace_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_revisions WHERE repo_id = ?1",
            [&cfg.repo.repo_id],
            |row| row.get(0),
        )
        .expect("count workspace_revisions");
    let artefact_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            [&cfg.repo.repo_id, "src/lib.rs"],
            |row| row.get(0),
        )
        .expect("count sync-shaped artefacts_current rows");
    let current_state: (String, String) = conn
        .query_row(
            "SELECT effective_content_id, effective_source \
             FROM current_file_state \
             WHERE repo_id = ?1 AND path = ?2",
            [&cfg.repo.repo_id, "src/lib.rs"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read current file state row");
    let content_id: String = conn
        .query_row(
            "SELECT content_id FROM artefacts_current \
             WHERE path = 'src/lib.rs' AND symbol_id = ?1",
            [file_symbol_id("src/lib.rs")],
            |row| row.get(0),
        )
        .expect("fetch file content_id");
    assert_eq!(
        workspace_rows, 1,
        "workspace_revisions should contain exactly one deduped tree hash batch"
    );
    assert!(
        artefact_rows >= 1,
        "watcher capture should delegate to sync and materialize sync-shaped rows"
    );
    assert_eq!(
        current_state.1, "worktree",
        "captured file should be tracked via sync current_file_state"
    );
    assert_eq!(
        current_state.0, content_id,
        "current_file_state should use the same effective content id as artefacts_current"
    );
    assert!(
        workspace_id > 0,
        "workspace_revisions must insert a batch id"
    );
}

#[test]
fn capture_does_not_write_to_temporary_checkpoints_table() {
    // Ensure the watcher no longer touches the checkpoint schema at all.
    let dir = seed_repo();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn no_checkpoint() -> i32 {\n    0\n}\n",
    )
    .expect("update file");

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    capture_temporary_checkpoint_batch(&cfg, &[dir.path().join("src/lib.rs")])
        .expect("capture batch");

    let db_path = devql_sqlite_path(dir.path());
    let conn = Connection::open(db_path).expect("open sqlite");

    // temporary_checkpoints should not even exist in the DevQL database —
    // it lives in the separate checkpoint database.
    let table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'temporary_checkpoints'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    assert!(
        !table_exists,
        "temporary_checkpoints must NOT exist in the DevQL relational database"
    );
}

#[test]
fn capture_workspace_revisions_table_exists_after_first_capture() {
    let dir = seed_repo();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn after_first_capture() -> i32 {\n    1\n}\n",
    )
    .expect("update file");

    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
    let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
        .expect("build devql config");
    capture_temporary_checkpoint_batch(&cfg, &[dir.path().join("src/lib.rs")])
        .expect("capture batch");

    let db_path = devql_sqlite_path(dir.path());
    let conn = Connection::open(db_path).expect("open sqlite");

    let table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'workspace_revisions'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    assert!(
        table_exists,
        "workspace_revisions table must exist after capture"
    );
}
