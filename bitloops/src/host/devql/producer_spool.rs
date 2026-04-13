use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::daemon::{DevqlTaskSource, DevqlTaskSpec, SyncTaskMode};
use crate::host::runtime_store::{RepoSqliteRuntimeStore, open_runtime_sqlite_for_config_root};
use crate::storage::SqliteConnectionPool;

const PRODUCER_SPOOL_SCHEMA_SQLITE: &str = r#"
CREATE TABLE IF NOT EXISTS devql_producer_spool_jobs (
    job_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    config_root TEXT NOT NULL,
    repo_name TEXT NOT NULL,
    repo_provider TEXT NOT NULL,
    repo_organisation TEXT NOT NULL,
    repo_identity TEXT NOT NULL,
    dedupe_key TEXT,
    payload TEXT NOT NULL,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at_unix INTEGER NOT NULL,
    submitted_at_unix INTEGER NOT NULL,
    updated_at_unix INTEGER NOT NULL,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_devql_producer_spool_jobs_status_available
ON devql_producer_spool_jobs (status, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_devql_producer_spool_jobs_repo_status
ON devql_producer_spool_jobs (repo_id, status, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_devql_producer_spool_jobs_repo_dedupe
ON devql_producer_spool_jobs (repo_id, dedupe_key, status, submitted_at_unix);
"#;

const CLAIM_BATCH_LIMIT: usize = 16;
const REQUEUE_BACKOFF_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProducerSpoolJobStatus {
    Pending,
    Running,
}

impl ProducerSpoolJobStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ProducerSpoolJobPayload {
    Task {
        source: DevqlTaskSource,
        spec: DevqlTaskSpec,
    },
    PostCommitRefresh {
        commit_sha: String,
        changed_files: Vec<String>,
    },
    PostMergeRefresh {
        head_sha: String,
        changed_files: Vec<String>,
    },
    PrePushSync {
        remote: String,
        stdin_lines: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProducerSpoolJobRecord {
    pub job_id: String,
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub config_root: PathBuf,
    pub repo_name: String,
    pub repo_provider: String,
    pub repo_organisation: String,
    pub repo_identity: String,
    pub dedupe_key: Option<String>,
    pub payload: ProducerSpoolJobPayload,
    pub status: ProducerSpoolJobStatus,
    pub attempts: u32,
    pub available_at_unix: u64,
    pub submitted_at_unix: u64,
    pub updated_at_unix: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProducerSpoolJobInsert {
    dedupe_key: Option<String>,
    payload: ProducerSpoolJobPayload,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ProducerSpoolEnqueueResult {
    pub inserted_jobs: u64,
    pub updated_jobs: u64,
}

pub(crate) fn producer_spool_schema_sql_sqlite() -> &'static str {
    PRODUCER_SPOOL_SCHEMA_SQLITE
}

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
    let mut matchers = HashMap::<PathBuf, super::RepoExclusionMatcher>::new();
    let now = unix_timestamp_now();
    for row in rows {
        let job = row?;
        let matcher = matchers
            .entry(job.repo_root.clone())
            .or_insert(super::load_repo_exclusion_matcher(&job.repo_root)?)
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

fn prune_excluded_paths_from_payload(
    payload: ProducerSpoolJobPayload,
    matcher: &super::RepoExclusionMatcher,
) -> Option<ProducerSpoolJobPayload> {
    match payload {
        ProducerSpoolJobPayload::Task {
            source,
            spec:
                DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                    mode: SyncTaskMode::Paths { paths },
                }),
        } => {
            let paths = paths
                .into_iter()
                .filter(|path| !matcher.excludes_repo_relative_path(path))
                .collect::<Vec<_>>();
            if paths.is_empty() {
                None
            } else {
                Some(ProducerSpoolJobPayload::Task {
                    source,
                    spec: DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                        mode: SyncTaskMode::Paths { paths },
                    }),
                })
            }
        }
        ProducerSpoolJobPayload::PostCommitRefresh {
            commit_sha,
            changed_files,
        } => {
            let changed_files = changed_files
                .into_iter()
                .filter(|path| !matcher.excludes_repo_relative_path(path))
                .collect::<Vec<_>>();
            if changed_files.is_empty() {
                None
            } else {
                Some(ProducerSpoolJobPayload::PostCommitRefresh {
                    commit_sha,
                    changed_files,
                })
            }
        }
        ProducerSpoolJobPayload::PostMergeRefresh {
            head_sha,
            changed_files,
        } => {
            let changed_files = changed_files
                .into_iter()
                .filter(|path| !matcher.excludes_repo_relative_path(path))
                .collect::<Vec<_>>();
            if changed_files.is_empty() {
                None
            } else {
                Some(ProducerSpoolJobPayload::PostMergeRefresh {
                    head_sha,
                    changed_files,
                })
            }
        }
        payload => Some(payload),
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

fn merge_pending_payload(
    existing: ProducerSpoolJobPayload,
    incoming: ProducerSpoolJobPayload,
) -> ProducerSpoolJobPayload {
    match (existing, incoming) {
        (
            ProducerSpoolJobPayload::Task {
                source: existing_source,
                spec:
                    DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                        mode:
                            SyncTaskMode::Paths {
                                paths: existing_paths,
                            },
                    }),
            },
            ProducerSpoolJobPayload::Task {
                source: incoming_source,
                spec:
                    DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                        mode:
                            SyncTaskMode::Paths {
                                paths: incoming_paths,
                            },
                    }),
            },
        ) if existing_source == incoming_source => {
            let mut paths = existing_paths;
            paths.extend(incoming_paths);
            paths.sort();
            paths.dedup();
            ProducerSpoolJobPayload::Task {
                source: existing_source,
                spec: DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                    mode: SyncTaskMode::Paths { paths },
                }),
            }
        }
        (_, incoming) => incoming,
    }
}

