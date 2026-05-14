use std::path::{Path, PathBuf};

mod current_state;
mod filesystem;
mod introspection;
mod schema;
mod write_lock;

#[cfg(test)]
mod tests;

pub(crate) use write_lock::{with_sqlite_write_lock, with_sqlite_write_lock_map};

#[derive(Debug, Clone)]
pub struct SqliteConnectionPool {
    db_path: PathBuf,
}

impl SqliteConnectionPool {
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}
