use super::*;

const COMMIT_HISTORY_STATUS_COMPLETED: &str = "completed";
const COMMIT_HISTORY_STATUS_FAILED: &str = "failed";
const COMMIT_CHECKPOINT_STATUS_COMPLETED: &str = "completed";
const COMMIT_CHECKPOINT_STATUS_FAILED: &str = "failed";
const COMMIT_CHECKPOINT_STATUS_NOT_APPLICABLE: &str = "not_applicable";
const HISTORICAL_BRANCH_WATERMARK_PREFIX: &str = "historical_ingest.branch.";

#[derive(Debug, Clone, Default)]
pub(super) struct CommitIngestLedgerEntry {
    pub(super) history_status: String,
    pub(super) checkpoint_status: String,
    pub(super) checkpoint_id: Option<String>,
}

pub(super) fn historical_branch_watermark_key(branch: &str) -> String {
    format!(
        "{HISTORICAL_BRANCH_WATERMARK_PREFIX}{}",
        branch.trim().replace('\n', " ")
    )
}

pub(super) fn checked_out_branch_name(repo_root: &Path) -> Option<String> {
    run_git(repo_root, &["branch", "--show-current"])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) async fn select_missing_branch_commit_segment(
    repo_root: &Path,
    relational: &RelationalStorage,
    repo_id: &str,
    branch_name: Option<&str>,
    head_sha: &str,
) -> Result<Vec<String>> {
    let head_sha = head_sha.trim();
    if head_sha.is_empty() {
        return Ok(Vec::new());
    }

    if uses_local_ingest_watermarks(relational)
        && let Some(branch_name) = branch_name.map(str::trim).filter(|value| !value.is_empty())
    {
        let watermark_key = historical_branch_watermark_key(branch_name);
        let branch_watermark = load_sync_state_value_for_repo(relational, repo_id, &watermark_key)
            .await?;
        if let Some(branch_watermark) = branch_watermark.as_deref()
            && !branch_watermark.is_empty()
            && commit_is_ancestor_of(repo_root, branch_watermark, head_sha)
        {
            return list_commit_range(repo_root, &format!("{branch_watermark}..{head_sha}"));
        }
    }

    if let Some(ancestor_sha) =
        nearest_reachable_completed_commit(repo_root, relational, repo_id, head_sha).await?
    {
        return list_commit_range(repo_root, &format!("{ancestor_sha}..{head_sha}"));
    }

    list_commit_range(repo_root, head_sha)
}

pub(super) fn uses_local_ingest_watermarks(relational: &RelationalStorage) -> bool {
    matches!(relational.dialect(), RelationalDialect::Sqlite)
}

