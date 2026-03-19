use std::path::Path;

use anyhow::Result;

use crate::db;

pub fn handle(db_path: &Path, seed: bool, commit_sha: &str) -> Result<()> {
    db::init_database(db_path, seed, commit_sha)
}
