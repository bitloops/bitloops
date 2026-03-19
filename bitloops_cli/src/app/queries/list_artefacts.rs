use std::path::Path;

use anyhow::Result;

use crate::read::query_test_harness;

pub fn handle(db_path: &Path, commit_sha: &str, kind: Option<&str>) -> Result<()> {
    query_test_harness::list_artefacts(db_path, commit_sha, kind)
}