pub(super) async fn load_commit_ingest_ledger_entry(
    relational: &RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
) -> Result<Option<CommitIngestLedgerEntry>> {
    let sql = format!(
        "SELECT history_status, checkpoint_status, checkpoint_id \
         FROM commit_ingest_ledger \
         WHERE repo_id = '{}' AND commit_sha = '{}' \
         LIMIT 1",
        esc_pg(repo_id),
        esc_pg(commit_sha),
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows.first().map(|row| CommitIngestLedgerEntry {
        history_status: row
            .get("history_status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        checkpoint_status: row
            .get("checkpoint_status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        checkpoint_id: row
            .get("checkpoint_id")
            .and_then(Value::as_str)
            .map(str::to_string),
    }))
}

pub(super) fn commit_is_fully_ingested(entry: &CommitIngestLedgerEntry) -> bool {
    entry.history_status == COMMIT_HISTORY_STATUS_COMPLETED
        && matches!(
            entry.checkpoint_status.as_str(),
            COMMIT_CHECKPOINT_STATUS_COMPLETED | COMMIT_CHECKPOINT_STATUS_NOT_APPLICABLE
        )
}

pub(super) async fn mark_commit_history_completed(
    relational: &RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
    checkpoint_id: Option<&str>,
) -> Result<()> {
    let checkpoint_status = checkpoint_id
        .map(|_| "pending")
        .unwrap_or(COMMIT_CHECKPOINT_STATUS_NOT_APPLICABLE);
    upsert_commit_ingest_ledger_row(
        relational,
        repo_id,
        commit_sha,
        COMMIT_HISTORY_STATUS_COMPLETED,
        checkpoint_status,
        checkpoint_id,
        None,
    )
    .await
}

pub(super) async fn mark_commit_checkpoint_completed(
    relational: &RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
    checkpoint_id: Option<&str>,
) -> Result<()> {
    upsert_commit_ingest_ledger_row(
        relational,
        repo_id,
        commit_sha,
        COMMIT_HISTORY_STATUS_COMPLETED,
        checkpoint_id
            .map(|_| COMMIT_CHECKPOINT_STATUS_COMPLETED)
            .unwrap_or(COMMIT_CHECKPOINT_STATUS_NOT_APPLICABLE),
        checkpoint_id,
        None,
    )
    .await
}

pub(super) async fn mark_commit_ingest_failed(
    relational: &RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
    checkpoint_id: Option<&str>,
    history_completed: bool,
    error_message: &str,
) -> Result<()> {
    let history_status = if history_completed {
        COMMIT_HISTORY_STATUS_COMPLETED
    } else {
        COMMIT_HISTORY_STATUS_FAILED
    };
    let checkpoint_status = if checkpoint_id.is_some() {
        COMMIT_CHECKPOINT_STATUS_FAILED
    } else if history_completed {
        COMMIT_CHECKPOINT_STATUS_NOT_APPLICABLE
    } else {
        COMMIT_CHECKPOINT_STATUS_FAILED
    };
    upsert_commit_ingest_ledger_row(
        relational,
        repo_id,
        commit_sha,
        history_status,
        checkpoint_status,
        checkpoint_id,
        Some(error_message),
    )
    .await
}

async fn nearest_reachable_completed_commit(
    repo_root: &Path,
    relational: &RelationalStorage,
    repo_id: &str,
    head_sha: &str,
) -> Result<Option<String>> {
    let completed = load_fully_ingested_commits(relational, repo_id).await?;
    if completed.is_empty() {
        return Ok(None);
    }

    let output = run_git(repo_root, &["rev-list", head_sha])?;
    Ok(output
        .lines()
        .map(str::trim)
        .find(|commit_sha| completed.contains(*commit_sha))
        .map(str::to_string))
}

async fn load_fully_ingested_commits(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<HashSet<String>> {
    let sql = format!(
        "SELECT commit_sha, history_status, checkpoint_status \
         FROM commit_ingest_ledger \
         WHERE repo_id = '{}'",
        esc_pg(repo_id),
    );
    let rows = relational.query_rows(&sql).await?;
    let mut out = HashSet::new();
    for row in rows {
        let Some(commit_sha) = row.get("commit_sha").and_then(Value::as_str) else {
            continue;
        };
        let entry = CommitIngestLedgerEntry {
            history_status: row
                .get("history_status")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            checkpoint_status: row
                .get("checkpoint_status")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            checkpoint_id: None,
        };
        if commit_is_fully_ingested(&entry) {
            out.insert(commit_sha.to_string());
        }
    }
    Ok(out)
}

async fn load_sync_state_value_for_repo(
    relational: &RelationalStorage,
    repo_id: &str,
    key: &str,
) -> Result<Option<String>> {
    let sql = format!(
        "SELECT state_value FROM sync_state WHERE repo_id = '{}' AND state_key = '{}' LIMIT 1",
        esc_pg(repo_id),
        esc_pg(key),
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .first()
        .and_then(|row| row.get("state_value"))
        .and_then(Value::as_str)
        .map(str::to_string))
}

async fn upsert_commit_ingest_ledger_row(
    relational: &RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
    history_status: &str,
    checkpoint_status: &str,
    checkpoint_id: Option<&str>,
    error_message: Option<&str>,
) -> Result<()> {
    let now_sql = sql_now(relational);
    let checkpoint_id_sql = checkpoint_id
        .map(|value| format!("'{}'", esc_pg(value)))
        .unwrap_or_else(|| "NULL".to_string());
    let error_sql = error_message
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("'{}'", esc_pg(value)))
        .unwrap_or_else(|| "NULL".to_string());
    let sql = format!(
        "INSERT INTO commit_ingest_ledger (
            repo_id, commit_sha, history_status, checkpoint_status, checkpoint_id, last_error, updated_at
         ) VALUES (
            '{repo_id}', '{commit_sha}', '{history_status}', '{checkpoint_status}', {checkpoint_id}, {last_error}, {now_sql}
         )
         ON CONFLICT (repo_id, commit_sha) DO UPDATE SET
            history_status = EXCLUDED.history_status,
            checkpoint_status = EXCLUDED.checkpoint_status,
            checkpoint_id = EXCLUDED.checkpoint_id,
            last_error = EXCLUDED.last_error,
            updated_at = {now_sql}",
        repo_id = esc_pg(repo_id),
        commit_sha = esc_pg(commit_sha),
        history_status = esc_pg(history_status),
        checkpoint_status = esc_pg(checkpoint_status),
        checkpoint_id = checkpoint_id_sql,
        last_error = error_sql,
        now_sql = now_sql,
    );
    relational.exec(&sql).await
}

fn list_commit_range(repo_root: &Path, range: &str) -> Result<Vec<String>> {
    let output = run_git(repo_root, &["rev-list", "--reverse", range])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}
