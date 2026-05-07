use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;
use async_graphql::{Result as GraphqlResult, SimpleObject};
use serde_json::Value;

use super::config::{map_runtime_api_error, resolve_runtime_devql_config};
use super::roots::RuntimeRequestContext;
use super::util::{to_graphql_i32, to_graphql_i64};
use crate::api::DashboardState;
use crate::daemon::{DevqlTaskSpec, SyncTaskMode};
use crate::graphql::graphql_error;
use crate::host::devql::{ProducerSpoolJobPayload, ProducerSpoolJobRecord, ProducerSpoolJobStatus};
use crate::host::runtime_store::RepoSqliteRuntimeStore;

const PRODUCER_SPOOL_DEBUG_LIMIT: usize = 100;
const SUPPORTING_LOG_LINE_LIMIT: usize = 80;

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeDebugSnapshotObject {
    #[graphql(name = "repoId")]
    pub repo_id: String,
    #[graphql(name = "producerSpool")]
    pub producer_spool: RuntimeDebugProducerSpoolObject,
    #[graphql(name = "repoState")]
    pub repo_state: RuntimeDebugRepoStateObject,
    pub watcher: RuntimeDebugWatcherObject,
    #[graphql(name = "supportingLogs")]
    pub supporting_logs: RuntimeDebugLogTailObject,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeDebugProducerSpoolObject {
    #[graphql(name = "pendingCount")]
    pub pending_count: i32,
    #[graphql(name = "runningCount")]
    pub running_count: i32,
    pub jobs: Vec<RuntimeDebugProducerSpoolJobObject>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeDebugProducerSpoolJobObject {
    #[graphql(name = "jobId")]
    pub job_id: String,
    pub status: String,
    #[graphql(name = "payloadKind")]
    pub payload_kind: String,
    pub source: Option<String>,
    #[graphql(name = "dedupeKey")]
    pub dedupe_key: Option<String>,
    pub attempts: i32,
    #[graphql(name = "availableAtUnix")]
    pub available_at_unix: i64,
    #[graphql(name = "submittedAtUnix")]
    pub submitted_at_unix: i64,
    #[graphql(name = "updatedAtUnix")]
    pub updated_at_unix: i64,
    #[graphql(name = "lastError")]
    pub last_error: Option<String>,
    #[graphql(name = "pathCount")]
    pub path_count: i32,
    pub paths: Vec<String>,
    #[graphql(name = "commitSha")]
    pub commit_sha: Option<String>,
    #[graphql(name = "headSha")]
    pub head_sha: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeDebugRepoStateObject {
    pub branch: String,
    #[graphql(name = "headSha")]
    pub head_sha: String,
    #[graphql(name = "mergeState")]
    pub merge_state: String,
    #[graphql(name = "stagedPaths")]
    pub staged_paths: Vec<String>,
    #[graphql(name = "unstagedPaths")]
    pub unstaged_paths: Vec<String>,
    #[graphql(name = "untrackedPaths")]
    pub untracked_paths: Vec<String>,
    #[graphql(name = "deletedPaths")]
    pub deleted_paths: Vec<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeDebugWatcherObject {
    pub registered: bool,
    #[graphql(name = "repoRoot")]
    pub repo_root: Option<String>,
    pub pid: Option<i32>,
    pub state: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeDebugLogTailObject {
    pub available: bool,
    pub path: String,
    pub lines: Vec<RuntimeDebugLogLineObject>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeDebugLogLineObject {
    pub level: Option<String>,
    pub message: Option<String>,
    pub raw: String,
    #[graphql(name = "timestampUnix")]
    pub timestamp_unix: Option<i64>,
}

pub(crate) async fn load_runtime_debug_snapshot(
    state: &DashboardState,
    request_context: RuntimeRequestContext,
    repo_id: &str,
) -> GraphqlResult<RuntimeDebugSnapshotObject> {
    let cfg = resolve_runtime_devql_config(state, &request_context, repo_id)
        .await
        .map_err(map_runtime_api_error)?;
    let jobs = crate::host::devql::list_recent_producer_spool_jobs(
        &cfg.daemon_config_root,
        cfg.repo.repo_id.as_str(),
        PRODUCER_SPOOL_DEBUG_LIMIT,
    )
    .map_err(|err| {
        graphql_error(
            "internal",
            format!("failed to load producer spool diagnostics: {err:#}"),
        )
    })?;
    let repo_state = load_repo_state(&cfg.repo_root).map_err(|err| {
        graphql_error(
            "internal",
            format!("failed to load git repository diagnostics: {err:#}"),
        )
    })?;
    let watcher = load_watcher_state(&cfg).map_err(|err| {
        graphql_error(
            "internal",
            format!("failed to load watcher diagnostics: {err:#}"),
        )
    })?;

    Ok(RuntimeDebugSnapshotObject {
        repo_id: cfg.repo.repo_id,
        producer_spool: map_producer_spool(jobs),
        repo_state,
        watcher,
        supporting_logs: load_supporting_logs(),
    })
}

fn map_producer_spool(jobs: Vec<ProducerSpoolJobRecord>) -> RuntimeDebugProducerSpoolObject {
    let pending_count = jobs
        .iter()
        .filter(|job| job.status == ProducerSpoolJobStatus::Pending)
        .count();
    let running_count = jobs
        .iter()
        .filter(|job| job.status == ProducerSpoolJobStatus::Running)
        .count();
    RuntimeDebugProducerSpoolObject {
        pending_count: to_graphql_i32(pending_count),
        running_count: to_graphql_i32(running_count),
        jobs: jobs.into_iter().map(map_producer_spool_job).collect(),
    }
}

fn map_producer_spool_job(job: ProducerSpoolJobRecord) -> RuntimeDebugProducerSpoolJobObject {
    let payload = map_producer_spool_payload(&job.payload);
    RuntimeDebugProducerSpoolJobObject {
        job_id: job.job_id,
        status: job.status.as_str().to_string(),
        payload_kind: payload.payload_kind,
        source: payload.source,
        dedupe_key: job.dedupe_key,
        attempts: to_graphql_i32(job.attempts),
        available_at_unix: to_graphql_i64(job.available_at_unix),
        submitted_at_unix: to_graphql_i64(job.submitted_at_unix),
        updated_at_unix: to_graphql_i64(job.updated_at_unix),
        last_error: job.last_error,
        path_count: to_graphql_i32(payload.paths.len()),
        paths: payload.paths,
        commit_sha: payload.commit_sha,
        head_sha: payload.head_sha,
    }
}

struct MappedProducerSpoolPayload {
    payload_kind: String,
    source: Option<String>,
    paths: Vec<String>,
    commit_sha: Option<String>,
    head_sha: Option<String>,
}

fn map_producer_spool_payload(payload: &ProducerSpoolJobPayload) -> MappedProducerSpoolPayload {
    match payload {
        ProducerSpoolJobPayload::Task { source, spec } => {
            let paths = match spec {
                DevqlTaskSpec::Sync(sync) => match &sync.mode {
                    SyncTaskMode::Paths { paths } => paths.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            MappedProducerSpoolPayload {
                payload_kind: "task".to_string(),
                source: Some(source.to_string()),
                paths,
                commit_sha: None,
                head_sha: None,
            }
        }
        ProducerSpoolJobPayload::PostCommitRefresh {
            commit_sha,
            changed_files,
        } => MappedProducerSpoolPayload {
            payload_kind: "post_commit_refresh".to_string(),
            source: Some("post_commit".to_string()),
            paths: changed_files.clone(),
            commit_sha: Some(commit_sha.clone()),
            head_sha: None,
        },
        ProducerSpoolJobPayload::PostCommitDerivation {
            commit_sha,
            committed_files,
            ..
        } => MappedProducerSpoolPayload {
            payload_kind: "post_commit_derivation".to_string(),
            source: Some("post_commit".to_string()),
            paths: committed_files.clone(),
            commit_sha: Some(commit_sha.clone()),
            head_sha: None,
        },
        ProducerSpoolJobPayload::PostMergeRefresh {
            head_sha,
            changed_files,
        } => MappedProducerSpoolPayload {
            payload_kind: "post_merge_refresh".to_string(),
            source: Some("post_merge".to_string()),
            paths: changed_files.clone(),
            commit_sha: None,
            head_sha: Some(head_sha.clone()),
        },
        ProducerSpoolJobPayload::PrePushSync { .. } => MappedProducerSpoolPayload {
            payload_kind: "pre_push_sync".to_string(),
            source: Some("pre_push".to_string()),
            paths: Vec::new(),
            commit_sha: None,
            head_sha: None,
        },
    }
}

fn load_repo_state(repo_root: &Path) -> anyhow::Result<RuntimeDebugRepoStateObject> {
    let status = run_git(
        repo_root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .context("running git status")?
    .unwrap_or_default();
    let mut staged_paths = Vec::new();
    let mut unstaged_paths = Vec::new();
    let mut untracked_paths = Vec::new();
    let mut deleted_paths = Vec::new();
    for line in status.lines() {
        let Some(parsed) = parse_porcelain_status_line(line) else {
            continue;
        };
        if parsed.untracked {
            untracked_paths.push(parsed.path);
            continue;
        }
        if parsed.staged {
            staged_paths.push(parsed.path.clone());
        }
        if parsed.unstaged {
            unstaged_paths.push(parsed.path.clone());
        }
        if parsed.deleted {
            deleted_paths.push(parsed.path);
        }
    }

    Ok(RuntimeDebugRepoStateObject {
        branch: run_git(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])?
            .unwrap_or_else(|| "unknown".to_string()),
        head_sha: run_git(repo_root, &["rev-parse", "HEAD"])?
            .unwrap_or_else(|| "unknown".to_string()),
        merge_state: detect_merge_state(repo_root),
        staged_paths,
        unstaged_paths,
        untracked_paths,
        deleted_paths,
    })
}

struct PorcelainStatusLine {
    path: String,
    staged: bool,
    unstaged: bool,
    untracked: bool,
    deleted: bool,
}

fn parse_porcelain_status_line(line: &str) -> Option<PorcelainStatusLine> {
    let status = line.as_bytes();
    if status.len() < 2 {
        return None;
    }
    let staged_status = status[0] as char;
    let unstaged_status = status[1] as char;
    let path_start = if status.get(2) == Some(&b' ') { 3 } else { 2 };
    let path = line.get(path_start..)?.trim_start();
    if path.is_empty() {
        return None;
    }

    Some(PorcelainStatusLine {
        path: normalize_porcelain_path(path),
        staged: staged_status != ' ' && staged_status != '?',
        unstaged: unstaged_status != ' ' && unstaged_status != '?',
        untracked: staged_status == '?' && unstaged_status == '?',
        deleted: staged_status == 'D' || unstaged_status == 'D',
    })
}

fn normalize_porcelain_path(path: &str) -> String {
    path.rsplit_once(" -> ")
        .map(|(_, to_path)| to_path)
        .unwrap_or(path)
        .trim_matches('"')
        .to_string()
}

fn detect_merge_state(repo_root: &Path) -> String {
    let Some(git_dir) = resolve_git_dir(repo_root) else {
        return "unknown".to_string();
    };
    if git_dir.join("MERGE_HEAD").exists() {
        "merge".to_string()
    } else if git_dir.join("REBASE_HEAD").exists()
        || git_dir.join("rebase-merge").exists()
        || git_dir.join("rebase-apply").exists()
    {
        "rebase".to_string()
    } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
        "cherry_pick".to_string()
    } else {
        "none".to_string()
    }
}

fn resolve_git_dir(repo_root: &Path) -> Option<PathBuf> {
    let raw = run_git(repo_root, &["rev-parse", "--git-dir"])
        .ok()
        .flatten()?;
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(repo_root.join(path))
    }
}

fn run_git(repo_root: &Path, args: &[&str]) -> anyhow::Result<Option<String>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

fn load_watcher_state(
    cfg: &crate::host::devql::DevqlConfig,
) -> anyhow::Result<RuntimeDebugWatcherObject> {
    let store = RepoSqliteRuntimeStore::open_for_roots(&cfg.daemon_config_root, &cfg.repo_root)?;
    let Some(registration) = store.load_watcher_registration()? else {
        return Ok(RuntimeDebugWatcherObject {
            registered: false,
            repo_root: None,
            pid: None,
            state: None,
        });
    };
    Ok(RuntimeDebugWatcherObject {
        registered: true,
        repo_root: Some(registration.repo_root.to_string_lossy().to_string()),
        pid: Some(to_graphql_i32(registration.pid)),
        state: Some(registration.state.as_str().to_string()),
    })
}

fn load_supporting_logs() -> RuntimeDebugLogTailObject {
    let path = crate::daemon::daemon_log_file_path();
    let raw = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(_) => {
            return RuntimeDebugLogTailObject {
                available: false,
                path: path.to_string_lossy().to_string(),
                lines: Vec::new(),
            };
        }
    };
    let mut lines = raw
        .lines()
        .rev()
        .take(SUPPORTING_LOG_LINE_LIMIT)
        .map(parse_log_line)
        .collect::<Vec<_>>();
    lines.reverse();
    RuntimeDebugLogTailObject {
        available: true,
        path: path.to_string_lossy().to_string(),
        lines,
    }
}

fn parse_log_line(raw: &str) -> RuntimeDebugLogLineObject {
    let parsed = serde_json::from_str::<Value>(raw).ok();
    let level = parsed
        .as_ref()
        .and_then(|value| value.get("level").or_else(|| value.get("lvl")))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let message = parsed
        .as_ref()
        .and_then(|value| {
            value
                .get("message")
                .or_else(|| value.get("msg"))
                .or_else(|| value.pointer("/fields/message"))
        })
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let timestamp_unix = parsed
        .as_ref()
        .and_then(|value| value.get("timestamp_unix"))
        .and_then(Value::as_i64);
    RuntimeDebugLogLineObject {
        level,
        message,
        raw: raw.to_string(),
        timestamp_unix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_porcelain_status_line_keeps_paths_for_two_and_one_column_statuses() {
        let staged = parse_porcelain_status_line("M  bitloops/src/api/runtime_schema.rs")
            .expect("parse staged status");
        assert!(staged.staged);
        assert!(!staged.unstaged);
        assert_eq!(staged.path, "bitloops/src/api/runtime_schema.rs");

        let unstaged = parse_porcelain_status_line(" M bitloops/src/api/runtime_schema.rs")
            .expect("parse unstaged status");
        assert!(!unstaged.staged);
        assert!(unstaged.unstaged);
        assert_eq!(unstaged.path, "bitloops/src/api/runtime_schema.rs");

        let compact = parse_porcelain_status_line("M bitloops/src/api/runtime_schema.rs")
            .expect("parse compact status");
        assert!(compact.staged);
        assert_eq!(compact.path, "bitloops/src/api/runtime_schema.rs");
    }
}
