// Command handler for batch coverage ingestion via a JSON manifest file.
// Each entry in the manifest is ingested as a separate coverage capture.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::domain::BatchManifestEntry;

pub fn handle(db_path: &Path, manifest_path: &Path, commit_sha: &str) -> Result<()> {
    let raw = fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read manifest file {}", manifest_path.display()))?;

    let entries: Vec<BatchManifestEntry> = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse manifest JSON from {}", manifest_path.display()))?;

    if entries.is_empty() {
        anyhow::bail!("manifest is empty — expected at least one entry");
    }

    let manifest_dir = manifest_path.parent().unwrap_or(Path::new("."));

    for (i, entry) in entries.iter().enumerate() {
        let coverage_path = manifest_dir.join(&entry.path);
        if !coverage_path.exists() {
            anyhow::bail!(
                "manifest entry {} references non-existent file: {}",
                i,
                coverage_path.display()
            );
        }

        super::ingest_coverage::handle(
            db_path,
            None,
            Some(&coverage_path),
            commit_sha,
            &entry.scope,
            &entry.tool,
            entry.test_artefact_id.as_deref(),
            Some(&entry.format),
        )?;
    }

    println!(
        "batch ingested {} coverage entries for commit {}",
        entries.len(),
        commit_sha,
    );
    Ok(())
}
