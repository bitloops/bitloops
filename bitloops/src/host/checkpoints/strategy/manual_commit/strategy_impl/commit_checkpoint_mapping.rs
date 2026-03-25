use super::*;

fn open_commit_checkpoint_mapping_db(
    repo_root: &Path,
) -> Result<(crate::storage::SqliteConnectionPool, String)> {
    let sqlite_path = resolve_temporary_checkpoint_sqlite_path(repo_root)
        .context("resolving SQLite path for commit_checkpoints")?;
    let sqlite = crate::storage::SqliteConnectionPool::connect_existing(sqlite_path)
        .context("opening SQLite for commit_checkpoints")?;
    sqlite
        .initialise_checkpoint_schema()
        .context("initialising checkpoint schema for commit_checkpoints")?;

    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for commit_checkpoints")?
        .repo_id;
    Ok((sqlite, repo_id))
}

pub(crate) fn commit_has_checkpoint_mapping(repo_root: &Path, commit_sha: &str) -> Result<bool> {
    use rusqlite::OptionalExtension;

    let (sqlite, repo_id) = open_commit_checkpoint_mapping_db(repo_root)?;
    sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT 1
             FROM commit_checkpoints
             WHERE commit_sha = ?1 AND repo_id = ?2
             LIMIT 1",
            rusqlite::params![commit_sha, repo_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|hit| hit.is_some())
        .map_err(anyhow::Error::from)
    })
}

pub fn insert_commit_checkpoint_mapping(
    repo_root: &Path,
    commit_sha: &str,
    checkpoint_id: &str,
) -> Result<()> {
    let (sqlite, repo_id) = open_commit_checkpoint_mapping_db(repo_root)?;
    sqlite.with_connection(|conn| {
        conn.execute(
            "INSERT OR IGNORE INTO commit_checkpoints (commit_sha, checkpoint_id, repo_id)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![commit_sha, checkpoint_id, repo_id],
        )
        .context("inserting commit_checkpoints row")?;
        Ok(())
    })
}
