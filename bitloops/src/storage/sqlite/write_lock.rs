use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};

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
    let _process_guard = match process_lock.lock() {
        Ok(guard) => guard,
        Err(_) => {
            return Err(map_lock_error(anyhow!(
                "locking SQLite process write guard"
            )));
        }
    };

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
    if let Ok(canonical) = db_path.canonicalize() {
        return canonical;
    }
    if let (Some(parent), Some(file_name)) = (db_path.parent(), db_path.file_name())
        && let Ok(parent) = parent.canonicalize()
    {
        return parent.join(file_name);
    }
    db_path.to_path_buf()
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
