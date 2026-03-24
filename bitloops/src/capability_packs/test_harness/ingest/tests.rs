use std::path::Path;

use anyhow::Result;

use crate::capability_packs::test_harness::mapping;
use crate::capability_packs::test_harness::storage::TestHarnessRepository;
use crate::host::capability_host::gateways::RelationalGateway;
use crate::models::{TestDiscoveryDiagnosticRecord, TestDiscoveryRunRecord};

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

pub fn execute(
    repository: &mut impl TestHarnessRepository,
    relational: &dyn RelationalGateway,
    repo_dir: &Path,
    commit_sha: &str,
) -> Result<IngestTestsSummary> {
    let repo_id = relational.load_repo_id_for_commit(commit_sha)?;
    let production = relational.load_production_artefacts(commit_sha)?;
    let started_at = chrono::Utc::now().to_rfc3339();
    let mapping = mapping::execute(&repo_id, repo_dir, commit_sha, &production)?;
    let finished_at = chrono::Utc::now().to_rfc3339();

    let discovery_run_id = format!("discovery:{commit_sha}");
    let stats_json = serde_json::json!({
        "files": mapping.stats.files,
        "test_artefacts": mapping.stats.test_artefacts,
        "test_edges": mapping.stats.test_edges,
        "enumerated_scenarios": mapping.stats.enumerated_scenarios,
    })
    .to_string();
    let discovery_run = TestDiscoveryRunRecord {
        discovery_run_id: discovery_run_id.clone(),
        repo_id: repo_id.clone(),
        commit_sha: commit_sha.to_string(),
        language: None,
        started_at,
        finished_at: Some(finished_at),
        status: "complete".to_string(),
        enumeration_status: Some(mapping.enumeration_status.clone()),
        notes_json: Some(serde_json::to_string(&mapping.enumeration_notes)?),
        stats_json: Some(stats_json),
    };
    let diagnostics: Vec<TestDiscoveryDiagnosticRecord> = mapping
        .issues
        .iter()
        .enumerate()
        .map(|(idx, issue)| TestDiscoveryDiagnosticRecord {
            diagnostic_id: format!("diagnostic:{commit_sha}:{idx}"),
            discovery_run_id: discovery_run_id.clone(),
            repo_id: repo_id.clone(),
            commit_sha: commit_sha.to_string(),
            path: Some(issue.path.clone()),
            line: None,
            severity: "warning".to_string(),
            code: "mapping_issue".to_string(),
            message: issue.message.clone(),
            metadata_json: None,
        })
        .collect();

    repository.replace_test_discovery(
        commit_sha,
        &mapping.test_artefacts,
        &mapping.test_edges,
        &discovery_run,
        &diagnostics,
    )?;

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
