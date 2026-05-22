use std::path::{Path, PathBuf};

mod current_state;
mod filesystem;
mod introspection;
mod phase;
mod schema;
mod write_lock;

#[cfg(test)]
mod tests;

pub(crate) use phase::SqliteWritePhaseMetrics;
#[cfg(test)]
pub(crate) use write_lock::hold_sqlite_write_lock_until_release;
pub(crate) use write_lock::{
    with_sqlite_write_lock, with_sqlite_write_lock_map, with_sqlite_write_phase,
};

#[derive(Debug, Clone)]
pub struct SqliteConnectionPool {
    db_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ReadOnlySqliteConnectionPool {
    db_path: PathBuf,
}

impl SqliteConnectionPool {
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

impl ReadOnlySqliteConnectionPool {
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}
