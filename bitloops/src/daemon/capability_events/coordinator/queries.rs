use anyhow::{Result, anyhow};
use rusqlite::{OptionalExtension, params};

use crate::daemon::capability_events::queue::{load_run_by_id, load_runs};
use crate::daemon::types::{
    CapabilityEventQueueState, CapabilityEventQueueStatus, CapabilityEventRunRecord,
    CapabilityEventRunStatus,
};

use super::types::CapabilityEventCoordinator;

impl CapabilityEventCoordinator {
    pub(crate) fn clear_queued_runs_for_repo(&self, repo_id: &str) -> Result<u64> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow!("current-state consumer lock poisoned"))?;
        let deleted = self.runtime_store.with_connection(|conn| {
            conn.execute(
                "DELETE FROM capability_workplane_cursor_runs WHERE repo_id = ?1 AND status = ?2",
                params![repo_id, CapabilityEventRunStatus::Queued.to_string()],
            )
            .map(|count| u64::try_from(count).unwrap_or_default())
            .map_err(anyhow::Error::from)
        })?;
        self.notify.notify_waiters();
        Ok(deleted)
    }

    pub(crate) fn snapshot(&self, repo_id: Option<&str>) -> Result<CapabilityEventQueueStatus> {
        self.runtime_store.with_connection(|conn| {
            let pending_runs = count_runs_with_status(conn, CapabilityEventRunStatus::Queued)?;
            let running_runs = count_runs_with_status(conn, CapabilityEventRunStatus::Running)?;
            let failed_runs = count_runs_with_status(conn, CapabilityEventRunStatus::Failed)?;
            let completed_recent_runs =
                count_runs_with_status(conn, CapabilityEventRunStatus::Completed)?;
            let queue_activity = load_queue_activity(conn)?;
            let current_repo_run = repo_id
                .map(|repo_id| load_current_repo_run(conn, repo_id))
                .transpose()?
                .flatten();

            Ok(CapabilityEventQueueStatus {
                state: CapabilityEventQueueState {
                    version: 1,
                    pending_runs,
                    running_runs,
                    failed_runs,
                    completed_recent_runs,
                    last_action: queue_activity.last_action,
                    last_updated_unix: queue_activity.last_updated_unix,
                },
                persisted: true,
                current_repo_run,
            })
        })
    }

    #[allow(dead_code)]
    pub(crate) fn run(&self, run_id: &str) -> Result<Option<CapabilityEventRunRecord>> {
        self.runtime_store.with_connection(|conn| {
            load_run_by_id(conn, run_id).map(|record| record.map(|r| r.record))
        })
    }
}

fn count_runs_with_status(
    conn: &rusqlite::Connection,
    status: CapabilityEventRunStatus,
) -> Result<u64> {
    conn.query_row(
        "SELECT COUNT(*) FROM capability_workplane_cursor_runs WHERE status = ?1",
        params![status.to_string()],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| u64::try_from(value).unwrap_or_default())
    .map_err(anyhow::Error::from)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueueActivity {
    last_action: Option<String>,
    last_updated_unix: u64,
}

fn load_queue_activity(conn: &rusqlite::Connection) -> Result<QueueActivity> {
    conn.query_row(
        "SELECT status, updated_at_unix FROM capability_workplane_cursor_runs ORDER BY updated_at_unix DESC, submitted_at_unix DESC LIMIT 1",
        [],
        |row| {
            Ok(QueueActivity {
                last_action: Some(row.get(0)?),
                last_updated_unix: u64::try_from(row.get::<_, i64>(1)?).unwrap_or_default(),
            })
        },
    )
    .optional()
    .map(|row| {
        row.unwrap_or(QueueActivity {
            last_action: None,
            last_updated_unix: 0,
        })
    })
    .map_err(anyhow::Error::from)
}

fn load_current_repo_run(
    conn: &rusqlite::Connection,
    repo_id: &str,
) -> Result<Option<CapabilityEventRunRecord>> {
    if let Some(run) = load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, mailbox_name, capability_id, init_session_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM capability_workplane_cursor_runs WHERE repo_id = ?1 AND status = ?2 ORDER BY submitted_at_unix ASC LIMIT 1",
        params![repo_id, CapabilityEventRunStatus::Running.to_string()],
    )?
    .into_iter()
    .next()
    {
        return Ok(Some(run.record));
    }

    Ok(load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, mailbox_name, capability_id, init_session_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM capability_workplane_cursor_runs WHERE repo_id = ?1 AND status = ?2 ORDER BY submitted_at_unix ASC LIMIT 1",
        params![repo_id, CapabilityEventRunStatus::Queued.to_string()],
    )?
    .into_iter()
    .next()
    .map(|run| run.record))
}
