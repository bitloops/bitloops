use std::path::Path;

use anyhow::Result;

use crate::storage::init;

pub fn handle(db_path: &Path, seed: bool, commit_sha: &str) -> Result<()> {
    init::init_database(db_path, seed, commit_sha)
}
