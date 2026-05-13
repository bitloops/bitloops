use std::path::{Path, PathBuf};

mod current_state;
mod filesystem;
mod introspection;
mod schema;

#[cfg(test)]
mod tests;

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
