use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;

use crate::daemon::{DevqlTaskSource, DevqlTaskSpec};
use crate::host::runtime_store::RepoSqliteRuntimeStore;

use super::payload::{
    merge_pending_payload, normalize_paths, spool_task_dedupe_key, sync_task_spec_from_mode,
};
use super::storage::{map_producer_spool_job_record_row, sql_i64, unix_timestamp_now};
use super::{
    ProducerSpoolEnqueueResult, ProducerSpoolJobInsert, ProducerSpoolJobPayload,
    ProducerSpoolJobRecord, ProducerSpoolJobStatus,
};

pub(crate) fn enqueue_spooled_sync_task(
    cfg: &crate::host::devql::DevqlConfig,
    source: DevqlTaskSource,
    mode: crate::host::devql::SyncMode,
) -> Result<ProducerSpoolEnqueueResult> {
    let store = RepoSqliteRuntimeStore::open_for_roots(&cfg.daemon_config_root, &cfg.repo_root)
        .context("opening repo runtime store for DevQL sync producer spool")?;
    let spec = DevqlTaskSpec::Sync(sync_task_spec_from_mode(mode));
    enqueue_job(
        &store,
        &cfg.repo,
        ProducerSpoolJobInsert {
            dedupe_key: spool_task_dedupe_key(source, &spec),
            payload: ProducerSpoolJobPayload::Task { source, spec },
        },
    )
}

pub(crate) fn enqueue_spooled_sync_task_for_repo_root(
    repo_root: &Path,
    source: DevqlTaskSource,
    mode: crate::host::devql::SyncMode,
) -> Result<ProducerSpoolEnqueueResult> {
    let repo = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for DevQL sync producer spool")?;
    let config_root = crate::config::resolve_bound_daemon_config_root_for_repo(repo_root)
        .context("resolving daemon config root for DevQL sync producer spool")?;
    let cfg =
        crate::host::devql::DevqlConfig::from_roots(config_root, repo_root.to_path_buf(), repo)
            .context("building DevQL config for sync producer spool")?;
    enqueue_spooled_sync_task(&cfg, source, mode)
}

pub(crate) fn enqueue_spooled_post_commit_refresh(
    repo_root: &Path,
    commit_sha: &str,
    changed_files: &[String],
) -> Result<ProducerSpoolEnqueueResult> {
    enqueue_hook_job(
        repo_root,
        Some(format!("post_commit:{}", commit_sha.trim())),
        ProducerSpoolJobPayload::PostCommitRefresh {
            commit_sha: commit_sha.trim().to_string(),
            changed_files: normalize_paths(changed_files),
        },
    )
}

pub(crate) fn enqueue_spooled_post_merge_refresh(
    repo_root: &Path,
    head_sha: &str,
    changed_files: &[String],
) -> Result<ProducerSpoolEnqueueResult> {
    enqueue_hook_job(
        repo_root,
        Some(format!("post_merge:{}", head_sha.trim())),
        ProducerSpoolJobPayload::PostMergeRefresh {
            head_sha: head_sha.trim().to_string(),
            changed_files: normalize_paths(changed_files),
        },
    )
}

