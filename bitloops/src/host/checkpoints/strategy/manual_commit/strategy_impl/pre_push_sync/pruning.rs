use super::*;

pub(super) async fn prune_historical_rows_with_retention(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    keep_commits: usize,
) -> Result<()> {
    let statements = build_retention_prune_sql(repo_id, keep_commits);
    local.exec_batch_transactional(&statements).await
}

fn build_retention_prune_sql(repo_id: &str, keep_commits: usize) -> Vec<String> {
    let repo_id = crate::host::devql::esc_pg(repo_id);
    vec![
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
            "DELETE FROM artefact_snapshots \
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
            "DELETE FROM artefacts \
WHERE repo_id = '{repo_id}' \
  AND NOT EXISTS (\
    SELECT 1 FROM artefact_snapshots s \
    WHERE s.repo_id = artefacts.repo_id \
      AND s.artefact_id = artefacts.artefact_id\
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
