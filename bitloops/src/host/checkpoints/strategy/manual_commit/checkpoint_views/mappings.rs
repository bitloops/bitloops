use super::*;

/// Look up the session_id for a given commit SHA via commit_checkpoints → checkpoint_sessions.
pub fn lookup_session_id_for_commit(repo_root: &Path, commit_sha: &str) -> Result<Option<String>> {
    let (relational, repo_id) = open_checkpoint_relational_store(repo_root)
        .context("opening relational store for session lookup")?;
    let sql = format!(
        "SELECT cs.session_id
         FROM commit_checkpoints cc
         JOIN checkpoint_sessions cs ON cs.checkpoint_id = cc.checkpoint_id
         WHERE cc.commit_sha = '{}' AND cc.repo_id = '{}'
         LIMIT 1",
        crate::host::devql::esc_pg(commit_sha),
        crate::host::devql::esc_pg(&repo_id),
    );
    Ok(query_checkpoint_metadata_rows(&relational, &sql)?
        .into_iter()
        .next()
        .and_then(|row| checkpoint_row_text(&row, "session_id")))
}

pub fn read_commit_checkpoint_mappings(
    repo_root: &Path,
) -> Result<std::collections::HashMap<String, String>> {
    let (relational, repo_id) = open_checkpoint_relational_store(repo_root)
        .context("opening relational store for commit-checkpoint mappings")?;
    let sql = format!(
        "SELECT commit_sha, checkpoint_id
         FROM commit_checkpoints
         WHERE repo_id = '{}'
         ORDER BY created_at DESC, checkpoint_id DESC",
        crate::host::devql::esc_pg(repo_id.as_str()),
    );
    let mut out: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for row in query_checkpoint_metadata_rows(&relational, &sql)? {
        let Some(commit_sha) = checkpoint_row_text(&row, "commit_sha") else {
            continue;
        };
        let Some(checkpoint_id) = checkpoint_row_text(&row, "checkpoint_id") else {
            continue;
        };
        if !is_valid_checkpoint_id(&checkpoint_id) {
            continue;
        }
        out.entry(commit_sha).or_insert(checkpoint_id);
    }
    Ok(out)
}