fn spool_task_dedupe_key(source: DevqlTaskSource, spec: &DevqlTaskSpec) -> Option<String> {
    match spec {
        DevqlTaskSpec::Sync(sync) => Some(format!(
            "task:{source}:sync:{}",
            spool_sync_mode_key(&sync.mode)
        )),
        DevqlTaskSpec::Ingest(spec) => Some(format!(
            "task:{source}:ingest:{}",
            spec.backfill
                .map(|backfill| backfill.to_string())
                .unwrap_or_else(|| "all".to_string())
        )),
        DevqlTaskSpec::EmbeddingsBootstrap(_) => None,
    }
}

fn spool_sync_mode_key(mode: &SyncTaskMode) -> String {
    match mode {
        SyncTaskMode::Auto => "auto".to_string(),
        SyncTaskMode::Full => "full".to_string(),
        SyncTaskMode::Repair => "repair".to_string(),
        SyncTaskMode::Validate => "validate".to_string(),
        SyncTaskMode::Paths { .. } => "paths".to_string(),
    }
}

fn sync_task_spec_from_mode(mode: crate::host::devql::SyncMode) -> crate::daemon::SyncTaskSpec {
    crate::daemon::SyncTaskSpec {
        mode: match mode {
            crate::host::devql::SyncMode::Auto => SyncTaskMode::Auto,
            crate::host::devql::SyncMode::Full => SyncTaskMode::Full,
            crate::host::devql::SyncMode::Paths(paths) => SyncTaskMode::Paths {
                paths: normalize_paths(&paths),
            },
            crate::host::devql::SyncMode::Repair => SyncTaskMode::Repair,
            crate::host::devql::SyncMode::Validate => SyncTaskMode::Validate,
        },
    }
}

fn normalize_paths(paths: &[String]) -> Vec<String> {
    let mut normalized = paths
        .iter()
        .map(|path| normalize_repo_path(path))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_repo_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized.trim_start_matches('/').to_string()
}

