// Command handler for batch coverage ingestion via a JSON manifest file.
// Each entry in the manifest is ingested as a separate coverage capture.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::engine::devql::capability_host::gateways::TestHarnessCoverageGateway;
use crate::models::{BatchManifestEntry, CoverageFormat, ScopeKind};

#[derive(Debug, Clone)]
pub struct IngestCoverageBatchSummary {
    pub entries: usize,
}

pub fn execute(
    store: &mut impl TestHarnessCoverageGateway,
    manifest_path: &Path,
    commit_sha: &str,
) -> Result<IngestCoverageBatchSummary> {
    let entries = parse_manifest_entries(manifest_path)?;
    let manifest_dir = manifest_path.parent().unwrap_or(Path::new("."));

    for (index, entry) in entries.iter().enumerate() {
        let coverage_path = manifest_dir.join(&entry.path);
        if !coverage_path.exists() {
            anyhow::bail!(
                "manifest entry {} references non-existent file: {}",
                index,
                coverage_path.display()
            );
        }

        let scope_kind = entry.scope.parse::<ScopeKind>().map_err(|_| {
            anyhow::anyhow!(
                "invalid scope: {} (expected workspace, package, test-scenario, or doctest)",
                entry.scope
            )
        })?;
        let format = entry.format.parse::<CoverageFormat>().map_err(|_| {
            anyhow::anyhow!(
                "unknown format: {} (expected lcov or llvm-json)",
                entry.format
            )
        })?;

        crate::app::commands::ingest_coverage::execute(
            store,
            &coverage_path,
            commit_sha,
            scope_kind,
            &entry.tool,
            entry.test_artefact_id.as_deref(),
            format,
        )?;
    }

    Ok(IngestCoverageBatchSummary {
        entries: entries.len(),
    })
}

pub fn print_summary(commit_sha: &str, summary: &IngestCoverageBatchSummary) {
    println!(
        "batch ingested {} coverage entries for commit {}",
        summary.entries, commit_sha
    );
}

pub fn parse_manifest_entries(manifest_path: &Path) -> Result<Vec<BatchManifestEntry>> {
    let raw = fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read manifest file {}", manifest_path.display()))?;

    let entries: Vec<BatchManifestEntry> = serde_json::from_str(&raw).with_context(|| {
        format!(
            "failed to parse manifest JSON from {}",
            manifest_path.display()
        )
    })?;

    if entries.is_empty() {
        anyhow::bail!("manifest is empty — expected at least one entry");
    }

    Ok(entries)
}
