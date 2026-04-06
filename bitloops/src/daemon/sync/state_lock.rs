use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use uuid::Uuid;

use super::super::process_is_running;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StateLockOwner {
    pid: u32,
    token: String,
}

#[derive(Debug)]
pub(super) struct StateFileLockGuard {
    lock_path: PathBuf,
    owner: StateLockOwner,
}

impl Drop for StateFileLockGuard {
    fn drop(&mut self) {
        if matches!(
            read_state_lock_owner(&self.lock_path),
            Ok(Some(owner)) if owner == self.owner
        ) {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

pub(super) fn acquire_state_file_lock(lock_path: &Path) -> Result<StateFileLockGuard> {
    let owner = StateLockOwner {
        pid: std::process::id(),
        token: Uuid::new_v4().to_string(),
    };

    if try_write_state_lock(lock_path, &owner)? {
        return Ok(StateFileLockGuard {
            lock_path: lock_path.to_path_buf(),
            owner,
        });
    }

    if clear_stale_state_lock(lock_path)? && try_write_state_lock(lock_path, &owner)? {
        return Ok(StateFileLockGuard {
            lock_path: lock_path.to_path_buf(),
            owner,
        });
    }

    bail!("sync queue lock already held at {}", lock_path.display())
}

fn try_write_state_lock(lock_path: &Path, owner: &StateLockOwner) -> Result<bool> {
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating sync queue lock directory {}", parent.display()))?;
    }
    let payload = format!("{}\n{}\n", owner.pid, owner.token);
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("creating sync queue lock {}", lock_path.display()));
        }
    };
    file.write_all(payload.as_bytes())
        .with_context(|| format!("writing sync queue lock {}", lock_path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing sync queue lock {}", lock_path.display()))?;
    Ok(true)
}

fn clear_stale_state_lock(lock_path: &Path) -> Result<bool> {
    let Some(owner) = read_state_lock_owner(lock_path)? else {
        return Ok(false);
    };
    if process_is_running(owner.pid)? {
        return Ok(false);
    }
    fs::remove_file(lock_path)
        .or_else(|err| {
            if err.kind() == ErrorKind::NotFound {
                Ok(())
            } else {
                Err(err)
            }
        })
        .with_context(|| format!("removing stale sync queue lock {}", lock_path.display()))?;
    Ok(true)
}

fn read_state_lock_owner(lock_path: &Path) -> Result<Option<StateLockOwner>> {
    let payload = match fs::read_to_string(lock_path) {
        Ok(payload) => payload,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("reading {}", lock_path.display())),
    };
    let mut lines = payload.lines();
    let Some(pid) = lines.next() else {
        return Ok(None);
    };
    let Some(token) = lines.next() else {
        return Ok(None);
    };
    let Ok(pid) = pid.trim().parse::<u32>() else {
        return Ok(None);
    };
    Ok(Some(StateLockOwner {
        pid,
        token: token.trim().to_string(),
    }))
}