pub(crate) fn enqueue_spooled_pre_push_sync(
    repo_root: &Path,
    remote: &str,
    stdin_lines: &[String],
) -> Result<ProducerSpoolEnqueueResult> {
    let normalized_remote = remote.trim().to_string();
    let normalized_lines = stdin_lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut hasher = Sha256::new();
    hasher.update(normalized_remote.as_bytes());
    hasher.update(b"\n");
    for line in &normalized_lines {
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    enqueue_hook_job(
        repo_root,
        Some(format!("pre_push:{}", hex::encode(hasher.finalize()))),
        ProducerSpoolJobPayload::PrePushSync {
            remote: normalized_remote,
            stdin_lines: normalized_lines,
        },
    )
}

fn enqueue_hook_job(
    repo_root: &Path,
    dedupe_key: Option<String>,
    payload: ProducerSpoolJobPayload,
) -> Result<ProducerSpoolEnqueueResult> {
    let repo = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for DevQL producer spool")?;
    let store = RepoSqliteRuntimeStore::open(repo_root)
        .context("opening repo runtime store for DevQL producer spool")?;
    enqueue_job(
        &store,
        &repo,
        ProducerSpoolJobInsert {
            dedupe_key,
            payload,
        },
    )
}

fn enqueue_job(
    store: &RepoSqliteRuntimeStore,
    repo: &crate::host::devql::RepoIdentity,
    job: ProducerSpoolJobInsert,
) -> Result<ProducerSpoolEnqueueResult> {
    let sqlite = store.connect_repo_sqlite()?;
    sqlite.with_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting DevQL producer spool enqueue transaction")?;
        let result = (|| {
            let now = unix_timestamp_now();
            if let Some(existing) =
                load_pending_deduped_job(conn, store.repo_id(), job.dedupe_key.as_deref())?
            {
                let payload = merge_pending_payload(existing.payload, job.payload.clone());
                conn.execute(
                    "UPDATE devql_producer_spool_jobs
                     SET payload = ?1,
                         updated_at_unix = ?2,
                         available_at_unix = ?3,
                         last_error = NULL
                     WHERE job_id = ?4",
                    params![
                        serde_json::to_string(&payload)
                            .context("serialising DevQL producer spool payload")?,
                        sql_i64(now)?,
                        sql_i64(now)?,
                        existing.job_id,
                    ],
                )
                .with_context(|| {
                    format!(
                        "refreshing pending DevQL producer spool job `{}`",
                        existing.job_id
                    )
                })?;
                return Ok(ProducerSpoolEnqueueResult {
                    inserted_jobs: 0,
                    updated_jobs: 1,
                });
            }

            let job_id = format!("producer-spool-job-{}", Uuid::new_v4());
            conn.execute(
                "INSERT INTO devql_producer_spool_jobs (
                    job_id, repo_id, repo_root, config_root, repo_name, repo_provider,
                    repo_organisation, repo_identity, dedupe_key, payload, status, attempts,
                    available_at_unix, submitted_at_unix, updated_at_unix, last_error
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, ?9, ?10, ?11, 0,
                    ?12, ?13, ?14, NULL
                 )",
                params![
                    &job_id,
                    store.repo_id(),
                    store.repo_root.to_string_lossy().to_string(),
                    store.config_root.to_string_lossy().to_string(),
                    &repo.name,
                    &repo.provider,
                    &repo.organization,
                    &repo.identity,
                    job.dedupe_key.as_deref(),
                    serde_json::to_string(&job.payload)
                        .context("serialising DevQL producer spool payload")?,
                    ProducerSpoolJobStatus::Pending.as_str(),
                    sql_i64(now)?,
                    sql_i64(now)?,
                    sql_i64(now)?,
                ],
            )
            .with_context(|| format!("inserting DevQL producer spool job `{job_id}`"))?;

            Ok(ProducerSpoolEnqueueResult {
                inserted_jobs: 1,
                updated_jobs: 0,
            })
        })();

        match result {
            Ok(result) => {
                conn.execute_batch("COMMIT;")
                    .context("committing DevQL producer spool enqueue transaction")?;
                Ok(result)
            }
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
    })
}

fn load_pending_deduped_job(
    conn: &rusqlite::Connection,
    repo_id: &str,
    dedupe_key: Option<&str>,
) -> Result<Option<ProducerSpoolJobRecord>> {
    let Some(dedupe_key) = dedupe_key else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT job_id, repo_id, repo_root, config_root, repo_name, repo_provider,
                repo_organisation, repo_identity, dedupe_key, payload, status, attempts,
                available_at_unix, submitted_at_unix, updated_at_unix, last_error
         FROM devql_producer_spool_jobs
         WHERE repo_id = ?1
           AND dedupe_key = ?2
           AND status = ?3
         ORDER BY submitted_at_unix ASC, job_id ASC
         LIMIT 1",
        params![
            repo_id,
            dedupe_key,
            ProducerSpoolJobStatus::Pending.as_str(),
        ],
        map_producer_spool_job_record_row,
    )
    .optional()
    .map_err(anyhow::Error::from)
}