fn open_repo_runtime_sqlite_for_config_root(config_root: &Path) -> Result<SqliteConnectionPool> {
    open_runtime_sqlite_for_config_root(config_root)
}

fn map_producer_spool_job_record_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProducerSpoolJobRecord> {
    let payload_raw = row.get::<_, String>(9)?;
    let payload =
        serde_json::from_str(&payload_raw).unwrap_or(ProducerSpoolJobPayload::PrePushSync {
            remote: String::new(),
            stdin_lines: Vec::new(),
        });
    Ok(ProducerSpoolJobRecord {
        job_id: row.get(0)?,
        repo_id: row.get(1)?,
        repo_root: PathBuf::from(row.get::<_, String>(2)?),
        config_root: PathBuf::from(row.get::<_, String>(3)?),
        repo_name: row.get(4)?,
        repo_provider: row.get(5)?,
        repo_organisation: row.get(6)?,
        repo_identity: row.get(7)?,
        dedupe_key: row.get(8)?,
        payload,
        status: ProducerSpoolJobStatus::parse(&row.get::<_, String>(10)?),
        attempts: row.get(11)?,
        available_at_unix: parse_u64(row.get::<_, i64>(12)?),
        submitted_at_unix: parse_u64(row.get::<_, i64>(13)?),
        updated_at_unix: parse_u64(row.get::<_, i64>(14)?),
        last_error: row.get(15)?,
    })
}

fn parse_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn sql_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("converting DevQL producer spool integer to sqlite i64")
}

