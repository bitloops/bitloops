use super::*;

pub(super) async fn mark_branch_sync_pending(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    remote_name: &str,
    remote_branch: &str,
    local_sha: &str,
) -> Result<()> {
    let key = branch_sync_pending_key(remote_name, remote_branch);
    let sql = build_sync_state_upsert_sql(repo_id, &key, local_sha);
    local.exec(&sql).await
}

pub(super) async fn mark_branch_sync_complete(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    remote_name: &str,
    remote_branch: &str,
    local_sha: &str,
) -> Result<()> {
    let watermark_key = branch_sync_watermark_key(remote_name, remote_branch);
    let pending_key = branch_sync_pending_key(remote_name, remote_branch);
    let statements = vec![
        build_sync_state_upsert_sql(repo_id, constants::PRE_PUSH_SYNC_WATERMARK_KEY, local_sha),
        build_sync_state_upsert_sql(repo_id, &watermark_key, local_sha),
        format!(
            "DELETE FROM sync_state WHERE repo_id = '{}' AND state_key = '{}'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(&pending_key),
        ),
    ];
    local.exec_batch_transactional(&statements).await
}

pub(super) fn branch_sync_watermark_key(remote_name: &str, remote_branch: &str) -> String {
    format!(
        "{}:{}:{}",
        constants::PRE_PUSH_SYNC_WATERMARK_KEY,
        remote_name.trim(),
        remote_branch.trim()
    )
}

fn branch_sync_pending_key(remote_name: &str, remote_branch: &str) -> String {
    format!(
        "{}:{}:{}",
        constants::PRE_PUSH_SYNC_PENDING_KEY_PREFIX,
        remote_name.trim(),
        remote_branch.trim()
    )
}

pub(super) async fn load_sync_state_value(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    state_key: &str,
) -> Result<Option<String>> {
    let rows = local
        .query_rows(&format!(
            "SELECT state_value FROM sync_state \
WHERE repo_id = '{}' AND state_key = '{}' LIMIT 1",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(state_key),
        ))
        .await?;
    Ok(rows
        .first()
        .and_then(|row| sql_helpers::row_text(row, "state_value"))
        .map(str::to_string))
}

fn build_sync_state_upsert_sql(repo_id: &str, state_key: &str, state_value: &str) -> String {
    format!(
        "INSERT INTO sync_state (repo_id, state_key, state_value, updated_at) VALUES ('{}', '{}', '{}', datetime('now')) \
ON CONFLICT (repo_id, state_key) DO UPDATE SET state_value = EXCLUDED.state_value, updated_at = datetime('now')",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(state_key),
        crate::host::devql::esc_pg(state_value),
    )
}
