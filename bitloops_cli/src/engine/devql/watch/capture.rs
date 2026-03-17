use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub(crate) fn capture_temporary_checkpoint_batch(
    cfg: &crate::engine::devql::DevqlConfig,
    changed_paths: &[PathBuf],
) -> Result<()> {
    if changed_paths.is_empty() {
        return Ok(());
    }

    let repo_root = &cfg.repo_root;
    let base_commit = crate::engine::strategy::manual_commit::run_git(repo_root, &["rev-parse", "HEAD"])
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
            modified.push(rel);
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

    let backend_cfg = crate::store_config::resolve_store_backend_config_for_repo(repo_root)
        .context("resolving store config for watcher capture")?;
    let sqlite_path = crate::store_config::resolve_sqlite_db_path_for_repo(
        repo_root,
        backend_cfg.relational.sqlite_path.as_deref(),
    )
    .context("resolving SQLite path for watcher capture")?;
    let sqlite = crate::engine::db::SqliteConnectionPool::connect(sqlite_path)?;
    sqlite.initialise_checkpoint_schema()?;
    sqlite.initialise_devql_schema()?;

    let repo_id = crate::engine::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for watch capture")?
        .repo_id;

    let session_id = "devql-watcher".to_string();

    let row_id = sqlite.with_connection(|conn| {
        let next_step: i64 = conn.query_row(
            "SELECT COALESCE(MAX(step_number), 0) + 1 FROM temporary_checkpoints WHERE session_id = ?1 AND repo_id = ?2",
            rusqlite::params![session_id, repo_id],
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

    // Temporary capture remains local-only by design.
    sqlite.with_connection(|conn| {
        for rel_path in &modified {
            let content = load_file_from_tree(repo_root, &tree_hash, rel_path).unwrap_or_default();
            let blob_sha = crate::engine::devql::deterministic_uuid(&format!("{tree_hash}|{rel_path}"));

            conn.execute(
                "INSERT INTO file_state (repo_id, revision_kind, revision_id, tree_hash, commit_sha, base_commit_sha, path, blob_sha)
                 VALUES (?1, 'temporary', ?2, ?3, NULL, ?4, ?5, ?6)
                 ON CONFLICT(repo_id, revision_kind, revision_id, path)
                 DO UPDATE SET blob_sha = excluded.blob_sha",
                rusqlite::params![repo_id, revision_id, tree_hash, base_commit, rel_path, blob_sha],
            )?;

            conn.execute(
                "INSERT INTO current_file_state (repo_id, path, current_scope, revision_kind, revision_id, tree_hash, commit_sha, base_commit_sha, blob_sha, committed_at, updated_at)
                 VALUES (?1, ?2, 'visible', 'temporary', ?3, ?4, NULL, ?5, ?6, datetime('now'), datetime('now'))
                 ON CONFLICT(repo_id, path, current_scope)
                 DO UPDATE SET revision_kind = excluded.revision_kind, revision_id = excluded.revision_id, tree_hash = excluded.tree_hash, base_commit_sha = excluded.base_commit_sha, blob_sha = excluded.blob_sha, updated_at = datetime('now')",
                rusqlite::params![repo_id, rel_path, revision_id, tree_hash, base_commit, blob_sha],
            )?;

            let _ = content;
        }
        Ok(())
    })?;

    Ok(())
}

fn load_file_from_tree(repo_root: &Path, tree_hash: &str, path: &str) -> Result<String> {
    crate::engine::strategy::manual_commit::run_git(repo_root, &["show", &format!("{tree_hash}:{path}")])
}
