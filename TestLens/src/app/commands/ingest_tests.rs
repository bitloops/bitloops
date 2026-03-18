use std::path::Path;

use anyhow::Result;

use crate::app::test_mapping;
use crate::repository::{TestHarnessRepository, open_sqlite_repository};

pub fn handle(db_path: &Path, repo_dir: &Path, commit_sha: &str) -> Result<()> {
    let mut repository = open_sqlite_repository(db_path)?;
    let repo_id = repository.load_repo_id_for_commit(commit_sha)?;
    let production = repository.load_production_artefacts(commit_sha)?;
    let mapping = test_mapping::execute(&repo_id, repo_dir, commit_sha, &production)?;

    repository.replace_test_discovery(commit_sha, &mapping.artefacts, &mapping.links)?;
    println!(
        "ingest-tests complete for commit {} (files: {}, suites: {}, scenarios: {}, links: {}, enumeration: {}, enumerated_scenarios: {})",
        commit_sha,
        mapping.stats.files,
        mapping.stats.suites,
        mapping.stats.scenarios,
        mapping.stats.links,
        mapping.enumeration_status,
        mapping.stats.enumerated_scenarios,
    );
    for note in mapping.enumeration_notes {
        println!("ingest-tests note: {note}");
    }
    for issue in mapping.issues {
        println!("ingest-tests issue: {} ({})", issue.message, issue.path);
    }
    Ok(())
}
