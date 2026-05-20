use std::path::PathBuf;

use anyhow::Result;
use rusqlite::params;

use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
    ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
};
use crate::daemon::types::unix_timestamp_now;
use crate::host::inference::InferenceGateway;
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, SemanticMailboxItemStatus, WorkplaneJobStatus,
};

use super::super::{WORKPLANE_TERMINAL_RETENTION_SECS, WORKPLANE_TERMINAL_ROW_LIMIT};
use super::sql::repo_identity_from_runtime_metadata;
use super::sql::sql_i64;

pub(crate) fn compact_and_prune_workplane_jobs(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<()> {
    let unconfigured_role_adjudication_repos =
        unconfigured_architecture_role_adjudication_repos(workplane_store)?;
    workplane_store.with_write_connection(|conn| {
        complete_unconfigured_architecture_role_adjudication_jobs(
            conn,
            &unconfigured_role_adjudication_repos,
        )?;
        prune_terminal_workplane_jobs(conn)?;
        Ok(())
    })
}

#[derive(Debug, Clone)]
struct PendingArchitectureRoleAdjudicationRepo {
    repo_id: String,
    repo_root: PathBuf,
    repo_root_text: String,
}

fn unconfigured_architecture_role_adjudication_repos(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<Vec<PendingArchitectureRoleAdjudicationRepo>> {
    let candidates = load_pending_architecture_role_adjudication_repos(workplane_store)?;
    let mut unconfigured = Vec::new();
    for candidate in candidates {
        let repo = repo_identity_from_runtime_metadata(&candidate.repo_root, &candidate.repo_id);
        let host = crate::host::devql::build_capability_host(&candidate.repo_root, repo)?;
        let inference = host.inference_for_capability(ARCHITECTURE_GRAPH_CAPABILITY_ID);
        if !inference.has_slot(ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT) {
            unconfigured.push(candidate);
        }
    }
    Ok(unconfigured)
}

fn load_pending_architecture_role_adjudication_repos(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<Vec<PendingArchitectureRoleAdjudicationRepo>> {
    workplane_store.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT repo_id, repo_root
             FROM capability_workplane_jobs
             WHERE capability_id = ?1
               AND mailbox_name = ?2
               AND status = ?3",
        )?;
        let rows = stmt.query_map(
            params![
                ARCHITECTURE_GRAPH_CAPABILITY_ID,
                ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
                WorkplaneJobStatus::Pending.as_str(),
            ],
            |row| {
                let repo_root_text = row.get::<_, String>(1)?;
                Ok(PendingArchitectureRoleAdjudicationRepo {
                    repo_id: row.get::<_, String>(0)?,
                    repo_root: PathBuf::from(&repo_root_text),
                    repo_root_text,
                })
            },
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(anyhow::Error::from)
    })
}

fn complete_unconfigured_architecture_role_adjudication_jobs(
    conn: &rusqlite::Connection,
    unconfigured_repos: &[PendingArchitectureRoleAdjudicationRepo],
) -> Result<()> {
    let now = unix_timestamp_now();
    for repo in unconfigured_repos {
        conn.execute(
            "UPDATE capability_workplane_jobs
             SET status = ?1,
                 updated_at_unix = ?2,
                 completed_at_unix = ?3,
                 last_error = ?4,
                 lease_owner = NULL,
                 lease_expires_at_unix = NULL
             WHERE repo_id = ?5
               AND repo_root = ?6
               AND capability_id = ?7
               AND mailbox_name = ?8
               AND status = ?9",
            params![
                WorkplaneJobStatus::Completed.as_str(),
                sql_i64(now)?,
                sql_i64(now)?,
                "skipped: structured-generation slot `role_adjudication` is not configured",
                repo.repo_id.as_str(),
                repo.repo_root_text.as_str(),
                ARCHITECTURE_GRAPH_CAPABILITY_ID,
                ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
                WorkplaneJobStatus::Pending.as_str(),
            ],
        )?;
    }
    Ok(())
}

fn prune_terminal_workplane_jobs(conn: &rusqlite::Connection) -> Result<()> {
    let cutoff = unix_timestamp_now().saturating_sub(WORKPLANE_TERMINAL_RETENTION_SECS);
    let mut stmt = conn.prepare(
        "SELECT repo_id, capability_id, mailbox_name, COUNT(*)
         FROM capability_workplane_jobs
         WHERE status IN (?1, ?2)
         GROUP BY repo_id, capability_id, mailbox_name
         HAVING COUNT(*) > ?3",
    )?;
    let rows = stmt.query_map(
        params![
            WorkplaneJobStatus::Completed.as_str(),
            WorkplaneJobStatus::Failed.as_str(),
            sql_i64(WORKPLANE_TERMINAL_ROW_LIMIT)?,
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )?;
    for row in rows {
        let (repo_id, capability_id, mailbox_name) = row?;
        conn.execute(
            "DELETE FROM capability_workplane_jobs
             WHERE repo_id = ?1
               AND capability_id = ?2
               AND mailbox_name = ?3
               AND status IN (?4, ?5)
               AND COALESCE(completed_at_unix, updated_at_unix) <= ?6",
            params![
                repo_id,
                capability_id,
                mailbox_name,
                WorkplaneJobStatus::Completed.as_str(),
                WorkplaneJobStatus::Failed.as_str(),
                sql_i64(cutoff)?,
            ],
        )?;
    }
    Ok(())
}

pub(crate) fn recover_expired_semantic_inbox_leases(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<u64> {
    let now = unix_timestamp_now();
    workplane_store.with_write_connection(|conn| {
        let summary = conn.execute(
            "UPDATE semantic_summary_mailbox_items
             SET status = ?1,
                 leased_at_unix = NULL,
                 lease_expires_at_unix = NULL,
                 lease_token = NULL,
                 updated_at_unix = ?2
             WHERE status = ?3
               AND lease_expires_at_unix IS NOT NULL
               AND lease_expires_at_unix <= ?4",
            params![
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
                SemanticMailboxItemStatus::Leased.as_str(),
                sql_i64(now)?,
            ],
        )?;
        let embedding = conn.execute(
            "UPDATE semantic_embedding_mailbox_items
             SET status = ?1,
                 leased_at_unix = NULL,
                 lease_expires_at_unix = NULL,
                 lease_token = NULL,
                 updated_at_unix = ?2
             WHERE status = ?3
               AND lease_expires_at_unix IS NOT NULL
               AND lease_expires_at_unix <= ?4",
            params![
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
                SemanticMailboxItemStatus::Leased.as_str(),
                sql_i64(now)?,
            ],
        )?;
        Ok(u64::try_from(summary + embedding).unwrap_or_default())
    })
}

pub(crate) fn requeue_leased_semantic_inbox_items(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<u64> {
    let now = unix_timestamp_now();
    workplane_store.with_write_connection(|conn| {
        let summary = conn.execute(
            "UPDATE semantic_summary_mailbox_items
             SET status = ?1,
                 leased_at_unix = NULL,
                 lease_expires_at_unix = NULL,
                 lease_token = NULL,
                 updated_at_unix = ?2,
                 last_error = NULL
             WHERE status = ?3",
            params![
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
                SemanticMailboxItemStatus::Leased.as_str(),
            ],
        )?;
        let embedding = conn.execute(
            "UPDATE semantic_embedding_mailbox_items
             SET status = ?1,
                 leased_at_unix = NULL,
                 lease_expires_at_unix = NULL,
                 lease_token = NULL,
                 updated_at_unix = ?2,
                 last_error = NULL
             WHERE status = ?3",
            params![
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
                SemanticMailboxItemStatus::Leased.as_str(),
            ],
        )?;
        Ok(u64::try_from(summary + embedding).unwrap_or_default())
    })
}

pub(crate) fn requeue_running_workplane_jobs(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<u64> {
    let now = unix_timestamp_now();
    workplane_store.with_write_connection(|conn| {
        let recovered = conn.execute(
            "UPDATE capability_workplane_jobs
             SET status = ?1,
                 started_at_unix = NULL,
                 updated_at_unix = ?2,
                 lease_owner = NULL,
                 lease_expires_at_unix = NULL
             WHERE status = ?3",
            params![
                WorkplaneJobStatus::Pending.as_str(),
                sql_i64(now)?,
                WorkplaneJobStatus::Running.as_str(),
            ],
        )?;
        Ok(u64::try_from(recovered).unwrap_or_default())
    })
}

pub(crate) fn prune_failed_semantic_inbox_items(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<()> {
    let cutoff = unix_timestamp_now().saturating_sub(WORKPLANE_TERMINAL_RETENTION_SECS);
    workplane_store.with_write_connection(|conn| {
        conn.execute(
            "DELETE FROM semantic_summary_mailbox_items
             WHERE status = ?1
               AND updated_at_unix <= ?2",
            params![SemanticMailboxItemStatus::Failed.as_str(), sql_i64(cutoff)?,],
        )?;
        conn.execute(
            "DELETE FROM semantic_embedding_mailbox_items
             WHERE status = ?1
               AND updated_at_unix <= ?2",
            params![SemanticMailboxItemStatus::Failed.as_str(), sql_i64(cutoff)?,],
        )?;
        Ok(())
    })
}

pub(crate) fn retry_failed_semantic_inbox_items(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<u64> {
    let now = unix_timestamp_now();
    workplane_store.with_write_connection(|conn| {
        let summary = conn.execute(
            "UPDATE semantic_summary_mailbox_items
             SET status = ?1,
                 leased_at_unix = NULL,
                 lease_expires_at_unix = NULL,
                 lease_token = NULL,
                 updated_at_unix = ?2,
                 last_error = NULL
             WHERE status = ?3",
            params![
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
                SemanticMailboxItemStatus::Failed.as_str(),
            ],
        )?;
        let embedding = conn.execute(
            "UPDATE semantic_embedding_mailbox_items
             SET status = ?1,
                 leased_at_unix = NULL,
                 lease_expires_at_unix = NULL,
                 lease_token = NULL,
                 updated_at_unix = ?2,
                 last_error = NULL
             WHERE status = ?3",
            params![
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
                SemanticMailboxItemStatus::Failed.as_str(),
            ],
        )?;
        Ok(u64::try_from(summary + embedding).unwrap_or_default())
    })
}

pub(crate) fn retry_failed_workplane_jobs(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<u64> {
    workplane_store.with_write_connection(|conn| {
        conn.execute(
            "UPDATE capability_workplane_jobs
                 SET status = ?1,
                     started_at_unix = NULL,
                     updated_at_unix = ?2,
                     completed_at_unix = NULL,
                     last_error = NULL
                 WHERE status = ?3",
            params![
                WorkplaneJobStatus::Pending.as_str(),
                sql_i64(unix_timestamp_now())?,
                WorkplaneJobStatus::Failed.as_str(),
            ],
        )
        .map(|count| u64::try_from(count).unwrap_or_default())
        .map_err(anyhow::Error::from)
    })
}
