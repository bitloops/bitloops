use anyhow::{Context, Result};
use rusqlite::params;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::payload::prune_excluded_paths_from_payload;
use super::storage::{
    map_producer_spool_job_record_row, open_repo_runtime_sqlite_for_config_root, sql_i64,
    unix_timestamp_now,
};
use super::{
    CLAIM_BATCH_LIMIT, ProducerSpoolJobRecord, ProducerSpoolJobStatus, REQUEUE_BACKOFF_SECS,
};

pub(crate) fn recover_running_producer_spool_jobs(config_root: &Path) -> Result<u64> {
    let sqlite = open_repo_runtime_sqlite_for_config_root(config_root)?;
    sqlite.with_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting DevQL producer spool recovery transaction")?;
        let result = (|| {
            let now = unix_timestamp_now();
            let updated = conn
                .execute(
                    "UPDATE devql_producer_spool_jobs
                     SET status = ?1, available_at_unix = ?2, updated_at_unix = ?3
                     WHERE status = ?4",
                    params![
                        ProducerSpoolJobStatus::Pending.as_str(),
                        sql_i64(now)?,
                        sql_i64(now)?,
                        ProducerSpoolJobStatus::Running.as_str(),
                    ],
                )
                .context("recovering interrupted DevQL producer spool jobs")?;
            prune_excluded_pending_producer_spool_jobs(conn)?;
            Ok(u64::try_from(updated).unwrap_or_default())
        })();

        match result {
            Ok(updated) => {
                conn.execute_batch("COMMIT;")
                    .context("committing DevQL producer spool recovery transaction")?;
                Ok(updated)
            }
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
    })
}

pub(crate) fn claim_next_producer_spool_jobs(
    config_root: &Path,
) -> Result<Vec<ProducerSpoolJobRecord>> {
    let sqlite = open_repo_runtime_sqlite_for_config_root(config_root)?;
    sqlite.with_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting DevQL producer spool claim transaction")?;
        let result = (|| {
            prune_excluded_pending_producer_spool_jobs(conn)?;
            let now = unix_timestamp_now();
            let running_repo_ids = load_running_repo_ids(conn)?;
            let mut claimed_repo_ids = HashSet::new();
            let mut selected = Vec::new();
            let mut stmt = conn.prepare(
                "SELECT job_id, repo_id, repo_root, config_root, repo_name, repo_provider,
                        repo_organisation, repo_identity, dedupe_key, payload, status, attempts,
                        available_at_unix, submitted_at_unix, updated_at_unix, last_error
                 FROM devql_producer_spool_jobs
                 WHERE status = ?1 AND available_at_unix <= ?2
                 ORDER BY available_at_unix ASC, submitted_at_unix ASC, job_id ASC",
            )?;
            let rows = stmt.query_map(
                params![ProducerSpoolJobStatus::Pending.as_str(), sql_i64(now)?,],
                map_producer_spool_job_record_row,
            )?;
            for row in rows {
                let mut job = row?;
                if running_repo_ids.contains(&job.repo_id)
                    || claimed_repo_ids.contains(&job.repo_id)
                {
                    continue;
                }
                job.status = ProducerSpoolJobStatus::Running;
                job.attempts = job.attempts.saturating_add(1);
                job.updated_at_unix = now;
                selected.push(job.clone());
                claimed_repo_ids.insert(job.repo_id);
                if selected.len() >= CLAIM_BATCH_LIMIT {
                    break;
                }
            }

            for job in &selected {
                conn.execute(
                    "UPDATE devql_producer_spool_jobs
                     SET status = ?1, attempts = ?2, updated_at_unix = ?3, last_error = NULL
                     WHERE job_id = ?4",
                    params![
                        ProducerSpoolJobStatus::Running.as_str(),
                        i64::from(job.attempts),
                        sql_i64(now)?,
                        &job.job_id,
                    ],
                )
                .with_context(|| {
                    format!(
                        "marking DevQL producer spool job `{}` as running",
                        job.job_id
                    )
                })?;
            }

            Ok(selected)
        })();

        match result {
            Ok(selected) => {
                conn.execute_batch("COMMIT;")
                    .context("committing DevQL producer spool claim transaction")?;
                Ok(selected)
            }
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
    })
}

