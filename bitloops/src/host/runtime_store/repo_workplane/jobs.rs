//! Enqueue and dedupe of capability workplane jobs.

use anyhow::{Context, Result};
use rusqlite::params;
use uuid::Uuid;

use super::dedupe::load_deduped_job;
use super::types::{
    CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneJobInsert, WorkplaneJobStatus,
};
use super::util::{sql_i64, unix_timestamp_now};
use crate::host::runtime_store::types::RepoSqliteRuntimeStore;

impl RepoSqliteRuntimeStore {
    pub fn enqueue_capability_workplane_jobs(
        &self,
        capability_id: &str,
        jobs: Vec<CapabilityWorkplaneJobInsert>,
    ) -> Result<CapabilityWorkplaneEnqueueResult> {
        if jobs.is_empty() {
            return Ok(CapabilityWorkplaneEnqueueResult::default());
        }

        let sqlite = self.connect_repo_sqlite()?;
        sqlite.with_write_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting capability workplane enqueue transaction")?;
            let result = (|| {
                let now = unix_timestamp_now();
                let mut inserted_jobs = 0u64;
                let mut updated_jobs = 0u64;
                for job in jobs {
                    if let Some(existing) = load_deduped_job(
                        conn,
                        &self.repo_id,
                        capability_id,
                        &job.mailbox_name,
                        job.init_session_id.as_deref(),
                        job.dedupe_key.as_deref(),
                    )? {
                        if existing.status == WorkplaneJobStatus::Pending {
                            conn.execute(
                                "UPDATE capability_workplane_jobs
                                 SET payload = ?1, updated_at_unix = ?2, available_at_unix = ?3, last_error = NULL
                                 WHERE job_id = ?4",
                                params![
                                    job.payload.to_string(),
                                    sql_i64(now)?,
                                    sql_i64(now)?,
                                    existing.job_id,
                                ],
                            )
                            .with_context(|| {
                                format!(
                                    "refreshing pending capability workplane job `{}`",
                                    existing.job_id
                                )
                            })?;
                        }
                        updated_jobs += 1;
                        continue;
                    }

                    let job_id = format!("workplane-job-{}", Uuid::new_v4());
                    conn.execute(
                        "INSERT INTO capability_workplane_jobs (
                            job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                            init_session_id, dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                            started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                            lease_expires_at_unix, last_error
                         ) VALUES (
                            ?1, ?2, ?3, ?4, ?5, ?6,
                            ?7, ?8, ?9, ?10, 0, ?11, ?12,
                            NULL, ?13, NULL, NULL,
                            NULL, NULL
                         )",
                        params![
                            &job_id,
                            &self.repo_id,
                            self.repo_root.to_string_lossy().to_string(),
                            self.config_root.to_string_lossy().to_string(),
                            capability_id,
                            &job.mailbox_name,
                            job.init_session_id.as_deref(),
                            job.dedupe_key.as_deref(),
                            job.payload.to_string(),
                            WorkplaneJobStatus::Pending.as_str(),
                            sql_i64(now)?,
                            sql_i64(now)?,
                            sql_i64(now)?,
                        ],
                    )
                    .with_context(|| {
                        format!(
                            "inserting capability workplane job `{job_id}` for mailbox `{}`",
                            job.mailbox_name
                        )
                    })?;
                    inserted_jobs += 1;
                }
                Ok(CapabilityWorkplaneEnqueueResult {
                    inserted_jobs,
                    updated_jobs,
                })
            })();

            match result {
                Ok(result) => {
                    conn.execute_batch("COMMIT;")
                        .context("committing capability workplane enqueue transaction")?;
                    Ok(result)
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(err)
                }
            }
        })
    }
}
