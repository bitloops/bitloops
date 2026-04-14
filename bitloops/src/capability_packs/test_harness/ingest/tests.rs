//! Legacy commit-scoped test-harness ingestion.
//!
//! Prefer automatic current-state sync for workspace validation and current-tree
//! test linkage queries. Keep this path only for historical or commit-scoped
//! materialization where the caller explicitly targets a commit SHA.

use std::path::Path;

use anyhow::Result;

use crate::capability_packs::test_harness::mapping;
use crate::capability_packs::test_harness::storage::TestHarnessRepository;
use crate::host::capability_host::gateways::LanguageServicesGateway;
use crate::host::capability_host::gateways::RelationalGateway;

#[derive(Debug, Clone)]
pub struct IngestTestsIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct IngestTestsSummary {
    pub files: usize,
    pub test_artefacts: usize,
    pub test_edges: usize,
    pub enumeration_status: String,
    pub enumerated_scenarios: usize,
    pub notes: Vec<String>,
    pub issues: Vec<IngestTestsIssue>,
}

/// Materialize test discovery/linkage for a specific historical commit.
///
/// This is the legacy commit-scoped path. Automatic current-state sync should be
/// the default for validating the active workspace state.
pub fn execute(
    repository: &mut impl TestHarnessRepository,
    relational: &dyn RelationalGateway,
    repo_dir: &Path,
    commit_sha: &str,
    languages: &dyn LanguageServicesGateway,
) -> Result<IngestTestsSummary> {
    let repo_id = relational.load_repo_id_for_commit(commit_sha)?;
    let production = relational.load_production_artefacts(commit_sha)?;
    let mapping = mapping::execute(&repo_id, repo_dir, commit_sha, &production, languages)?;

    repository.replace_test_discovery(commit_sha, &mapping.test_artefacts, &mapping.test_edges)?;

    Ok(IngestTestsSummary {
        files: mapping.stats.files,
        test_artefacts: mapping.stats.test_artefacts,
        test_edges: mapping.stats.test_edges,
        enumeration_status: mapping.enumeration_status,
        enumerated_scenarios: mapping.stats.enumerated_scenarios,
        notes: mapping.enumeration_notes,
        issues: mapping
            .issues
            .into_iter()
            .map(|issue| IngestTestsIssue {
                path: issue.path,
                message: issue.message,
            })
            .collect(),
    })
}

pub fn format_summary(commit_sha: &str, summary: &IngestTestsSummary) -> String {
    let mut out = format!(
        "ingest-tests complete for commit {} (files: {}, test_artefacts: {}, test_edges: {}, enumeration: {}, enumerated_scenarios: {})",
        commit_sha,
        summary.files,
        summary.test_artefacts,
        summary.test_edges,
        summary.enumeration_status,
        summary.enumerated_scenarios,
    );
    for note in &summary.notes {
        out.push_str(&format!("\ningest-tests note: {note}"));
    }
    for issue in &summary.issues {
        out.push_str(&format!(
            "\ningest-tests issue: {} ({})",
            issue.message, issue.path
        ));
    }
    out
}

pub fn print_summary(commit_sha: &str, summary: &IngestTestsSummary) {
    println!("{}", format_summary(commit_sha, summary));
}
