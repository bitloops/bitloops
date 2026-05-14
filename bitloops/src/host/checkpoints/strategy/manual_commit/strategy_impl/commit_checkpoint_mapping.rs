use super::*;

fn open_commit_checkpoint_mapping_db(
    repo_root: &Path,
) -> Result<(crate::storage::SqliteConnectionPool, String)> {
    open_commit_checkpoint_mapping_pool(repo_root, true)
}

fn open_commit_checkpoint_mapping_pool(
    repo_root: &Path,
    initialise_schema: bool,
) -> Result<(crate::storage::SqliteConnectionPool, String)> {
    let relational =
        crate::host::relational_store::DefaultRelationalStore::open_local_for_repo_root(repo_root)
            .context("opening relational store for commit_checkpoints")?;
    if initialise_schema {
        relational
            .initialise_local_relational_checkpoint_schema()
            .context("initialising relational checkpoint schema for commit_checkpoints")?;
    }
    let sqlite = crate::host::relational_store::RelationalStore::local_sqlite_pool(&relational)
        .context("opening SQLite for commit_checkpoints")?;

    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for commit_checkpoints")?
        .repo_id;
    Ok((sqlite, repo_id))
}

pub(crate) fn commit_has_checkpoint_mapping(repo_root: &Path, commit_sha: &str) -> Result<bool> {
    use rusqlite::OptionalExtension;

    let (sqlite, repo_id) = open_commit_checkpoint_mapping_pool(repo_root, false)?;
    sqlite.with_connection(|conn| {
        let result = conn
            .query_row(
                "SELECT 1
             FROM commit_checkpoints
             WHERE commit_sha = ?1 AND repo_id = ?2
             LIMIT 1",
                rusqlite::params![commit_sha, repo_id],
                |row| row.get::<_, i64>(0),
            )
            .optional();

        match result {
            Ok(hit) => Ok(hit.is_some()),
            Err(err)
                if err
                    .to_string()
                    .contains("no such table: commit_checkpoints") =>
            {
                Ok(false)
            }
            Err(err) => Err(anyhow::Error::from(err)),
        }
    })
}

pub fn insert_commit_checkpoint_mapping(
    repo_root: &Path,
    commit_sha: &str,
    checkpoint_id: &str,
) -> Result<()> {
    let (sqlite, repo_id) = open_commit_checkpoint_mapping_db(repo_root)?;
    sqlite.with_write_connection(|conn| {
        conn.execute(
            "INSERT OR IGNORE INTO commit_checkpoints (commit_sha, checkpoint_id, repo_id)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![commit_sha, checkpoint_id, repo_id],
        )
        .context("inserting commit_checkpoints row")?;
        Ok(())
    })
}
