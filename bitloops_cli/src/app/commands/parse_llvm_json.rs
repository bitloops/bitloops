// Parser for LLVM JSON coverage export format (cargo-llvm-cov --json).

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::domain::{CoverageDiagnosticRecord, CoverageHitRecord};
use crate::engine::devql::capability_host::gateways::TestHarnessCoverageGateway;

#[derive(Debug, Deserialize)]
struct LlvmCoverageExport {
    data: Vec<LlvmCoverageData>,
}

#[derive(Debug, Deserialize)]
struct LlvmCoverageData {
    files: Vec<LlvmCoverageFile>,
}

#[derive(Debug, Deserialize)]
struct LlvmCoverageFile {
    filename: String,
    segments: Vec<Vec<serde_json::Value>>,
}

/// A decoded segment: [line, col, count, has_count, is_region_entry, is_gap_region]
struct Segment {
    line: i64,
    count: i64,
    has_count: bool,
}

pub fn ingest_llvm_json(
    store: &dyn TestHarnessCoverageGateway,
    json_path: &Path,
    commit_sha: &str,
    repo_id: &str,
    capture_id: &str,
) -> Result<(Vec<CoverageHitRecord>, Vec<CoverageDiagnosticRecord>)> {
    let raw = fs::read_to_string(json_path)
        .with_context(|| format!("failed to read LLVM JSON file {}", json_path.display()))?;

    let export: LlvmCoverageExport = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse LLVM JSON from {}", json_path.display()))?;

    let mut hits = Vec::new();
    let mut diagnostics = Vec::new();
    let mut diag_idx: usize = 0;

    for data in &export.data {
        for file in &data.files {
            let line_hits = extract_line_hits(&file.segments);
            let artefacts = store.load_artefacts_for_file_lines(commit_sha, &file.filename)?;

            if artefacts.is_empty() {
                diagnostics.push(CoverageDiagnosticRecord {
                    diagnostic_id: format!("diag:{capture_id}:unmapped:{diag_idx}"),
                    capture_id: capture_id.to_string(),
                    repo_id: repo_id.to_string(),
                    commit_sha: commit_sha.to_string(),
                    path: Some(file.filename.clone()),
                    line: None,
                    severity: "warning".to_string(),
                    code: "unmapped_file".to_string(),
                    message: format!(
                        "coverage file '{}' has no matching production artefacts",
                        file.filename
                    ),
                    metadata_json: None,
                });
                diag_idx += 1;
                continue;
            }

            for (artefact_id, start_line, end_line) in &artefacts {
                for (line, count) in &line_hits {
                    if *line < *start_line || *line > *end_line {
                        continue;
                    }
                    hits.push(CoverageHitRecord {
                        capture_id: capture_id.to_string(),
                        production_artefact_id: artefact_id.clone(),
                        file_path: file.filename.clone(),
                        line: *line,
                        branch_id: -1,
                        covered: *count > 0,
                        hit_count: *count,
                    });
                }
            }
        }
    }

    Ok((hits, diagnostics))
}

/// Extract per-line hit counts from LLVM JSON segments.
/// Each segment = [line, col, count, has_count, is_region_entry, is_gap_region].
/// Segments define ranges: each segment's count applies from its line until the next segment's line.
fn extract_line_hits(segments: &[Vec<serde_json::Value>]) -> Vec<(i64, i64)> {
    let parsed: Vec<Segment> = segments
        .iter()
        .filter_map(|seg| {
            if seg.len() < 4 {
                return None;
            }
            let line = seg[0].as_i64()?;
            let count = seg[2].as_i64().unwrap_or(0);
            let has_count = seg[3]
                .as_bool()
                .or_else(|| seg[3].as_i64().map(|v| v != 0))?;
            Some(Segment {
                line,
                count,
                has_count,
            })
        })
        .collect();

    let mut line_hits: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();

    for i in 0..parsed.len() {
        let seg = &parsed[i];
        if !seg.has_count {
            continue;
        }

        let start = seg.line;
        let end = if i + 1 < parsed.len() {
            parsed[i + 1].line
        } else {
            start + 1
        };

        for line in start..end {
            let entry = line_hits.entry(line).or_insert(0);
            *entry = (*entry).max(seg.count);
        }
    }

    let mut result: Vec<(i64, i64)> = line_hits.into_iter().collect();
    result.sort_by_key(|(line, _)| *line);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_line_hits_from_segments() {
        let segments = vec![
            vec![
                serde_json::json!(10),
                serde_json::json!(1),
                serde_json::json!(5),
                serde_json::json!(true),
                serde_json::json!(true),
                serde_json::json!(false),
            ],
            vec![
                serde_json::json!(13),
                serde_json::json!(1),
                serde_json::json!(0),
                serde_json::json!(true),
                serde_json::json!(true),
                serde_json::json!(false),
            ],
        ];

        let hits = extract_line_hits(&segments);
        // Lines 10, 11, 12 should have count=5; line 13 should have count=0
        assert_eq!(hits.len(), 4);
        assert!(hits.contains(&(10, 5)));
        assert!(hits.contains(&(11, 5)));
        assert!(hits.contains(&(12, 5)));
        assert!(hits.contains(&(13, 0)));
    }
}
