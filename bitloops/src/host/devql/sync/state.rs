use anyhow::{Result, bail};

use crate::host::devql::RelationalStorage;
use crate::host::devql::db_utils::esc_pg;
use crate::host::devql::sql_now;

pub(crate) async fn write_sync_started(
    store: &RelationalStorage,
    repo_id: &str,
    repo_root: &str,
    reason: &str,
    parser_version: &str,
    extractor_version: &str,
) -> Result<()> {
    let now_sql = sql_now(store);
    let sql = format!(
        "INSERT INTO repo_sync_state (\
repo_id, repo_root, active_branch, head_commit_sha, head_tree_sha, parser_version, extractor_version, \
last_sync_started_at, last_sync_completed_at, last_sync_status, last_sync_reason\
) VALUES (\
'{}', '{}', NULL, NULL, NULL, '{}', '{}', {}, NULL, 'running', '{}'\
) ON CONFLICT (repo_id) DO UPDATE SET \
repo_root = EXCLUDED.repo_root, \
active_branch = NULL, \
head_commit_sha = NULL, \
head_tree_sha = NULL, \
parser_version = EXCLUDED.parser_version, \
extractor_version = EXCLUDED.extractor_version, \
last_sync_started_at = {}, \
last_sync_completed_at = NULL, \
last_sync_status = 'running', \
last_sync_reason = EXCLUDED.last_sync_reason",
        esc_pg(repo_id),
        esc_pg(repo_root),
        esc_pg(parser_version),
        esc_pg(extractor_version),
        now_sql,
        esc_pg(reason),
        now_sql,
    );
    store.exec(&sql).await
}

pub(crate) async fn write_sync_completed(
    store: &RelationalStorage,
    repo_id: &str,
    head_commit_sha: Option<&str>,
    head_tree_sha: Option<&str>,
    active_branch: Option<&str>,
    parser_version: &str,
    extractor_version: &str,
) -> Result<()> {
    ensure_repo_sync_state_exists(store, repo_id).await?;
    let now_sql = sql_now(store);
    let sql = format!(
        "UPDATE repo_sync_state SET \
active_branch = {}, \
head_commit_sha = {}, \
head_tree_sha = {}, \
parser_version = '{}', \
extractor_version = '{}', \
last_sync_completed_at = {}, \
last_sync_status = 'completed' \
WHERE repo_id = '{}'",
        nullable_text_sql(active_branch),
        nullable_text_sql(head_commit_sha),
        nullable_text_sql(head_tree_sha),
        esc_pg(parser_version),
        esc_pg(extractor_version),
        now_sql,
        esc_pg(repo_id),
    );
    store.exec(&sql).await
}

pub(crate) async fn write_sync_failed(store: &RelationalStorage, repo_id: &str) -> Result<()> {
    ensure_repo_sync_state_exists(store, repo_id).await?;
    let sql = format!(
        "UPDATE repo_sync_state SET last_sync_status = 'failed' WHERE repo_id = '{}'",
        esc_pg(repo_id),
    );
    store.exec(&sql).await
}

async fn ensure_repo_sync_state_exists(store: &RelationalStorage, repo_id: &str) -> Result<()> {
    let rows = store
        .query_rows(&format!(
            "SELECT repo_id FROM repo_sync_state WHERE repo_id = '{}' LIMIT 1",
            esc_pg(repo_id),
        ))
        .await?;
    if rows.is_empty() {
        bail!("repo_sync_state row missing for repo_id `{repo_id}`")
    }
    Ok(())
}

fn nullable_text_sql(value: Option<&str>) -> String {
    value
        .map(|value| format!("'{}'", esc_pg(value)))
        .unwrap_or_else(|| "NULL".to_string())
}
