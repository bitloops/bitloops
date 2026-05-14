use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

const WRITE_LOCK_WAIT_WARN_THRESHOLD: Duration = Duration::from_secs(1);
const WRITE_LOCK_HOLD_WARN_THRESHOLD: Duration = Duration::from_secs(5);

thread_local! {
    static HELD_SQLITE_WRITE_LOCKS: RefCell<HashMap<PathBuf, usize>> = RefCell::new(HashMap::new());
}

pub(crate) fn with_sqlite_write_lock<T>(
    db_path: &Path,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    with_sqlite_write_lock_map(db_path, |err| err, operation)
}

pub(crate) fn with_sqlite_write_lock_map<T, E>(
    db_path: &Path,
    map_lock_error: impl Fn(anyhow::Error) -> E,
    operation: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    let canonical_db_path = canonical_sqlite_db_path(db_path);
    if sqlite_write_lock_is_held(&canonical_db_path) {
        return operation();
    }

    let started = Instant::now();
    let process_lock = sqlite_process_write_lock_for(&canonical_db_path);
    let _process_guard = process_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let lock_path = sqlite_write_lock_path(&canonical_db_path);
    let lock_file = match open_sqlite_write_lock_file(&lock_path) {
        Ok(file) => file,
        Err(err) => return Err(map_lock_error(err)),
    };
    let _file_guard = match lock_sqlite_write_file(lock_file, &lock_path) {
        Ok(guard) => guard,
        Err(err) => return Err(map_lock_error(err)),
    };
    let waited = started.elapsed();

    let _thread_guard = ThreadSqliteWriteLockGuard::new(canonical_db_path.clone());
    let hold_started = Instant::now();
    let result = operation();
    let held = hold_started.elapsed();

    log_sqlite_write_lock_timing(&canonical_db_path, waited, held);
    result
}

fn sqlite_write_lock_is_held(db_path: &Path) -> bool {
    HELD_SQLITE_WRITE_LOCKS.with(|held| held.borrow().contains_key(db_path))
}

struct ThreadSqliteWriteLockGuard {
    db_path: PathBuf,
}

impl ThreadSqliteWriteLockGuard {
    fn new(db_path: PathBuf) -> Self {
        HELD_SQLITE_WRITE_LOCKS.with(|held| {
            let mut held = held.borrow_mut();
            *held.entry(db_path.clone()).or_insert(0) += 1;
        });
        Self { db_path }
    }
}

impl Drop for ThreadSqliteWriteLockGuard {
    fn drop(&mut self) {
        HELD_SQLITE_WRITE_LOCKS.with(|held| {
            let mut held = held.borrow_mut();
            if let Some(depth) = held.get_mut(&self.db_path) {
                if *depth > 1 {
                    *depth -= 1;
                } else {
                    held.remove(&self.db_path);
                }
            }
        });
    }
}

fn sqlite_process_write_lock_for(db_path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks
        .entry(db_path.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn canonical_sqlite_db_path(db_path: &Path) -> PathBuf {
    // Keep the lock key stable even while the database or parent directory is being created.
    // Filesystem canonicalization can change once those paths appear.
    let absolute = if db_path.is_absolute() {
        db_path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(db_path)
    };
    normalize_sqlite_lock_path(&absolute)
}

fn normalize_sqlite_lock_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn sqlite_write_lock_path(db_path: &Path) -> PathBuf {
    let file_name = db_path
        .file_name()
        .map(|name| format!("{}-write-lock", name.to_string_lossy()))
        .unwrap_or_else(|| "sqlite-write-lock".to_string());
    db_path.with_file_name(file_name)
}

fn open_sqlite_write_lock_file(lock_path: &Path) -> Result<File> {
    if let Some(parent) = lock_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("creating SQLite write lock directory {}", parent.display())
        })?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .with_context(|| format!("opening SQLite write lock file {}", lock_path.display()))
}

#[cfg(unix)]
fn lock_sqlite_write_file(file: File, lock_path: &Path) -> Result<SqliteWriteFileGuard> {
    use std::os::fd::AsRawFd;

    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if rc == 0 {
        return Ok(SqliteWriteFileGuard { file });
    }
    Err(std::io::Error::last_os_error())
        .with_context(|| format!("locking SQLite write lock file {}", lock_path.display()))
}

#[cfg(not(unix))]
fn lock_sqlite_write_file(file: File, _lock_path: &Path) -> Result<SqliteWriteFileGuard> {
    Ok(SqliteWriteFileGuard { file })
}

struct SqliteWriteFileGuard {
    file: File,
}

#[cfg(unix)]
impl Drop for SqliteWriteFileGuard {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;

        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

fn log_sqlite_write_lock_timing(db_path: &Path, waited: Duration, held: Duration) {
    if waited >= WRITE_LOCK_WAIT_WARN_THRESHOLD {
        log::warn!(
            "waited {}ms for SQLite write lock on {}",
            waited.as_millis(),
            db_path.display()
        );
    } else {
        log::debug!(
            "waited {}ms for SQLite write lock on {}",
            waited.as_millis(),
            db_path.display()
        );
    }

    if held >= WRITE_LOCK_HOLD_WARN_THRESHOLD {
        log::warn!(
            "held SQLite write lock for {}ms on {}",
            held.as_millis(),
            db_path.display()
        );
    } else {
        log::debug!(
            "held SQLite write lock for {}ms on {}",
            held.as_millis(),
            db_path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::sync::Arc;

    #[test]
    fn canonical_sqlite_db_path_is_stable_when_parent_is_created_later() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let anchor = temp.path().join("anchor");
        std::fs::create_dir_all(&anchor)?;
        let db_path = anchor.join("..").join("missing").join("runtime.sqlite");

        let before_parent_exists = canonical_sqlite_db_path(&db_path);
        std::fs::create_dir_all(temp.path().join("missing"))?;
        let after_parent_exists = canonical_sqlite_db_path(&db_path);

        assert_eq!(before_parent_exists, after_parent_exists);
        assert!(before_parent_exists.is_absolute());
        Ok(())
    }

    #[test]
    fn sqlite_write_lock_recovers_from_poisoned_process_mutex() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let db_path = temp.path().join("runtime.sqlite");
        let lock_key = canonical_sqlite_db_path(&db_path);
        let process_lock = sqlite_process_write_lock_for(&lock_key);
        let lock_for_thread = Arc::clone(&process_lock);

        let _ = std::thread::spawn(move || {
            let _guard = lock_for_thread.lock().expect("lock process mutex");
            panic!("poison SQLite process write lock");
        })
        .join();

        let value = with_sqlite_write_lock(&db_path, || Ok(42))?;
        assert_eq!(value, 42);
        Ok(())
    }
}
