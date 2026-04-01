use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::host::devql::RelationalStorage;
use crate::host::devql::db_utils::esc_pg;
use crate::host::devql::sql_now;

const LOCK_DIR: &str = ".bitloops";
const LOCK_FILE: &str = "sync.lock";

#[derive(Debug, Clone, PartialEq, Eq)]
struct LockOwner {
    pid: u32,
    token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LockOwnerState {
    Missing,
    Parsed(LockOwner),
    Unparseable,
}

#[derive(Debug)]
pub(crate) struct SyncLock {
    lock_path: PathBuf,
    owner: LockOwner,
}

impl SyncLock {
    pub(crate) fn acquire(config_root: &Path) -> Result<Self> {
        acquire_lock(config_root)
    }

    #[cfg(test)]
    pub(crate) fn try_acquire(config_root: &Path) -> Result<Self> {
        acquire_lock(config_root)
    }

    #[cfg(test)]
    pub(crate) fn is_held(&self) -> bool {
        self.lock_path.exists()
    }
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        let owned_by_self = matches!(
            read_lock_owner_state(&self.lock_path),
            Ok(LockOwnerState::Parsed(owner)) if owner == self.owner
        );
        if owned_by_self {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

pub(crate) async fn write_sync_started(
    store: &RelationalStorage,
    repo_id: &str,
    repo_root: &str,
    reason: &str,
    parser_version: &str,
    extractor_version: &str,
) -> Result<()> {
    let now_sql = sql_now(store);
    let sql = format!(
        "INSERT INTO repo_sync_state (\
repo_id, repo_root, active_branch, head_commit_sha, head_tree_sha, parser_version, extractor_version, \
last_sync_started_at, last_sync_completed_at, last_sync_status, last_sync_reason\
) VALUES (\
'{}', '{}', NULL, NULL, NULL, '{}', '{}', {}, NULL, 'running', '{}'\
) ON CONFLICT (repo_id) DO UPDATE SET \
repo_root = EXCLUDED.repo_root, \
active_branch = NULL, \
head_commit_sha = NULL, \
head_tree_sha = NULL, \
parser_version = EXCLUDED.parser_version, \
extractor_version = EXCLUDED.extractor_version, \
last_sync_started_at = {}, \
last_sync_completed_at = NULL, \
last_sync_status = 'running', \
last_sync_reason = EXCLUDED.last_sync_reason",
        esc_pg(repo_id),
        esc_pg(repo_root),
        esc_pg(parser_version),
        esc_pg(extractor_version),
        now_sql,
        esc_pg(reason),
        now_sql,
    );
    store.exec(&sql).await
}

pub(crate) async fn write_sync_completed(
    store: &RelationalStorage,
    repo_id: &str,
    head_commit_sha: Option<&str>,
    head_tree_sha: Option<&str>,
    active_branch: Option<&str>,
    parser_version: &str,
    extractor_version: &str,
) -> Result<()> {
    ensure_repo_sync_state_exists(store, repo_id).await?;
    let now_sql = sql_now(store);
    let sql = format!(
        "UPDATE repo_sync_state SET \
active_branch = {}, \
head_commit_sha = {}, \
head_tree_sha = {}, \
parser_version = '{}', \
extractor_version = '{}', \
last_sync_completed_at = {}, \
last_sync_status = 'completed' \
WHERE repo_id = '{}'",
        nullable_text_sql(active_branch),
        nullable_text_sql(head_commit_sha),
        nullable_text_sql(head_tree_sha),
        esc_pg(parser_version),
        esc_pg(extractor_version),
        now_sql,
        esc_pg(repo_id),
    );
    store.exec(&sql).await
}

pub(crate) async fn write_sync_failed(store: &RelationalStorage, repo_id: &str) -> Result<()> {
    ensure_repo_sync_state_exists(store, repo_id).await?;
    let sql = format!(
        "UPDATE repo_sync_state SET last_sync_status = 'failed' WHERE repo_id = '{}'",
        esc_pg(repo_id),
    );
    store.exec(&sql).await
}

fn acquire_lock(config_root: &Path) -> Result<SyncLock> {
    let lock_path = sync_lock_path(config_root);
    let owner = LockOwner {
        pid: std::process::id(),
        token: uuid::Uuid::new_v4().to_string(),
    };

    if try_write_lock_file(&lock_path, &owner)? {
        return Ok(SyncLock { lock_path, owner });
    }

    if clear_stale_lock(&lock_path)? && try_write_lock_file(&lock_path, &owner)? {
        return Ok(SyncLock { lock_path, owner });
    }

    let holder = describe_lock_holder(&lock_path);
    bail!(
        "sync lock already held at {} by {}",
        lock_path.display(),
        holder,
    )
}

fn sync_lock_path(config_root: &Path) -> PathBuf {
    config_root.join(LOCK_DIR).join(LOCK_FILE)
}

fn try_write_lock_file(lock_path: &Path, owner: &LockOwner) -> Result<bool> {
    ensure_lock_parent_dir(lock_path)?;
    let payload = format!("{}\n{}\n", owner.pid, owner.token);
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(err) => {
            return Err(err).with_context(|| format!("creating sync lock {}", lock_path.display()));
        }
    };

    file.write_all(payload.as_bytes())
        .with_context(|| format!("writing sync lock payload to {}", lock_path.display()))?;
    file.sync_all()
        .with_context(|| format!("flushing sync lock {}", lock_path.display()))?;
    Ok(true)
}

fn ensure_lock_parent_dir(lock_path: &Path) -> Result<()> {
    let Some(parent) = lock_path.parent() else {
        bail!(
            "sync lock path {} has no parent directory",
            lock_path.display()
        );
    };
    fs::create_dir_all(parent)
        .with_context(|| format!("creating sync lock directory {}", parent.display()))
}

fn clear_stale_lock(lock_path: &Path) -> Result<bool> {
    match read_lock_owner_state(lock_path)? {
        LockOwnerState::Parsed(owner) if !process_is_alive(owner.pid) => {
            fs::remove_file(lock_path)
                .or_else(|err| {
                    if err.kind() == ErrorKind::NotFound {
                        Ok(())
                    } else {
                        Err(err)
                    }
                })
                .with_context(|| format!("removing stale sync lock {}", lock_path.display()))?;
            Ok(true)
        }
        LockOwnerState::Parsed(_) | LockOwnerState::Unparseable | LockOwnerState::Missing => {
            Ok(false)
        }
    }
}

fn read_lock_owner_state(lock_path: &Path) -> Result<LockOwnerState> {
    let content = match fs::read_to_string(lock_path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(LockOwnerState::Missing),
        Err(err) => {
            return Err(err).with_context(|| format!("reading sync lock {}", lock_path.display()));
        }
    };

    let mut lines = content.lines();
    let Some(pid_line) = lines.next().map(str::trim).filter(|line| !line.is_empty()) else {
        return Ok(LockOwnerState::Unparseable);
    };
    let Some(token) = lines.next().map(str::trim).filter(|line| !line.is_empty()) else {
        return Ok(LockOwnerState::Unparseable);
    };
    let Ok(pid) = pid_line.parse::<u32>() else {
        return Ok(LockOwnerState::Unparseable);
    };

    Ok(LockOwnerState::Parsed(LockOwner {
        pid,
        token: token.to_string(),
    }))
}

fn describe_lock_holder(lock_path: &Path) -> String {
    match read_lock_owner_state(lock_path) {
        Ok(LockOwnerState::Parsed(owner)) => format!("pid {}", owner.pid),
        Ok(LockOwnerState::Unparseable) => "an unparseable lock file".to_string(),
        Ok(LockOwnerState::Missing) => "a disappearing lock file".to_string(),
        Err(_) => "an unreadable lock file".to_string(),
    }
}

async fn ensure_repo_sync_state_exists(store: &RelationalStorage, repo_id: &str) -> Result<()> {
    let rows = store
        .query_rows(&format!(
            "SELECT repo_id FROM repo_sync_state WHERE repo_id = '{}' LIMIT 1",
            esc_pg(repo_id),
        ))
        .await?;
    if rows.is_empty() {
        bail!("repo_sync_state row missing for repo_id `{repo_id}`")
    }
    Ok(())
}

fn process_is_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args([
                "/C",
                &format!("tasklist /FI \"PID eq {pid}\" | findstr {pid}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(not(windows))]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

fn nullable_text_sql(value: Option<&str>) -> String {
    value
        .map(|value| format!("'{}'", esc_pg(value)))
        .unwrap_or_else(|| "NULL".to_string())
}