fn unix_timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::REPO_POLICY_LOCAL_FILE_NAME;
    use crate::test_support::git_fixtures::{init_test_repo, write_test_daemon_config};
    use tempfile::TempDir;

    fn seed_store() -> (
        TempDir,
        PathBuf,
        crate::host::devql::RepoIdentity,
        RepoSqliteRuntimeStore,
    ) {
        let dir = TempDir::new().expect("temp dir");
        let repo_root = dir.path().join("repo");
        std::fs::create_dir_all(&repo_root).expect("create repo dir");
        init_test_repo(
            &repo_root,
            "main",
            "Bitloops Test",
            "bitloops-test@example.com",
        );
        let config_path = write_test_daemon_config(dir.path());
        crate::config::settings::write_repo_daemon_binding(
            &repo_root.join(REPO_POLICY_LOCAL_FILE_NAME),
            &config_path,
        )
        .expect("write repo daemon binding");
        let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
        let store = RepoSqliteRuntimeStore::open_for_roots(dir.path(), &repo_root)
            .expect("open repo runtime store");
        (dir, repo_root, repo, store)
    }

    #[test]
    fn spooled_sync_paths_merge_into_existing_pending_job() {
        let (_dir, repo_root, repo, store) = seed_store();
        let cfg = crate::host::devql::DevqlConfig::from_roots(
            store.config_root.clone(),
            repo_root.clone(),
            repo.clone(),
        )
        .expect("build devql config");

        enqueue_spooled_sync_task(
            &cfg,
            DevqlTaskSource::Watcher,
            crate::host::devql::SyncMode::Paths(vec![
                "src/b.ts".to_string(),
                "src/a.ts".to_string(),
            ]),
        )
        .expect("enqueue first watcher sync");
        enqueue_spooled_sync_task(
            &cfg,
            DevqlTaskSource::Watcher,
            crate::host::devql::SyncMode::Paths(vec![
                "src/c.ts".to_string(),
                "src/a.ts".to_string(),
            ]),
        )
        .expect("enqueue second watcher sync");

        let claimed =
            claim_next_producer_spool_jobs(&store.config_root).expect("claim producer spool jobs");
        assert_eq!(claimed.len(), 1, "watcher path jobs should coalesce");
        match &claimed[0].payload {
            ProducerSpoolJobPayload::Task {
                source,
                spec:
                    DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                        mode: SyncTaskMode::Paths { paths },
                    }),
            } => {
                assert_eq!(*source, DevqlTaskSource::Watcher);
                assert_eq!(
                    paths,
                    &vec![
                        "src/a.ts".to_string(),
                        "src/b.ts".to_string(),
                        "src/c.ts".to_string(),
                    ]
                );
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[test]
    fn producer_spool_claims_at_most_one_running_job_per_repo() {
        let (_dir, repo_root, repo, store) = seed_store();
        let cfg = crate::host::devql::DevqlConfig::from_roots(
            store.config_root.clone(),
            repo_root.clone(),
            repo.clone(),
        )
        .expect("build devql config");

        enqueue_spooled_sync_task(
            &cfg,
            DevqlTaskSource::Watcher,
            crate::host::devql::SyncMode::Paths(vec!["src/a.ts".to_string()]),
        )
        .expect("enqueue first sync");
        enqueue_spooled_post_commit_refresh(&repo_root, "commit-a", &["src/a.ts".to_string()])
            .expect("enqueue post-commit refresh");

        let first = claim_next_producer_spool_jobs(&store.config_root)
            .expect("claim first producer spool batch");
        assert_eq!(first.len(), 1, "only one producer job should run per repo");

        let second = claim_next_producer_spool_jobs(&store.config_root)
            .expect("claim second producer spool batch");
        assert!(
            second.is_empty(),
            "pending jobs for the same repo should wait until the running job finishes"
        );
    }

    #[test]
    fn running_producer_spool_jobs_recover_back_to_pending() {
        let (_dir, repo_root, repo, store) = seed_store();
        let cfg = crate::host::devql::DevqlConfig::from_roots(
            store.config_root.clone(),
            repo_root.clone(),
            repo.clone(),
        )
        .expect("build devql config");

        enqueue_spooled_sync_task(
            &cfg,
            DevqlTaskSource::Watcher,
            crate::host::devql::SyncMode::Paths(vec!["src/a.ts".to_string()]),
        )
        .expect("enqueue sync");
        let claimed =
            claim_next_producer_spool_jobs(&store.config_root).expect("claim producer spool job");
        assert_eq!(claimed.len(), 1);

        recover_running_producer_spool_jobs(&store.config_root)
            .expect("recover running producer spool jobs");
        let reclaimed = claim_next_producer_spool_jobs(&store.config_root)
            .expect("reclaim recovered producer spool job");
        assert_eq!(
            reclaimed.len(),
            1,
            "recovered job should be claimable again"
        );
    }

    #[test]
    fn hook_enqueue_helpers_use_repo_binding_and_share_repo_runtime_store() {
        let (_dir, repo_root, _repo, store) = seed_store();

        crate::host::devql::enqueue_spooled_sync_task_for_repo_root(
            &repo_root,
            DevqlTaskSource::PostCheckout,
            crate::host::devql::SyncMode::Full,
        )
        .expect("enqueue post-checkout sync");
        crate::host::devql::enqueue_spooled_post_commit_refresh(
            &repo_root,
            "commit-head",
            &["src/lib.rs".to_string()],
        )
        .expect("enqueue post-commit refresh");
        crate::host::devql::enqueue_spooled_post_merge_refresh(
            &repo_root,
            "merge-head",
            &["src/lib.rs".to_string()],
        )
        .expect("enqueue post-merge refresh");
        crate::host::devql::enqueue_spooled_pre_push_sync(
            &repo_root,
            "origin",
            &["refs/heads/main abc refs/heads/main def".to_string()],
        )
        .expect("enqueue pre-push sync");

        let sqlite = store
            .connect_repo_sqlite()
            .expect("open repo runtime sqlite");
        let queued_rows = sqlite
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM devql_producer_spool_jobs WHERE repo_id = ?1",
                    rusqlite::params![store.repo_id()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(anyhow::Error::from)
            })
            .expect("count queued producer spool jobs");
        assert_eq!(
            queued_rows, 4,
            "helper enqueues should target the bound repo runtime db"
        );
    }
}
