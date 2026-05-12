use anyhow::Result;
use rusqlite::params;

use crate::daemon::types::unix_timestamp_now;
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, SemanticMailboxItemStatus, WorkplaneJobStatus,
};

use super::super::{WORKPLANE_TERMINAL_RETENTION_SECS, WORKPLANE_TERMINAL_ROW_LIMIT};
use super::sql::sql_i64;

pub(crate) fn compact_and_prune_workplane_jobs(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<()> {
    workplane_store.with_connection(|conn| {
        prune_terminal_workplane_jobs(conn)?;
        Ok(())
    })
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
    workplane_store.with_connection(|conn| {
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
    workplane_store.with_connection(|conn| {
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
    workplane_store.with_connection(|conn| {
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
    workplane_store.with_connection(|conn| {
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
    workplane_store.with_connection(|conn| {
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
    workplane_store.with_connection(|conn| {
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
