use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use tokio::task;

#[cfg(test)]
pub(crate) fn capture_temporary_checkpoint_batch(
    cfg: &crate::engine::devql::DevqlConfig,
    changed_paths: &[PathBuf],
) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("creating watcher capture runtime")?;
    capture_temporary_checkpoint_batch_with_handle(cfg, changed_paths, runtime.handle())
}

pub(crate) fn capture_temporary_checkpoint_batch_with_handle(
    cfg: &crate::engine::devql::DevqlConfig,
    changed_paths: &[PathBuf],
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    if changed_paths.is_empty() {
        return Ok(());
    }

    let repo_root = &cfg.repo_root;
    let base_commit =
        crate::engine::strategy::manual_commit::run_git(repo_root, &["rev-parse", "HEAD"])
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

    let parent_tree = crate::engine::strategy::manual_commit::run_git(
        repo_root,
        &["rev-parse", &format!("{}^{{tree}}", base_commit)],
    )
    .ok();

    let tree_hash = crate::engine::strategy::manual_commit::build_tree(
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
    let sqlite = crate::engine::db::SqliteConnectionPool::connect(sqlite_path.clone())?;
    sqlite.initialise_devql_schema()?;

    let repo_id = crate::engine::devql::resolve_repo_identity(repo_root)
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

    let row_id = sqlite.with_connection(|conn| {
        conn.execute(
            "INSERT INTO workspace_revisions (repo_id, tree_hash) VALUES (?1, ?2)",
            rusqlite::params![repo_id, tree_hash],
        )?;
        Ok(conn.last_insert_rowid())
    })?;

    let revision_unix = current_unix_timestamp();
    let relational = crate::engine::devql::RelationalStorage::Sqlite { path: sqlite_path };
    handle.block_on(apply_current_state_updates(
        cfg,
        &relational,
        repo_root,
        &base_commit,
        &tree_hash,
        row_id,
        revision_unix,
        &modified,
        &deleted,
    ))?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn apply_current_state_updates(
    cfg: &crate::engine::devql::DevqlConfig,
    relational: &crate::engine::devql::RelationalStorage,
    repo_root: &Path,
    base_commit: &str,
    tree_hash: &str,
    row_id: i64,
    revision_unix: i64,
    modified: &[String],
    deleted: &[String],
) -> Result<()> {
    let revision_id = format!("temp:{row_id}");
    for rel_path in modified {
        let content = match load_file_from_tree_blocking(repo_root, tree_hash, rel_path).await {
            Ok(value) => value,
            Err(err) => {
                log::warn!("devql watcher skipped `{rel_path}`: {err:#}");
                continue;
            }
        };
        let blob_sha = match load_blob_sha_from_tree_blocking(repo_root, tree_hash, rel_path).await
        {
            Ok(value) => value,
            Err(err) => {
                log::warn!("devql watcher skipped `{rel_path}` blob lookup: {err:#}");
                continue;
            }
        };
        let revision = crate::engine::devql::FileRevision {
            commit_sha: base_commit,
            revision: crate::engine::devql::TemporalRevisionRef {
                kind: crate::engine::devql::TemporalRevisionKind::Temporary,
                id: &revision_id,
                temp_checkpoint_id: Some(row_id),
            },
            commit_unix: revision_unix,
            path: rel_path,
            blob_sha: &blob_sha,
        };
        crate::engine::devql::upsert_current_state_for_content(
            cfg, relational, &revision, &content,
        )
        .await
        .with_context(|| format!("capturing current DevQL state for {rel_path}"))?;
    }
    for rel_path in deleted {
        if let Err(err) =
            crate::engine::devql::delete_current_state_for_path(cfg, relational, rel_path).await
        {
            log::warn!("devql watcher failed deleting `{rel_path}` current state: {err:#}");
        }
    }
    Ok(())
}

fn load_file_from_tree(repo_root: &Path, tree_hash: &str, path: &str) -> Result<String> {
    crate::engine::strategy::manual_commit::run_git(
        repo_root,
        &["show", &format!("{tree_hash}:{path}")],
    )
}

fn load_blob_sha_from_tree(repo_root: &Path, tree_hash: &str, path: &str) -> Result<String> {
    crate::engine::strategy::manual_commit::run_git(
        repo_root,
        &["rev-parse", &format!("{tree_hash}:{path}")],
    )
    .map(|value| value.trim().to_string())
}

async fn load_file_from_tree_blocking(
    repo_root: &Path,
    tree_hash: &str,
    path: &str,
) -> Result<String> {
    let repo_root = repo_root.to_path_buf();
    let tree_hash = tree_hash.to_string();
    let path = path.to_string();
    let path_for_log = path.clone();
    task::spawn_blocking(move || load_file_from_tree(&repo_root, &tree_hash, &path))
        .await
        .with_context(|| format!("joining blocking git show task for `{path_for_log}`"))?
}

async fn load_blob_sha_from_tree_blocking(
    repo_root: &Path,
    tree_hash: &str,
    path: &str,
) -> Result<String> {
    let repo_root = repo_root.to_path_buf();
    let tree_hash = tree_hash.to_string();
    let path = path.to_string();
    let path_for_log = path.clone();
    task::spawn_blocking(move || load_blob_sha_from_tree(&repo_root, &tree_hash, &path))
        .await
        .with_context(|| format!("joining blocking git rev-parse task for `{path_for_log}`"))?
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::engine::devql::file_symbol_id;
    use crate::test_support::git_fixtures::{git_ok, init_test_repo};
    use rusqlite::Connection;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    fn deterministic_uuid(input: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let digest = format!("{:x}", hasher.finalize());
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

        let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::engine::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
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

        let revision_row: (String, String, String, Option<i64>) = conn
            .query_row(
                "SELECT commit_sha, revision_kind, revision_id, temp_checkpoint_id \
                 FROM artefacts_current WHERE path = 'src/lib.rs' AND symbol_id = ?1",
                [file_symbol_id("src/lib.rs")],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("fetch current file revision row");
        assert!(
            !revision_row.0.is_empty(),
            "watch capture should retain base commit sha"
        );
        assert_eq!(revision_row.1, "temporary");
        assert!(revision_row.2.starts_with("temp:"));
        assert!(revision_row.3.is_some());
    }

    #[test]
    fn capture_ignores_directory_events_and_still_updates_file_state() {
        let dir = seed_repo();
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn third() -> i32 {\n    3\n}\n",
        )
        .expect("update file");

        let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::engine::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
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

        let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::engine::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
            .expect("build devql config");
        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
        let sqlite = crate::engine::db::SqliteConnectionPool::connect(db_path.clone())
            .expect("connect sqlite");
        sqlite
            .initialise_devql_schema()
            .expect("initialise devql schema");

        let head_sha = git_ok(dir.path(), &["rev-parse", "HEAD"]);
        let working_blob_sha = git_ok(dir.path(), &["hash-object", "src/lib.rs"]);
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
                        repo_id, symbol_id, artefact_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id,
                        blob_sha, path, language, canonical_kind, language_kind, symbol_fqn,
                        parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte,
                        signature, modifiers, docstring, content_hash, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, 'temporary', 'temp:0', 0, ?5, 'src/lib.rs', 'rust', 'file', 'file', 'src/lib.rs',
                        NULL, NULL, 1, 3, 0, 29, NULL, '[]', NULL, 'stale-hash', datetime(?6, 'unixepoch'))",
                    rusqlite::params![
                        cfg.repo.repo_id,
                        file_symbol,
                        file_artefact_id,
                        head_sha,
                        working_blob_sha,
                        committed_unix,
                    ],
                )?;
                Ok(())
            })
            .expect("seed stale path metadata");

        capture_temporary_checkpoint_batch(&cfg, &[dir.path().join("src/lib.rs")])
            .expect("capture temporary checkpoint over unchanged blob");

        let conn = Connection::open(db_path).expect("open sqlite");
        let current_row: (String, String) = conn
            .query_row(
                "SELECT commit_sha, revision_id FROM artefacts_current WHERE path = 'src/lib.rs' AND symbol_id = ?1",
                [file_symbol_id("src/lib.rs")],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read current file row");
        assert_eq!(current_row.0, head_sha);
        assert_ne!(current_row.1, "temp:0");

        let artefact_row: (String, String) = conn
            .query_row(
                "SELECT commit_sha, revision_id FROM artefacts_current WHERE path = 'src/lib.rs' AND symbol_id = ?1",
                rusqlite::params![file_symbol],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read artefact current row");
        assert_eq!(artefact_row.0, head_sha);
        assert_eq!(artefact_row.1, current_row.1);
    }

    #[test]
    fn capture_only_revises_affected_symbols_for_follow_up_temp_changes() {
        let dir = seed_repo();
        let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::engine::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
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
        capture_temporary_checkpoint_batch(&cfg, std::slice::from_ref(&target))
            .expect("capture second temp batch");

        let db_path = crate::utils::paths::default_relational_db_path(dir.path());
        let conn = Connection::open(db_path).expect("open sqlite");
        let file_revision: String = conn
            .query_row(
                "SELECT revision_id FROM artefacts_current WHERE path = 'src/lib.rs' AND symbol_id = ?1",
                [file_symbol_id("src/lib.rs")],
                |row| row.get(0),
            )
            .expect("read file revision");
        let first_revision: String = conn
            .query_row(
                "SELECT revision_id FROM artefacts_current WHERE path = 'src/lib.rs' AND symbol_fqn = 'src/lib.rs::first'",
                [],
                |row| row.get(0),
            )
            .expect("read first revision");
        let second_revision: String = conn
            .query_row(
                "SELECT revision_id FROM artefacts_current WHERE path = 'src/lib.rs' AND symbol_fqn = 'src/lib.rs::second'",
                [],
                |row| row.get(0),
            )
            .expect("read second revision");

        assert_eq!(file_revision, "temp:2");
        assert_eq!(first_revision, "temp:2");
        assert_eq!(second_revision, "temp:1");
    }

    #[test]
    fn capture_skips_no_content_change_events() {
        let dir = seed_repo();

        let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::engine::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
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

        let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::engine::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
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
        let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::engine::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
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

        let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::engine::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
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
    fn capture_workspace_revision_id_is_referenced_by_artefacts_current() {
        // Verify that temp_checkpoint_id on artefacts_current equals the
        // workspace_revisions.id that was inserted for that batch.
        let dir = seed_repo();
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn linked() -> i32 {\n    7\n}\n",
        )
        .expect("update file");

        let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::engine::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
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

        let temp_checkpoint_id: i64 = conn
            .query_row(
                "SELECT temp_checkpoint_id FROM artefacts_current \
                 WHERE path = 'src/lib.rs' AND symbol_id = ?1",
                [file_symbol_id("src/lib.rs")],
                |row| row.get(0),
            )
            .expect("fetch temp_checkpoint_id from artefacts_current");

        assert_eq!(
            temp_checkpoint_id, workspace_id,
            "temp_checkpoint_id in artefacts_current must equal workspace_revisions.id"
        );

        // revision_id should be "temp:<workspace_id>"
        let revision_id: String = conn
            .query_row(
                "SELECT revision_id FROM artefacts_current \
                 WHERE path = 'src/lib.rs' AND symbol_id = ?1",
                [file_symbol_id("src/lib.rs")],
                |row| row.get(0),
            )
            .expect("fetch revision_id");
        assert_eq!(
            revision_id,
            format!("temp:{workspace_id}"),
            "revision_id must be temp:<workspace_revisions.id>"
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

        let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::engine::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
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

        let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::engine::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
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
