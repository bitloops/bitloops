use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
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
use crate::host::devql::{ProducerSpoolJobCounts, ProducerSpoolJobPayload, ProducerSpoolJobRecord};
use crate::host::runtime_store::RepoSqliteRuntimeStore;

const PRODUCER_SPOOL_DEBUG_LIMIT: usize = 100;
const SUPPORTING_LOG_LINE_LIMIT: usize = 80;
const TAIL_SCAN_BLOCK_SIZE: usize = 8 * 1024;

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
    #[graphql(name = "errorLines")]
    pub error_lines: Vec<RuntimeDebugLogLineObject>,
    #[graphql(name = "warnLines")]
    pub warn_lines: Vec<RuntimeDebugLogLineObject>,
    #[graphql(name = "infoLines")]
    pub info_lines: Vec<RuntimeDebugLogLineObject>,
    #[graphql(name = "debugLines")]
    pub debug_lines: Vec<RuntimeDebugLogLineObject>,
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
    let spool_counts = crate::host::devql::count_producer_spool_jobs(
        &cfg.daemon_config_root,
        cfg.repo.repo_id.as_str(),
    )
    .map_err(|err| {
        graphql_error(
            "internal",
            format!("failed to load producer spool counts: {err:#}"),
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
        producer_spool: map_producer_spool(jobs, spool_counts),
        repo_state,
        watcher,
        supporting_logs: load_supporting_logs(),
    })
}

