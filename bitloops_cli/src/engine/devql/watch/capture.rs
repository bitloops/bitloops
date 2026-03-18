use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

pub(crate) fn capture_temporary_checkpoint_batch(
    cfg: &crate::engine::devql::DevqlConfig,
    changed_paths: &[PathBuf],
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

    let backend_cfg = crate::store_config::resolve_store_backend_config_for_repo(repo_root)
        .context("resolving store config for watcher capture")?;
    let sqlite_path = crate::store_config::resolve_sqlite_db_path_for_repo(
        repo_root,
        backend_cfg.relational.sqlite_path.as_deref(),
    )
    .context("resolving SQLite path for watcher capture")?;
    let sqlite = crate::engine::db::SqliteConnectionPool::connect(sqlite_path.clone())?;
    sqlite.initialise_checkpoint_schema()?;
    sqlite.initialise_devql_schema()?;

    let repo_id = crate::engine::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for watch capture")?
        .repo_id;

    let session_id = "devql-watcher".to_string();
    let latest_tree_hash = sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT tree_hash FROM temporary_checkpoints WHERE session_id = ?1 AND repo_id = ?2 ORDER BY id DESC LIMIT 1",
            rusqlite::params![&session_id, &repo_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(anyhow::Error::from)
    })?;
    if latest_tree_hash.as_deref() == Some(tree_hash.as_str()) {
        return Ok(());
    }

    let row_id = sqlite.with_connection(|conn| {
        let next_step: i64 = conn.query_row(
            "SELECT COALESCE(MAX(step_number), 0) + 1 FROM temporary_checkpoints WHERE session_id = ?1 AND repo_id = ?2",
            rusqlite::params![&session_id, &repo_id],
            |row| row.get(0),
        )?;

        conn.execute(
            "INSERT INTO temporary_checkpoints (
                session_id, repo_id, tree_hash, step_number,
                modified_files, new_files, deleted_files,
                author_name, author_email, commit_message
            ) VALUES (?1, ?2, ?3, ?4, ?5, '[]', ?6, 'Bitloops', 'bitloops@localhost', 'devql watcher checkpoint')",
            rusqlite::params![
                session_id,
                repo_id,
                tree_hash,
                next_step,
                serde_json::to_string(&modified).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&deleted).unwrap_or_else(|_| "[]".to_string()),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    })?;

    let revision_id = format!("temp:{row_id}");
    let revision_unix = current_unix_timestamp();
    let relational = crate::engine::devql::RelationalStorage::Sqlite { path: sqlite_path };
    let runtime = tokio::runtime::Runtime::new().context("creating watcher capture runtime")?;

    runtime.block_on(async {
        for rel_path in &modified {
            let content = match load_file_from_tree(repo_root, &tree_hash, rel_path) {
                Ok(value) => value,
                Err(err) => {
                    log::warn!("devql watcher skipped `{rel_path}`: {err:#}");
                    continue;
                }
            };
            let blob_sha = match load_blob_sha_from_tree(repo_root, &tree_hash, rel_path) {
                Ok(value) => value,
                Err(err) => {
                    log::warn!("devql watcher skipped `{rel_path}` blob lookup: {err:#}");
                    continue;
                }
            };
            let revision = crate::engine::devql::FileRevision {
                commit_sha: &base_commit,
                revision: crate::engine::devql::RevisionRef {
                    kind: "temporary",
                    id: &revision_id,
                    temp_checkpoint_id: Some(row_id),
                },
                commit_unix: revision_unix,
                path: rel_path,
                blob_sha: &blob_sha,
            };
            crate::engine::devql::upsert_current_state_for_content(
                cfg,
                &relational,
                &revision,
                &content,
            )
            .await
            .with_context(|| format!("capturing current DevQL state for {rel_path}"))?;
        }
        for rel_path in &deleted {
            if let Err(err) =
                crate::engine::devql::delete_current_state_for_path(cfg, &relational, rel_path)
                    .await
            {
                log::warn!("devql watcher failed deleting `{rel_path}` current state: {err:#}");
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;

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
    use crate::test_support::git_fixtures::{git_ok, init_test_repo};
    use rusqlite::Connection;
    use tempfile::TempDir;

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

        let db_path = crate::engine::paths::default_relational_db_path(dir.path());
        let conn = Connection::open(db_path).expect("open sqlite");

        let temp_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM temporary_checkpoints", [], |row| {
                row.get(0)
            })
            .expect("count temporary checkpoints");
        assert_eq!(temp_rows, 1);

        let current_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM current_file_state WHERE path = 'src/lib.rs'",
                [],
                |row| row.get(0),
            )
            .expect("count current file state rows");
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
                "SELECT commit_sha, revision_kind, revision_id, temp_checkpoint_id FROM current_file_state WHERE path = 'src/lib.rs'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("fetch current revision row");
        assert!(!revision_row.0.is_empty(), "watch capture should retain base commit sha");
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

        let db_path = crate::engine::paths::default_relational_db_path(dir.path());
        let conn = Connection::open(db_path).expect("open sqlite");
        let current_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM current_file_state WHERE path = 'src/lib.rs'",
                [],
                |row| row.get(0),
            )
            .expect("count current file state rows");
        assert_eq!(current_rows, 1);
    }

    #[test]
    fn capture_skips_no_content_change_events() {
        let dir = seed_repo();

        let repo = crate::engine::devql::resolve_repo_identity(dir.path()).expect("resolve repo");
        let cfg = crate::engine::devql::DevqlConfig::from_env(dir.path().to_path_buf(), repo)
            .expect("build devql config");
        capture_temporary_checkpoint_batch(&cfg, &[dir.path().join("src/lib.rs")])
            .expect("capture no-op batch");

        let db_path = crate::engine::paths::default_relational_db_path(dir.path());
        if !db_path.exists() {
            return;
        }
        let conn = Connection::open(db_path).expect("open sqlite");
        let temp_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM temporary_checkpoints", [], |row| {
                row.get(0)
            })
            .expect("count temporary checkpoints");
        assert_eq!(temp_rows, 0, "no-content-change capture should not persist temp rows");
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

        let db_path = crate::engine::paths::default_relational_db_path(dir.path());
        let conn = Connection::open(db_path).expect("open sqlite");
        let temp_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM temporary_checkpoints", [], |row| {
                row.get(0)
            })
            .expect("count temporary checkpoints");
        assert_eq!(temp_rows, 1, "duplicate tree hash should be deduplicated");
    }
}
