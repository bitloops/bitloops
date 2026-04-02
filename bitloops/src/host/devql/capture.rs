use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

#[cfg(test)]
pub(crate) fn capture_temporary_checkpoint_batch(
    cfg: &crate::host::devql::DevqlConfig,
    changed_paths: &[PathBuf],
) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("creating watcher capture runtime")?;
    capture_temporary_checkpoint_batch_with_handle(cfg, changed_paths, runtime.handle())
}

pub(crate) fn capture_temporary_checkpoint_batch_with_handle(
    cfg: &crate::host::devql::DevqlConfig,
    changed_paths: &[PathBuf],
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    if changed_paths.is_empty() {
        return Ok(());
    }

    let repo_root = &cfg.repo_root;
    let base_commit = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo_root,
        &["rev-parse", "HEAD"],
    )
    .unwrap_or_default();

    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    for path in changed_paths {
        let rel = path.strip_prefix(repo_root).unwrap_or(path.as_path());
        let rel = rel.to_string_lossy().to_string();
        if rel.is_empty() {
            continue;
        }
        if path.exists() {
            if path.is_file() {
                modified.push(rel);
            }
        } else {
            deleted.push(rel);
        }
    }

    if modified.is_empty() && deleted.is_empty() {
        return Ok(());
    }

    let parent_tree = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo_root,
        &["rev-parse", &format!("{}^{{tree}}", base_commit)],
    )
    .ok();

    let tree_hash = crate::host::checkpoints::strategy::manual_commit::build_tree(
        repo_root,
        parent_tree.as_deref(),
        &modified,
        &[],
        &deleted,
    )
    .context("building temporary checkpoint tree hash for devql watch")?;
    if parent_tree.as_deref() == Some(tree_hash.as_str()) {
        return Ok(());
    }

    let backend_cfg = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .context("resolving store config for watcher capture")?;
    let sqlite_path = crate::config::resolve_sqlite_db_path_for_repo(
        repo_root,
        backend_cfg.relational.sqlite_path.as_deref(),
    )
    .context("resolving SQLite path for watcher capture")?;
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.clone())?;
    sqlite.initialise_devql_schema()?;

    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for watch capture")?
        .repo_id;

    let latest_tree_hash = sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT tree_hash FROM workspace_revisions WHERE repo_id = ?1 ORDER BY id DESC LIMIT 1",
            rusqlite::params![&repo_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(anyhow::Error::from)
    })?;
    if latest_tree_hash.as_deref() == Some(tree_hash.as_str()) {
        return Ok(());
    }

    let _row_id = sqlite.with_connection(|conn| {
        conn.execute(
            "INSERT OR IGNORE INTO workspace_revisions (repo_id, tree_hash) VALUES (?1, ?2)",
            rusqlite::params![repo_id, tree_hash],
        )?;
        conn.query_row(
            "SELECT id FROM workspace_revisions WHERE repo_id = ?1 AND tree_hash = ?2",
            rusqlite::params![repo_id, tree_hash],
            |row| row.get::<_, i64>(0),
        )
        .map_err(anyhow::Error::from)
    })?;

    handle.block_on(sync_changed_paths(cfg, &modified, &deleted))?;

    Ok(())
}

async fn sync_changed_paths(
    cfg: &crate::host::devql::DevqlConfig,
    modified: &[String],
    deleted: &[String],
) -> Result<()> {
    let mut paths = modified
        .iter()
        .chain(deleted.iter())
        .map(|path| crate::host::devql::normalize_repo_path(path))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Ok(());
    }

    #[cfg(test)]
    {
        crate::host::devql::run_sync_with_summary(cfg, crate::host::devql::SyncMode::Paths(paths))
            .await
            .context("running DevQL sync inline for watcher capture paths in tests")?;
        Ok(())
    }

    #[cfg(not(test))]
    {
        crate::daemon::enqueue_sync_for_config(
            cfg,
            crate::daemon::SyncTaskSource::Watcher,
            crate::host::devql::SyncMode::Paths(paths),
        )
        .context("queueing DevQL sync for watcher capture paths")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::host::devql::file_symbol_id;
    use crate::test_support::git_fixtures::{git_ok, init_test_repo};
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

        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
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
                .all(|(_, content_id, language)| content_id == &working_blob_sha
                    && language == "rust"),
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

        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
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
        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
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

        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
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

        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
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

        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
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
    fn capture_with_handle_updates_and_deletes_current_file_row() {
        let dir = seed_repo();
        let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::host::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
            .expect("build devql config");
        let runtime = tokio::runtime::Runtime::new().expect("create test runtime");
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

        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
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
        let runtime = tokio::runtime::Runtime::new().expect("create test runtime");
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

        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
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

        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
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

        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
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

        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
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
}