fn map_producer_spool(
    jobs: Vec<ProducerSpoolJobRecord>,
    counts: ProducerSpoolJobCounts,
) -> RuntimeDebugProducerSpoolObject {
    RuntimeDebugProducerSpoolObject {
        pending_count: to_graphql_i32(counts.pending),
        running_count: to_graphql_i32(counts.running),
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
        ProducerSpoolJobPayload::PostMergeSyncRefresh {
            merge_head_sha,
            changed_files,
            ..
        } => MappedProducerSpoolPayload {
            payload_kind: "post_merge_sync_refresh".to_string(),
            source: Some("post_merge".to_string()),
            paths: changed_files.clone(),
            commit_sha: None,
            head_sha: Some(merge_head_sha.clone()),
        },
        ProducerSpoolJobPayload::PostMergeIngestBackfill { merge_head_sha, .. } => {
            MappedProducerSpoolPayload {
                payload_kind: "post_merge_ingest_backfill".to_string(),
                source: Some("post_merge".to_string()),
                paths: Vec::new(),
                commit_sha: None,
                head_sha: Some(merge_head_sha.clone()),
            }
        }
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
    let raw_lines = match tail_log_lines(&path, SUPPORTING_LOG_LINE_LIMIT) {
        Ok(lines) => lines,
        Err(_) => {
            return RuntimeDebugLogTailObject {
                available: false,
                path: path.to_string_lossy().to_string(),
                lines: Vec::new(),
                error_lines: Vec::new(),
                warn_lines: Vec::new(),
                info_lines: Vec::new(),
                debug_lines: Vec::new(),
            };
        }
    };
    let lines = raw_lines
        .into_iter()
        .map(|line| parse_log_line(&line))
        .collect();
    let level_lines =
        tail_log_lines_by_levels(&path, SUPPORTING_LOG_LINE_LIMIT).unwrap_or_default();
    let error_lines = parse_log_lines(level_lines.error_lines);
    let warn_lines = parse_log_lines(level_lines.warn_lines);
    let info_lines = parse_log_lines(level_lines.info_lines);
    let debug_lines = parse_log_lines(level_lines.debug_lines);
    RuntimeDebugLogTailObject {
        available: true,
        path: path.to_string_lossy().to_string(),
        lines,
        error_lines,
        warn_lines,
        info_lines,
        debug_lines,
    }
}

fn parse_log_lines(raw_lines: Vec<String>) -> Vec<RuntimeDebugLogLineObject> {
    raw_lines
        .into_iter()
        .map(|line| parse_log_line(&line))
        .collect()
}

fn tail_log_lines(path: &Path, lines: usize) -> anyhow::Result<Vec<String>> {
    if lines == 0 {
        return Ok(Vec::new());
    }

    let mut file =
        File::open(path).with_context(|| format!("opening daemon log {}", path.display()))?;
    let start = find_tail_start_offset(&mut file, lines, path)?;
    file.seek(SeekFrom::Start(start))
        .with_context(|| format!("seeking daemon log {}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("reading daemon log {}", path.display()))?;
    let content = String::from_utf8(bytes)
        .with_context(|| format!("decoding daemon log {}", path.display()))?;

    Ok(content.lines().map(str::to_owned).collect())
}

#[derive(Debug, Default)]
struct LogLevelLineTails {
    error_lines: Vec<String>,
    warn_lines: Vec<String>,
    info_lines: Vec<String>,
    debug_lines: Vec<String>,
}

fn tail_log_lines_by_levels(path: &Path, lines: usize) -> anyhow::Result<LogLevelLineTails> {
    if lines == 0 {
        return Ok(LogLevelLineTails::default());
    }

    let file =
        File::open(path).with_context(|| format!("opening daemon log {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut error_lines = VecDeque::with_capacity(lines);
    let mut warn_lines = VecDeque::with_capacity(lines);
    let mut info_lines = VecDeque::with_capacity(lines);
    let mut debug_lines = VecDeque::with_capacity(lines);
    for raw_line in reader.lines() {
        let raw_line =
            raw_line.with_context(|| format!("reading daemon log {}", path.display()))?;
        match log_line_level(&raw_line) {
            Some("error") => push_tail_line(&mut error_lines, raw_line, lines),
            Some("warn") => push_tail_line(&mut warn_lines, raw_line, lines),
            Some("info") => push_tail_line(&mut info_lines, raw_line, lines),
            Some("debug") => push_tail_line(&mut debug_lines, raw_line, lines),
            Some(_) | None => {}
        }
    }

    Ok(LogLevelLineTails {
        error_lines: error_lines.into_iter().collect(),
        warn_lines: warn_lines.into_iter().collect(),
        info_lines: info_lines.into_iter().collect(),
        debug_lines: debug_lines.into_iter().collect(),
    })
}

fn push_tail_line(tail: &mut VecDeque<String>, raw_line: String, lines: usize) {
    if tail.len() == lines {
        tail.pop_front();
    }
    tail.push_back(raw_line);
}

fn log_line_level(raw: &str) -> Option<&'static str> {
    let Ok(parsed) = serde_json::from_str::<Value>(raw) else {
        return None;
    };
    let level = parsed
        .get("level")
        .or_else(|| parsed.get("lvl"))
        .and_then(Value::as_str)?;
    normalize_log_level(level)
}

fn normalize_log_level(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_uppercase().as_str() {
        "DEBUG" => Some("debug"),
        "INFO" => Some("info"),
        "WARN" | "WARNING" => Some("warn"),
        "ERROR" => Some("error"),
        _ => None,
    }
}

fn find_tail_start_offset(file: &mut File, lines: usize, path: &Path) -> anyhow::Result<u64> {
    let file_len = file
        .metadata()
        .with_context(|| format!("reading daemon log metadata {}", path.display()))?
        .len();
    if file_len == 0 {
        return Ok(0);
    }

    let mut remaining = file_len;
    let mut needed = lines;
    let mut skip_trailing_newline = file_ends_with_newline(file, file_len, path)?;
    let mut buffer = vec![0_u8; TAIL_SCAN_BLOCK_SIZE];

    while remaining > 0 {
        let read_size = remaining.min(TAIL_SCAN_BLOCK_SIZE as u64) as usize;
        remaining -= read_size as u64;
        file.seek(SeekFrom::Start(remaining))
            .with_context(|| format!("seeking daemon log {}", path.display()))?;
        file.read_exact(&mut buffer[..read_size])
            .with_context(|| format!("reading daemon log {}", path.display()))?;

        for idx in (0..read_size).rev() {
            if buffer[idx] != b'\n' {
                continue;
            }

            let newline_pos = remaining + idx as u64;
            if skip_trailing_newline && newline_pos == file_len - 1 {
                skip_trailing_newline = false;
                continue;
            }

            needed -= 1;
            if needed == 0 {
                return Ok(newline_pos + 1);
            }
        }
    }

    Ok(0)
}

fn file_ends_with_newline(file: &mut File, file_len: u64, path: &Path) -> anyhow::Result<bool> {
    if file_len == 0 {
        return Ok(false);
    }

    let mut byte = [0_u8; 1];
    file.seek(SeekFrom::Start(file_len - 1))
        .with_context(|| format!("seeking daemon log {}", path.display()))?;
    file.read_exact(&mut byte)
        .with_context(|| format!("reading daemon log {}", path.display()))?;
    Ok(byte[0] == b'\n')
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
    use std::fs;
    use tempfile::TempDir;

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

    #[test]
    fn tail_log_lines_reads_only_requested_suffix() {
        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("daemon.log");
        let contents = (0..120)
            .map(|idx| format!(r#"{{"level":"INFO","message":"line-{idx}"}}"#))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&log_path, format!("{contents}\n")).expect("write log");

        let lines = tail_log_lines(&log_path, 3).expect("tail log lines");

        assert_eq!(
            lines,
            vec![
                r#"{"level":"INFO","message":"line-117"}"#.to_string(),
                r#"{"level":"INFO","message":"line-118"}"#.to_string(),
                r#"{"level":"INFO","message":"line-119"}"#.to_string(),
            ]
        );
    }

    #[test]
    fn tail_log_lines_by_levels_filters_before_limiting() {
        let temp = TempDir::new().expect("temp dir");
        let log_path = temp.path().join("daemon.log");
        let mut lines = vec![
            r#"{"level":"ERROR","message":"first-error"}"#.to_string(),
            r#"{"level":"ERROR","message":"second-error"}"#.to_string(),
        ];
        lines.extend((0..120).map(|idx| format!(r#"{{"level":"INFO","message":"line-{idx}"}}"#)));
        lines.push(r#"{"level":"WARN","message":"last-warn"}"#.to_string());
        lines.push(r#"{"level":"ERROR","message":"last-error"}"#.to_string());
        fs::write(&log_path, format!("{}\n", lines.join("\n"))).expect("write log");

        let level_lines = tail_log_lines_by_levels(&log_path, 2).expect("tail level log lines");

        assert_eq!(
            level_lines.error_lines,
            vec![
                r#"{"level":"ERROR","message":"second-error"}"#.to_string(),
                r#"{"level":"ERROR","message":"last-error"}"#.to_string(),
            ]
        );
        assert_eq!(
            level_lines.warn_lines,
            vec![r#"{"level":"WARN","message":"last-warn"}"#.to_string()]
        );
    }
}
