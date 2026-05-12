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
    CLAIM_BATCH_LIMIT, PostCommitDerivationClaimGuards, ProducerSpoolJobCounts,
    ProducerSpoolJobPayload, ProducerSpoolJobRecord, ProducerSpoolJobStatus,
    ProducerSpoolRunningTask, REQUEUE_BACKOFF_SECS,
    producer_spool_payload_conflicts_with_running_task,
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

#[cfg(test)]
pub(crate) fn claim_next_producer_spool_jobs(
    config_root: &Path,
) -> Result<Vec<ProducerSpoolJobRecord>> {
    claim_next_producer_spool_jobs_excluding(
        config_root,
        &HashSet::new(),
        &[],
        &PostCommitDerivationClaimGuards::default(),
    )
}

#[cfg(test)]
pub(crate) fn claim_next_producer_spool_jobs_excluding_repo_ids(
    config_root: &Path,
    blocked_repo_ids: &HashSet<String>,
) -> Result<Vec<ProducerSpoolJobRecord>> {
    claim_next_producer_spool_jobs_excluding(
        config_root,
        blocked_repo_ids,
        &[],
        &PostCommitDerivationClaimGuards::default(),
    )
}

pub(crate) fn claim_next_producer_spool_jobs_excluding(
    config_root: &Path,
    blocked_repo_ids: &HashSet<String>,
    running_tasks: &[ProducerSpoolRunningTask],
    post_commit_derivation_guards: &PostCommitDerivationClaimGuards,
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
            let mut pending = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            pending.sort_by_key(producer_spool_claim_sort_key);
            let mut abandoned_job_ids = Vec::new();
            for mut job in pending {
                if blocked_repo_ids.contains(&job.repo_id) {
                    continue;
                }
                if running_tasks.iter().any(|task| {
                    producer_spool_payload_conflicts_with_running_task(
                        &job.payload,
                        &job.repo_id,
                        task,
                    )
                }) {
                    continue;
                }
                if post_commit_derivation_is_abandoned(&job, post_commit_derivation_guards) {
                    abandoned_job_ids.push(job.job_id);
                    continue;
                }
                if post_commit_derivation_is_blocked(&job, post_commit_derivation_guards) {
                    continue;
                }
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

            for job_id in &abandoned_job_ids {
                conn.execute(
                    "DELETE FROM devql_producer_spool_jobs WHERE job_id = ?1",
                    params![job_id],
                )
                .with_context(|| {
                    format!(
                        "deleting abandoned DevQL post-commit derivation job `{}`",
                        job_id
                    )
                })?;
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

fn post_commit_derivation_is_blocked(
    job: &ProducerSpoolJobRecord,
    guards: &PostCommitDerivationClaimGuards,
) -> bool {
    let ProducerSpoolJobPayload::PostCommitDerivation { commit_sha, .. } = &job.payload else {
        return false;
    };
    guards
        .blocked
        .contains(&(job.repo_id.clone(), commit_sha.clone()))
}

fn post_commit_derivation_is_abandoned(
    job: &ProducerSpoolJobRecord,
    guards: &PostCommitDerivationClaimGuards,
) -> bool {
    let ProducerSpoolJobPayload::PostCommitDerivation { commit_sha, .. } = &job.payload else {
        return false;
    };
    guards
        .abandoned
        .contains(&(job.repo_id.clone(), commit_sha.clone()))
}

pub(crate) fn running_producer_spool_repo_ids(config_root: &Path) -> Result<HashSet<String>> {
    let sqlite = open_repo_runtime_sqlite_for_config_root(config_root)?;
    sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT repo_id
             FROM devql_producer_spool_jobs
             WHERE status = ?1",
        )?;
        let rows = stmt.query_map([ProducerSpoolJobStatus::Running.as_str()], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<HashSet<String>>>()
            .map_err(anyhow::Error::from)
    })
}

pub(crate) fn list_recent_producer_spool_jobs(
    config_root: &Path,
    repo_id: &str,
    limit: usize,
) -> Result<Vec<ProducerSpoolJobRecord>> {
    let sqlite = open_repo_runtime_sqlite_for_config_root(config_root)?;
    let limit = i64::try_from(limit.max(1)).unwrap_or(i64::MAX);
    sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT job_id, repo_id, repo_root, config_root, repo_name, repo_provider,
                    repo_organisation, repo_identity, dedupe_key, payload, status, attempts,
                    available_at_unix, submitted_at_unix, updated_at_unix, last_error
             FROM devql_producer_spool_jobs
             WHERE repo_id = ?1
             ORDER BY updated_at_unix DESC, submitted_at_unix DESC, job_id ASC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![repo_id, limit], map_producer_spool_job_record_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(anyhow::Error::from)
    })
}

pub(crate) fn count_producer_spool_jobs(
    config_root: &Path,
    repo_id: &str,
) -> Result<ProducerSpoolJobCounts> {
    let sqlite = open_repo_runtime_sqlite_for_config_root(config_root)?;
    sqlite.with_connection(|conn| {
        let (pending, running): (i64, i64) = conn.query_row(
            "SELECT
                COALESCE(SUM(CASE WHEN status = ?2 THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN status = ?3 THEN 1 ELSE 0 END), 0)
             FROM devql_producer_spool_jobs
             WHERE repo_id = ?1",
            params![
                repo_id,
                ProducerSpoolJobStatus::Pending.as_str(),
                ProducerSpoolJobStatus::Running.as_str(),
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(ProducerSpoolJobCounts {
            pending: u64::try_from(pending).unwrap_or_default(),
            running: u64::try_from(running).unwrap_or_default(),
        })
    })
}

fn producer_spool_claim_sort_key(job: &ProducerSpoolJobRecord) -> (u64, u64, u8, String) {
    (
        job.available_at_unix,
        job.submitted_at_unix,
        producer_spool_payload_priority(&job.payload),
        job.job_id.clone(),
    )
}

fn producer_spool_payload_priority(payload: &ProducerSpoolJobPayload) -> u8 {
    match payload {
        ProducerSpoolJobPayload::PostCommitRefresh { .. } => 0,
        ProducerSpoolJobPayload::PostCommitDerivation { .. } => 1,
        _ => 0,
    }
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
