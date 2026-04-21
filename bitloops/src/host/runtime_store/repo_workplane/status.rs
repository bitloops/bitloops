//! Aggregated mailbox status projection across capability and semantic queues.

use anyhow::Result;
use rusqlite::params;
use std::collections::BTreeMap;

use super::types::{SemanticMailboxItemStatus, WorkplaneCursorRunStatus, WorkplaneJobStatus};
use crate::host::runtime_store::types::RepoSqliteRuntimeStore;

const SEMANTIC_SUMMARY_REFRESH_MAILBOX_NAME: &str = "semantic_clones.summary_refresh";
const SEMANTIC_CODE_EMBEDDING_MAILBOX_NAME: &str = "semantic_clones.embedding.code";
const SEMANTIC_IDENTITY_EMBEDDING_MAILBOX_NAME: &str = "semantic_clones.embedding.identity";
const SEMANTIC_SUMMARY_EMBEDDING_MAILBOX_NAME: &str = "semantic_clones.embedding.summary";

impl RepoSqliteRuntimeStore {
    pub fn load_capability_workplane_mailbox_status<'a>(
        &self,
        capability_id: &str,
        mailbox_names: impl IntoIterator<Item = &'a str>,
    ) -> Result<BTreeMap<String, crate::host::capability_host::gateways::CapabilityMailboxStatus>>
    {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite.with_connection(|conn| {
            let mut status_by_mailbox = mailbox_names
                .into_iter()
                .map(|mailbox_name| {
                    (
                        mailbox_name.to_string(),
                        crate::host::capability_host::gateways::CapabilityMailboxStatus::default(),
                    )
                })
                .collect::<BTreeMap<_, _>>();

            {
                let mut stmt = conn.prepare(
                    "SELECT mailbox_name, active
                     FROM capability_workplane_mailbox_intents
                     WHERE repo_id = ?1 AND capability_id = ?2",
                )?;
                let rows = stmt.query_map(params![&self.repo_id, capability_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?;
                for row in rows {
                    let (mailbox_name, active) = row?;
                    let Some(entry) = status_by_mailbox.get_mut(&mailbox_name) else {
                        continue;
                    };
                    entry.intent_active = active != 0;
                }
            }

            {
                let mut stmt = conn.prepare(
                    "SELECT mailbox_name, status, COUNT(*)
                     FROM capability_workplane_jobs
                     WHERE repo_id = ?1 AND capability_id = ?2
                     GROUP BY mailbox_name, status",
                )?;
                let rows = stmt.query_map(params![&self.repo_id, capability_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?;
                for row in rows {
                    let (mailbox_name, status, count) = row?;
                    let Some(entry) = status_by_mailbox.get_mut(&mailbox_name) else {
                        continue;
                    };
                    let count = u64::try_from(count).unwrap_or_default();
                    match WorkplaneJobStatus::parse(&status) {
                        WorkplaneJobStatus::Pending => entry.pending_jobs += count,
                        WorkplaneJobStatus::Running => entry.running_jobs += count,
                        WorkplaneJobStatus::Completed => entry.completed_recent_jobs += count,
                        WorkplaneJobStatus::Failed => entry.failed_jobs += count,
                    }
                }
            }

            {
                let mut stmt = conn.prepare(
                    "SELECT status, COUNT(*)
                     FROM semantic_summary_mailbox_items
                     WHERE repo_id = ?1
                     GROUP BY status",
                )?;
                let rows = stmt.query_map(params![&self.repo_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?;
                for row in rows {
                    let (status, count) = row?;
                    let Some(entry) =
                        status_by_mailbox.get_mut(SEMANTIC_SUMMARY_REFRESH_MAILBOX_NAME)
                    else {
                        continue;
                    };
                    let count = u64::try_from(count).unwrap_or_default();
                    match SemanticMailboxItemStatus::parse(&status) {
                        SemanticMailboxItemStatus::Pending => entry.pending_jobs += count,
                        SemanticMailboxItemStatus::Leased => entry.running_jobs += count,
                        SemanticMailboxItemStatus::Failed => entry.failed_jobs += count,
                    }
                }
            }

            {
                let mut stmt = conn.prepare(
                    "SELECT representation_kind, status, COUNT(*)
                     FROM semantic_embedding_mailbox_items
                     WHERE repo_id = ?1
                     GROUP BY representation_kind, status",
                )?;
                let rows = stmt.query_map(params![&self.repo_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?;
                for row in rows {
                    let (representation_kind, status, count) = row?;
                    let mailbox_name = match representation_kind.as_str() {
                        "summary" => SEMANTIC_SUMMARY_EMBEDDING_MAILBOX_NAME,
                        "identity" | "locator" => SEMANTIC_IDENTITY_EMBEDDING_MAILBOX_NAME,
                        _ => SEMANTIC_CODE_EMBEDDING_MAILBOX_NAME,
                    };
                    let Some(entry) = status_by_mailbox.get_mut(mailbox_name) else {
                        continue;
                    };
                    let count = u64::try_from(count).unwrap_or_default();
                    match SemanticMailboxItemStatus::parse(&status) {
                        SemanticMailboxItemStatus::Pending => entry.pending_jobs += count,
                        SemanticMailboxItemStatus::Leased => entry.running_jobs += count,
                        SemanticMailboxItemStatus::Failed => entry.failed_jobs += count,
                    }
                }
            }

            {
                let mut stmt = conn.prepare(
                    "SELECT mailbox_name, status, COUNT(*)
                     FROM capability_workplane_cursor_runs
                     WHERE repo_id = ?1 AND capability_id = ?2
                     GROUP BY mailbox_name, status",
                )?;
                let rows = stmt.query_map(params![&self.repo_id, capability_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?;
                for row in rows {
                    let (mailbox_name, status, count) = row?;
                    let Some(entry) = status_by_mailbox.get_mut(&mailbox_name) else {
                        continue;
                    };
                    let count = u64::try_from(count).unwrap_or_default();
                    match WorkplaneCursorRunStatus::parse(&status) {
                        WorkplaneCursorRunStatus::Queued => entry.pending_cursor_runs += count,
                        WorkplaneCursorRunStatus::Running => entry.running_cursor_runs += count,
                        WorkplaneCursorRunStatus::Completed => {
                            entry.completed_recent_cursor_runs += count
                        }
                        WorkplaneCursorRunStatus::Failed => entry.failed_cursor_runs += count,
                        WorkplaneCursorRunStatus::Cancelled => {}
                    }
                }
            }

            Ok(status_by_mailbox)
        })
    }
}
