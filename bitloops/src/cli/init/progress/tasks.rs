use anyhow::Result;
use serde_json::Value;

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::devql_transport::SlimCliRepoScope;
use crate::host::relational_store::{DefaultRelationalStore, RelationalStore};
use crate::host::runtime_store::RepoSqliteRuntimeStore;

use super::EmbeddingQueueSnapshot;

const CURRENT_CODE_EMBEDDINGS_TABLE: &str = "symbol_embeddings_current";
const CURRENT_SUMMARY_SEMANTICS_TABLE: &str = "symbol_semantics_current";

pub(crate) async fn current_embedding_queue_snapshot(
    repo_root: &std::path::Path,
    repo_id: &str,
) -> Result<Option<EmbeddingQueueSnapshot>> {
    let status = RepoSqliteRuntimeStore::open(repo_root)
        .ok()
        .and_then(|store| {
            store
                .load_capability_workplane_mailbox_status(
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    [SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX],
                )
                .ok()
        })
        .and_then(|mut status_by_mailbox| {
            status_by_mailbox.remove(SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
        })
        .unwrap_or_default();
    let coverage = current_code_embedding_progress(repo_root, repo_id).await?;

    Ok(Some(EmbeddingQueueSnapshot {
        pending: status.pending_jobs,
        running: status.running_jobs,
        failed: status.failed_jobs,
        completed: coverage.completed,
        total: coverage.total,
    }))
}

pub(crate) async fn current_code_embedding_artefact_count(
    repo_root: &std::path::Path,
    repo_id: &str,
) -> Result<u64> {
    Ok(current_code_embedding_progress(repo_root, repo_id)
        .await?
        .completed)
}

pub(crate) async fn current_summary_queue_snapshot(
    repo_root: &std::path::Path,
    repo_id: &str,
) -> Result<Option<EmbeddingQueueSnapshot>> {
    let status = RepoSqliteRuntimeStore::open(repo_root)
        .ok()
        .and_then(|store| {
            store
                .load_capability_workplane_mailbox_status(
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    [SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX],
                )
                .ok()
        })
        .and_then(|mut status_by_mailbox| {
            status_by_mailbox.remove(SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
        })
        .unwrap_or_default();
    let coverage = current_summary_progress(repo_root, repo_id).await?;

    Ok(Some(EmbeddingQueueSnapshot {
        pending: status.pending_jobs,
        running: status.running_jobs,
        failed: status.failed_jobs,
        completed: coverage.completed,
        total: coverage.total,
    }))
}

pub(crate) async fn refresh_init_progress_task(
    scope: &SlimCliRepoScope,
    current: &crate::cli::devql::graphql::TaskGraphqlRecord,
) -> Result<Option<crate::cli::devql::graphql::TaskGraphqlRecord>> {
    if let Some(task) =
        crate::cli::devql::graphql::query_task_via_graphql(scope, current.task_id.as_str()).await?
    {
        return Ok(Some(task));
    }

    let kind = if current.is_sync() {
        Some("sync")
    } else if current.is_ingest() {
        Some("ingest")
    } else if current.is_embeddings_bootstrap() {
        Some("embeddings_bootstrap")
    } else {
        None
    };
    let Some(kind) = kind else {
        return Ok(None);
    };

    let tasks =
        crate::cli::devql::graphql::list_tasks_via_graphql(scope, Some(kind), None, Some(16))
            .await?;
    Ok(tasks
        .into_iter()
        .find(|task| task.task_id == current.task_id))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CoverageProgress {
    completed: u64,
    total: u64,
}

async fn current_code_embedding_progress(
    repo_root: &std::path::Path,
    repo_id: &str,
) -> Result<CoverageProgress> {
    let relational =
        DefaultRelationalStore::open_primary_for_repo_root_preferring_bound_config(repo_root)?;
    let total = count_eligible_current_artefacts(&relational, repo_id).await?;
    let completed = count_current_code_embedding_artefacts(&relational, repo_id).await?;
    Ok(CoverageProgress {
        completed: completed.min(total),
        total,
    })
}

async fn current_summary_progress(
    repo_root: &std::path::Path,
    repo_id: &str,
) -> Result<CoverageProgress> {
    let relational =
        DefaultRelationalStore::open_primary_for_repo_root_preferring_bound_config(repo_root)?;
    let total = count_eligible_current_artefacts(&relational, repo_id).await?;
    let completed = count_current_model_backed_summary_artefacts(&relational, repo_id).await?;
    Ok(CoverageProgress {
        completed: completed.min(total),
        total,
    })
}

async fn count_eligible_current_artefacts(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<u64> {
    query_progress_count(
        relational,
        &format!(
            "SELECT COUNT(DISTINCT a.artefact_id) AS total \
             FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             WHERE a.repo_id = '{}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import'",
            escape_sql_string(repo_id),
        ),
    )
    .await
}

async fn count_current_code_embedding_artefacts(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<u64> {
    query_progress_count(relational, &current_code_embedding_progress_sql(repo_id)).await
}

async fn count_current_model_backed_summary_artefacts(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<u64> {
    query_progress_count(relational, &current_summary_progress_sql(repo_id)).await
}

fn current_code_embedding_progress_sql(repo_id: &str) -> String {
    format!(
        "SELECT COUNT(DISTINCT a.artefact_id) AS total \
         FROM artefacts_current a \
         JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
         JOIN {CURRENT_CODE_EMBEDDINGS_TABLE} e ON e.repo_id = a.repo_id AND e.artefact_id = a.artefact_id \
         WHERE a.repo_id = '{}' \
           AND cfs.analysis_mode = 'code' \
           AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
           AND LOWER(COALESCE(e.representation_kind, 'code')) = 'code'",
        escape_sql_string(repo_id),
    )
}

fn current_summary_progress_sql(repo_id: &str) -> String {
    format!(
        "SELECT COUNT(DISTINCT a.artefact_id) AS total \
         FROM artefacts_current a \
         JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
         JOIN {CURRENT_SUMMARY_SEMANTICS_TABLE} s ON s.repo_id = a.repo_id AND s.artefact_id = a.artefact_id \
         WHERE a.repo_id = '{}' \
           AND cfs.analysis_mode = 'code' \
           AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
           AND ( \
                (s.llm_summary IS NOT NULL AND TRIM(s.llm_summary) <> '') \
                OR (s.source_model IS NOT NULL AND TRIM(s.source_model) <> '') \
           )",
        escape_sql_string(repo_id),
    )
}

async fn query_progress_count(relational: &DefaultRelationalStore, sql: &str) -> Result<u64> {
    let rows = match relational.query_rows_primary_blocking(sql) {
        Ok(rows) => rows,
        Err(err) if missing_progress_table(&err) => return Ok(0),
        Err(err) => return Err(err),
    };

    Ok(rows
        .first()
        .and_then(|row| row.get("total"))
        .and_then(value_as_u64)
        .unwrap_or_default())
}

fn value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
}

fn missing_progress_table(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("no such table:") || message.contains("does not exist")
}

fn escape_sql_string(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_embedding_progress_sql_uses_current_projection() {
        let sql = current_code_embedding_progress_sql("repo-1");

        assert!(sql.contains("JOIN symbol_embeddings_current e"));
        assert!(!sql.contains("JOIN symbol_embeddings e"));
    }

    #[test]
    fn summary_progress_sql_uses_current_projection() {
        let sql = current_summary_progress_sql("repo-1");

        assert!(sql.contains("JOIN symbol_semantics_current s"));
        assert!(!sql.contains("JOIN symbol_semantics s"));
    }
}
