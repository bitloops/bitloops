use std::path::Path;

use anyhow::Result;

use crate::cli::QueryViewArg;
use crate::read::query_test_harness;

pub fn handle(
    db_path: &Path,
    artefact_query: &str,
    commit_sha: &str,
    classification_filter: Option<&str>,
    view: QueryViewArg,
    min_strength: Option<f64>,
) -> Result<()> {
    query_test_harness::query_artefact_harness(
        db_path,
        artefact_query,
        commit_sha,
        classification_filter,
        view,
        min_strength,
    )
}