pub(crate) fn delete_producer_spool_job(config_root: &Path, job_id: &str) -> Result<()> {
    let sqlite = open_repo_runtime_sqlite_for_config_root(config_root)?;
    sqlite.with_connection(|conn| {
        conn.execute(
            "DELETE FROM devql_producer_spool_jobs WHERE job_id = ?1",
            params![job_id],
        )
        .with_context(|| format!("deleting completed DevQL producer spool job `{job_id}`"))?;
        Ok(())
    })
}

pub(crate) fn requeue_producer_spool_job(
    config_root: &Path,
    job_id: &str,
    err: &anyhow::Error,
) -> Result<()> {
    let sqlite = open_repo_runtime_sqlite_for_config_root(config_root)?;
    sqlite.with_connection(|conn| {
        let now = unix_timestamp_now();
        conn.execute(
            "UPDATE devql_producer_spool_jobs
             SET status = ?1,
                 available_at_unix = ?2,
                 updated_at_unix = ?3,
                 last_error = ?4
             WHERE job_id = ?5",
            params![
                ProducerSpoolJobStatus::Pending.as_str(),
                sql_i64(now.saturating_add(REQUEUE_BACKOFF_SECS))?,
                sql_i64(now)?,
                format!("{err:#}"),
                job_id,
            ],
        )
        .with_context(|| format!("requeueing DevQL producer spool job `{job_id}`"))?;
        Ok(())
    })
}

fn prune_excluded_pending_producer_spool_jobs(conn: &rusqlite::Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT job_id, repo_id, repo_root, config_root, repo_name, repo_provider,
                repo_organisation, repo_identity, dedupe_key, payload, status, attempts,
                available_at_unix, submitted_at_unix, updated_at_unix, last_error
         FROM devql_producer_spool_jobs
         WHERE status = ?1
         ORDER BY submitted_at_unix ASC, job_id ASC",
    )?;
    let rows = stmt.query_map(
        params![ProducerSpoolJobStatus::Pending.as_str()],
        map_producer_spool_job_record_row,
    )?;
    let mut matchers = HashMap::<PathBuf, crate::host::devql::RepoExclusionMatcher>::new();
    let now = unix_timestamp_now();
    for row in rows {
        let job = row?;
        let matcher = matchers
            .entry(job.repo_root.clone())
            .or_insert(crate::host::devql::load_repo_exclusion_matcher(
                &job.repo_root,
            )?)
            .clone();
        let Some(payload) = prune_excluded_paths_from_payload(job.payload.clone(), &matcher) else {
            conn.execute(
                "DELETE FROM devql_producer_spool_jobs WHERE job_id = ?1",
                params![&job.job_id],
            )
            .with_context(|| {
                format!(
                    "deleting excluded DevQL producer spool job `{}` during prune",
                    job.job_id
                )
            })?;
            continue;
        };
        if payload != job.payload {
            conn.execute(
                "UPDATE devql_producer_spool_jobs
                 SET payload = ?1, updated_at_unix = ?2, last_error = NULL
                 WHERE job_id = ?3",
                params![
                    serde_json::to_string(&payload)
                        .context("serialising pruned DevQL producer spool payload")?,
                    sql_i64(now)?,
                    &job.job_id,
                ],
            )
            .with_context(|| {
                format!(
                    "updating excluded DevQL producer spool job `{}` during prune",
                    job.job_id
                )
            })?;
        }
    }
    Ok(())
}

fn load_running_repo_ids(conn: &rusqlite::Connection) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT repo_id
         FROM devql_producer_spool_jobs
         WHERE status = ?1",
    )?;
    let rows = stmt.query_map(params![ProducerSpoolJobStatus::Running.as_str()], |row| {
        row.get::<_, String>(0)
    })?;
    let mut repo_ids = HashSet::new();
    for row in rows {
        repo_ids.insert(row?);
    }
    Ok(repo_ids)
}
