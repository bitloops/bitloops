use super::*;

fn open_commit_checkpoint_mapping_store(
    repo_root: &Path,
) -> Result<(
    crate::host::relational_store::DefaultRelationalStore,
    String,
)> {
    let relational = crate::host::relational_store::DefaultRelationalStore::
        open_primary_for_repo_root_preferring_bound_config(repo_root)
        .context("opening relational store for commit_checkpoints")?;
    initialise_checkpoint_relational_roles(&relational)?;
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for commit_checkpoints")?
        .repo_id;
    Ok((relational, repo_id))
}

pub(crate) fn commit_has_checkpoint_mapping(repo_root: &Path, commit_sha: &str) -> Result<bool> {
    let (relational, repo_id) = open_commit_checkpoint_mapping_store(repo_root)?;
    let sql = format!(
        "SELECT 1 AS present
         FROM commit_checkpoints
         WHERE commit_sha = '{}' AND repo_id = '{}'
         LIMIT 1",
        crate::host::devql::esc_pg(commit_sha),
        crate::host::devql::esc_pg(&repo_id),
    );
    let rows = query_checkpoint_metadata_rows(&relational, &sql);
    match rows {
        Ok(hit) => Ok(!hit.is_empty()),
        Err(err)
            if err
                .to_string()
                .contains("no such table: commit_checkpoints") =>
        {
            Ok(false)
        }
        Err(err) => Err(err),
    }
}

pub fn insert_commit_checkpoint_mapping(
    repo_root: &Path,
    commit_sha: &str,
    checkpoint_id: &str,
) -> Result<()> {
    let (relational, repo_id) = open_commit_checkpoint_mapping_store(repo_root)?;
    exec_checkpoint_metadata_statements(
        &relational,
        &[build_insert_commit_checkpoint_mapping_sql(
            &repo_id,
            commit_sha,
            checkpoint_id,
        )],
    )
    .context("inserting commit_checkpoints row")
}
