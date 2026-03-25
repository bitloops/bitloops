use super::*;

pub(super) async fn prune_historical_rows_up_to_commit(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
) -> Result<()> {
    let commit_rows = local
        .query_rows(&format!(
            "SELECT committed_at FROM commits WHERE repo_id = '{}' AND commit_sha = '{}' LIMIT 1",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(commit_sha),
        ))
        .await?;
    let Some(committed_at) = commit_rows
        .first()
        .and_then(|row| sql_helpers::row_text(row, "committed_at"))
    else {
        return Ok(());
    };
    if committed_at.trim().is_empty() {
        return Ok(());
    }

    let statements = build_prune_before_timestamp_sql(repo_id, committed_at);
    local.exec_batch_transactional(&statements).await
}

pub(super) async fn prune_historical_rows_with_retention(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    keep_commits: usize,
) -> Result<()> {
    let statements = build_retention_prune_sql(repo_id, keep_commits);
    local.exec_batch_transactional(&statements).await
}

fn build_prune_before_timestamp_sql(repo_id: &str, committed_at: &str) -> Vec<String> {
    let repo_id = crate::host::devql::esc_pg(repo_id);
    let committed_at = crate::host::devql::esc_pg(committed_at);
    vec![
        format!(
            "DELETE FROM artefacts \
WHERE repo_id = '{repo_id}' \
  AND blob_sha IN (\
    SELECT DISTINCT blob_sha \
    FROM file_state \
    WHERE repo_id = '{repo_id}' \
      AND commit_sha IN (\
        SELECT commit_sha FROM commits WHERE repo_id = '{repo_id}' AND committed_at <= '{committed_at}'\
      )\
  )"
        ),
        format!(
            "DELETE FROM artefact_edges \
WHERE repo_id = '{repo_id}' \
  AND blob_sha IN (\
    SELECT DISTINCT blob_sha \
    FROM file_state \
    WHERE repo_id = '{repo_id}' \
      AND commit_sha IN (\
        SELECT commit_sha FROM commits WHERE repo_id = '{repo_id}' AND committed_at <= '{committed_at}'\
      )\
  )"
        ),
        format!(
            "DELETE FROM file_state \
WHERE repo_id = '{repo_id}' \
  AND commit_sha IN (\
    SELECT commit_sha FROM commits WHERE repo_id = '{repo_id}' AND committed_at <= '{committed_at}'\
  )"
        ),
    ]
}

fn build_retention_prune_sql(repo_id: &str, keep_commits: usize) -> Vec<String> {
    let repo_id = crate::host::devql::esc_pg(repo_id);
    vec![
        format!(
            "DELETE FROM artefacts \
WHERE repo_id = '{repo_id}' \
  AND blob_sha IN (\
    SELECT DISTINCT blob_sha FROM file_state \
    WHERE repo_id = '{repo_id}' \
      AND commit_sha NOT IN (\
        SELECT commit_sha \
        FROM commits \
        WHERE repo_id = '{repo_id}' \
        ORDER BY committed_at DESC, commit_sha DESC \
        LIMIT {keep_commits}\
      )\
  )"
        ),
        format!(
            "DELETE FROM artefact_edges \
WHERE repo_id = '{repo_id}' \
  AND blob_sha IN (\
    SELECT DISTINCT blob_sha FROM file_state \
    WHERE repo_id = '{repo_id}' \
      AND commit_sha NOT IN (\
        SELECT commit_sha \
        FROM commits \
        WHERE repo_id = '{repo_id}' \
        ORDER BY committed_at DESC, commit_sha DESC \
        LIMIT {keep_commits}\
      )\
  )"
        ),
        format!(
            "DELETE FROM file_state \
WHERE repo_id = '{repo_id}' \
  AND commit_sha NOT IN (\
    SELECT commit_sha \
    FROM commits \
    WHERE repo_id = '{repo_id}' \
    ORDER BY committed_at DESC, commit_sha DESC \
    LIMIT {keep_commits}\
  )"
        ),
    ]
}
