//! Enqueue and dedupe of capability workplane jobs.

use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter, types::Value as SqlValue};
use uuid::Uuid;

use super::dedupe::load_deduped_job;
use super::types::{
    CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneJobInsert, WorkplaneJobQuery,
    WorkplaneJobRecord, WorkplaneJobStatus,
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
        sqlite.with_connection(|conn| {
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

    pub fn list_capability_workplane_jobs(
        &self,
        query: WorkplaneJobQuery,
    ) -> Result<Vec<WorkplaneJobRecord>> {
        let sqlite = self.connect_repo_sqlite()?;
        sqlite.with_connection(|conn| {
            let mut sql = String::from(
                "SELECT job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                        init_session_id, dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                        started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                        lease_expires_at_unix, last_error
                 FROM capability_workplane_jobs
                 WHERE repo_id = ?1",
            );
            let mut params = vec![SqlValue::Text(self.repo_id.clone())];
            let mut bind_index = 2usize;

            if let Some(capability_id) = query.capability_id {
                sql.push_str(&format!(" AND capability_id = ?{bind_index}"));
                params.push(SqlValue::Text(capability_id));
                bind_index += 1;
            }
            if let Some(mailbox_name) = query.mailbox_name {
                sql.push_str(&format!(" AND mailbox_name = ?{bind_index}"));
                params.push(SqlValue::Text(mailbox_name));
                bind_index += 1;
            }
            if !query.statuses.is_empty() {
                let placeholders = std::iter::repeat_n("?", query.statuses.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                sql.push_str(&format!(" AND status IN ({placeholders})"));
                for status in &query.statuses {
                    params.push(SqlValue::Text(status.as_str().to_string()));
                    bind_index += 1;
                }
            }
            sql.push_str(" ORDER BY updated_at_unix DESC, submitted_at_unix DESC");
            if let Some(limit) = query.limit {
                sql.push_str(&format!(" LIMIT ?{bind_index}"));
                let limit_i64 =
                    i64::try_from(limit).context("converting workplane job query limit to sqlite i64")?;
                params.push(SqlValue::Integer(limit_i64));
            }

            let mut stmt = conn.prepare(&sql).context("preparing capability workplane jobs query")?;
            let rows = stmt
                .query_map(params_from_iter(params.iter()), |row| {
                    let payload_raw = row.get::<_, String>(8)?;
                    let payload = serde_json::from_str(&payload_raw).unwrap_or(serde_json::Value::Null);
                    Ok(WorkplaneJobRecord {
                        job_id: row.get(0)?,
                        repo_id: row.get(1)?,
                        repo_root: std::path::PathBuf::from(row.get::<_, String>(2)?),
                        config_root: std::path::PathBuf::from(row.get::<_, String>(3)?),
                        capability_id: row.get(4)?,
                        mailbox_name: row.get(5)?,
                        init_session_id: row.get(6)?,
                        dedupe_key: row.get(7)?,
                        payload,
                        status: WorkplaneJobStatus::parse(&row.get::<_, String>(9)?),
                        attempts: row.get(10)?,
                        available_at_unix: u64::try_from(row.get::<_, i64>(11)?).unwrap_or_default(),
                        submitted_at_unix: u64::try_from(row.get::<_, i64>(12)?).unwrap_or_default(),
                        started_at_unix: row
                            .get::<_, Option<i64>>(13)?
                            .map(|value| u64::try_from(value).unwrap_or_default()),
                        updated_at_unix: u64::try_from(row.get::<_, i64>(14)?).unwrap_or_default(),
                        completed_at_unix: row
                            .get::<_, Option<i64>>(15)?
                            .map(|value| u64::try_from(value).unwrap_or_default()),
                        lease_owner: row.get(16)?,
                        lease_expires_at_unix: row
                            .get::<_, Option<i64>>(17)?
                            .map(|value| u64::try_from(value).unwrap_or_default()),
                        last_error: row.get(18)?,
                    })
                })
                .context("querying capability workplane jobs")?;

            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }
}
