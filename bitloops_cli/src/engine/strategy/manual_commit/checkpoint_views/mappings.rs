pub fn read_commit_checkpoint_mappings(repo_root: &Path) -> Result<std::collections::HashMap<String, String>> {
    let sqlite_path = resolve_temporary_checkpoint_sqlite_path(repo_root)?;
    let sqlite = crate::engine::db::SqliteConnectionPool::connect(sqlite_path)
        .context("opening SQLite database for commit-checkpoint mappings")?;
    sqlite
        .initialise_checkpoint_schema()
        .context("initialising checkpoint schema for commit-checkpoint mappings")?;

    let repo_id = crate::engine::devql::resolve_repo_id(repo_root)
        .context("resolving repo identity for commit-checkpoint mappings")?;

    sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT commit_sha, checkpoint_id
             FROM commit_checkpoints
             WHERE repo_id = ?1
             ORDER BY created_at DESC, checkpoint_id DESC",
        )?;
        let mut rows = stmt.query(rusqlite::params![repo_id.as_str()])?;

        let mut out: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        while let Some(row) = rows.next()? {
            let commit_sha = row.get::<_, String>(0)?.trim().to_string();
            let checkpoint_id = row.get::<_, String>(1)?.trim().to_string();
            if commit_sha.is_empty() || !is_valid_checkpoint_id(&checkpoint_id) {
                continue;
            }
            out.entry(commit_sha).or_insert(checkpoint_id);
        }
        Ok(out)
    })
}
