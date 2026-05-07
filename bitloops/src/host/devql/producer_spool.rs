use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::daemon::{DevqlTaskSource, DevqlTaskSpec};

#[path = "producer_spool/enqueue.rs"]
mod enqueue;
#[path = "producer_spool/payload.rs"]
mod payload;
#[path = "producer_spool/queue.rs"]
mod queue;
#[path = "producer_spool/storage.rs"]
mod storage;

#[cfg(test)]
#[path = "producer_spool/tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) use enqueue::enqueue_spooled_post_commit_derivation;
pub(crate) use enqueue::{
    enqueue_spooled_post_commit_refresh, enqueue_spooled_post_merge_refresh,
    enqueue_spooled_pre_push_sync, enqueue_spooled_sync_task,
    enqueue_spooled_sync_task_for_repo_root,
};
#[cfg(test)]
pub(crate) use queue::claim_next_producer_spool_jobs;
#[cfg(test)]
pub(crate) use queue::claim_next_producer_spool_jobs_excluding_repo_ids;
pub(crate) use queue::{
    claim_next_producer_spool_jobs_excluding, delete_producer_spool_job,
    list_recent_producer_spool_jobs, recover_running_producer_spool_jobs,
    requeue_producer_spool_job, running_producer_spool_repo_ids,
};

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

pub(crate) type PostCommitDerivationBlockKey = (String, String);

#[derive(Debug, Clone, Default)]
pub(crate) struct PostCommitDerivationClaimGuards {
    pub(crate) blocked: std::collections::HashSet<PostCommitDerivationBlockKey>,
    pub(crate) abandoned: std::collections::HashSet<PostCommitDerivationBlockKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProducerSpoolJobStatus {
    Pending,
    Running,
}

impl ProducerSpoolJobStatus {
    pub(crate) const fn as_str(self) -> &'static str {
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
    PostCommitDerivation {
        commit_sha: String,
        committed_files: Vec<String>,
        is_rebase_in_progress: bool,
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
