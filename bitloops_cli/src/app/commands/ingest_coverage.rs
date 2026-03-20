// Command handler for ingesting coverage data. Creates one coverage_captures row
// per invocation and N coverage_hits rows. No fan-out through test_links.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::domain::{
    CoverageCaptureRecord, CoverageDiagnosticRecord, CoverageFormat, CoverageHitRecord, ScopeKind,
};
use crate::repository::TestHarnessRepository;

#[derive(Debug, Clone)]
pub struct IngestCoverageSummary {
    pub format: CoverageFormat,
    pub scope_kind: ScopeKind,
    pub hits: usize,
    pub classifications: usize,
    pub diagnostics: usize,
}

#[derive(Debug, Clone)]
struct LcovFileCoverage {
    source_file: String,
    line_hits: HashMap<i64, i64>,
    branches: Vec<LcovBranchCoverage>,
}

#[derive(Debug, Clone, Copy)]
struct LcovBranchCoverage {
    line: i64,
    branch_id: i64,
    hit_count: i64,
}

pub fn execute(
    repository: &mut impl TestHarnessRepository,
    coverage_path: &Path,
    commit_sha: &str,
    scope_kind: ScopeKind,
    tool: &str,
    test_artefact_id: Option<&str>,
    format: CoverageFormat,
) -> Result<IngestCoverageSummary> {
    let repo_id = repository.load_repo_id_for_commit(commit_sha)?;

    let capture_id = format!(
        "capture:{commit_sha}:{}:{}",
        scope_kind,
        test_artefact_id.unwrap_or("all")
    );
    let captured_at = chrono_now();

    let has_branches = format == CoverageFormat::LlvmJson;

    let capture = CoverageCaptureRecord {
        capture_id: capture_id.clone(),
        repo_id: repo_id.clone(),
        commit_sha: commit_sha.to_string(),
        tool: tool.to_string(),
        format,
        scope_kind,
        subject_test_scenario_id: test_artefact_id.map(|s| s.to_string()),
        line_truth: true,
        branch_truth: has_branches,
        captured_at,
        status: "complete".to_string(),
        metadata_json: None,
    };

    let (hits, diagnostics) = match format {
        CoverageFormat::Lcov => {
            ingest_lcov(repository, coverage_path, commit_sha, &repo_id, &capture_id)?
        }
        CoverageFormat::LlvmJson => crate::app::commands::parse_llvm_json::ingest_llvm_json(
            repository,
            coverage_path,
            commit_sha,
            &repo_id,
            &capture_id,
        )?,
    };

    repository.insert_coverage_capture(&capture)?;
    repository.insert_coverage_hits(&hits)?;
    repository.insert_coverage_diagnostics(&diagnostics)?;

    let classifications = repository.rebuild_classifications_from_coverage(commit_sha)?;

    Ok(IngestCoverageSummary {
        format,
        scope_kind,
        hits: hits.len(),
        classifications,
        diagnostics: diagnostics.len(),
    })
}

pub fn print_summary(commit_sha: &str, summary: &IngestCoverageSummary) {
    println!(
        "ingested {} coverage for commit {} (scope: {}, hits: {}, classifications: {}, diagnostics: {})",
        summary.format,
        commit_sha,
        summary.scope_kind,
        summary.hits,
        summary.classifications,
        summary.diagnostics
    );
}

fn ingest_lcov(
    repository: &impl TestHarnessRepository,
    lcov_path: &Path,
    commit_sha: &str,
    repo_id: &str,
    capture_id: &str,
) -> Result<(Vec<CoverageHitRecord>, Vec<CoverageDiagnosticRecord>)> {
    let (report, parse_diagnostics) =
        parse_lcov_report(lcov_path, capture_id, repo_id, commit_sha)?;
    let mut hits = Vec::new();
    let mut diagnostics = parse_diagnostics;
    let mut diag_idx = diagnostics.len();

    for file in &report {
        let artefacts = repository.load_artefacts_for_file_lines(commit_sha, &file.source_file)?;
        if artefacts.is_empty() {
            diagnostics.push(CoverageDiagnosticRecord {
                diagnostic_id: format!("diag:{capture_id}:unmapped:{diag_idx}"),
                capture_id: capture_id.to_string(),
                repo_id: repo_id.to_string(),
                commit_sha: commit_sha.to_string(),
                path: Some(file.source_file.clone()),
                line: None,
                severity: "warning".to_string(),
                code: "unmapped_file".to_string(),
                message: format!(
                    "coverage file '{}' has no matching production artefacts",
                    file.source_file
                ),
                metadata_json: None,
            });
            diag_idx += 1;
            continue;
        }

        for (artefact_id, start_line, end_line) in &artefacts {
            for (&line_number, &hit_count) in &file.line_hits {
                if line_number < *start_line || line_number > *end_line {
                    continue;
                }
                hits.push(CoverageHitRecord {
                    capture_id: capture_id.to_string(),
                    production_artefact_id: artefact_id.clone(),
                    file_path: file.source_file.clone(),
                    line: line_number,
                    branch_id: -1,
                    covered: hit_count > 0,
                    hit_count,
                });
            }

            for branch in &file.branches {
                if branch.line < *start_line || branch.line > *end_line {
                    continue;
                }
                hits.push(CoverageHitRecord {
                    capture_id: capture_id.to_string(),
                    production_artefact_id: artefact_id.clone(),
                    file_path: file.source_file.clone(),
                    line: branch.line,
                    branch_id: branch.branch_id,
                    covered: branch.hit_count > 0,
                    hit_count: branch.hit_count,
                });
            }
        }
    }

    Ok((hits, diagnostics))
}

fn chrono_now() -> String {
    // Simple ISO-8601-ish timestamp without pulling in chrono crate
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", duration.as_secs())
}

fn parse_lcov_report(
    lcov_path: &Path,
    capture_id: &str,
    repo_id: &str,
    commit_sha: &str,
) -> Result<(Vec<LcovFileCoverage>, Vec<CoverageDiagnosticRecord>)> {
    let raw = fs::read_to_string(lcov_path)
        .with_context(|| format!("failed to read LCOV file {}", lcov_path.display()))?;

    let mut files = Vec::new();
    let mut diagnostics = Vec::new();
    let mut current: Option<LcovFileCoverage> = None;
    let mut diag_idx: usize = 0;

    let make_diag =
        |diag_idx: &mut usize, line_num: usize, source_file: Option<&str>, message: String| {
            let diag = CoverageDiagnosticRecord {
                diagnostic_id: format!("diag:{capture_id}:parse:{}", *diag_idx),
                capture_id: capture_id.to_string(),
                repo_id: repo_id.to_string(),
                commit_sha: commit_sha.to_string(),
                path: source_file.map(|s| s.to_string()),
                line: Some(line_num as i64),
                severity: "warning".to_string(),
                code: "malformed_line".to_string(),
                message,
                metadata_json: None,
            };
            *diag_idx += 1;
            diag
        };

    for (line_num, line) in raw.lines().enumerate() {
        let trimmed = line.trim();

        if let Some(path) = trimmed.strip_prefix("SF:") {
            if let Some(existing) = current.take() {
                files.push(existing);
            }
            current = Some(LcovFileCoverage {
                source_file: normalize_lcov_path(path),
                line_hits: HashMap::new(),
                branches: Vec::new(),
            });
            continue;
        }

        if trimmed == "end_of_record" {
            if let Some(existing) = current.take() {
                files.push(existing);
            }
            continue;
        }

        let Some(active) = current.as_mut() else {
            continue;
        };

        if let Some(da) = trimmed.strip_prefix("DA:") {
            let mut parts = da.splitn(3, ',');
            let (Some(line_no_raw), Some(hit_count_raw)) = (parts.next(), parts.next()) else {
                diagnostics.push(make_diag(
                    &mut diag_idx,
                    line_num + 1,
                    Some(&active.source_file),
                    format!("malformed DA line: '{trimmed}'"),
                ));
                continue;
            };
            let (Ok(line_no), Ok(hit_count)) =
                (line_no_raw.parse::<i64>(), hit_count_raw.parse::<i64>())
            else {
                diagnostics.push(make_diag(
                    &mut diag_idx,
                    line_num + 1,
                    Some(&active.source_file),
                    format!("unparseable DA values: '{trimmed}'"),
                ));
                continue;
            };
            active.line_hits.insert(line_no, hit_count);
            continue;
        }

        if let Some(brda) = trimmed.strip_prefix("BRDA:") {
            let parts: Vec<&str> = brda.split(',').collect();
            if parts.len() != 4 {
                diagnostics.push(make_diag(
                    &mut diag_idx,
                    line_num + 1,
                    Some(&active.source_file),
                    format!("malformed BRDA line (expected 4 fields): '{trimmed}'"),
                ));
                continue;
            }

            let (Ok(line_no), Ok(block_no), Ok(branch_no)) = (
                parts[0].parse::<i64>(),
                parts[1].parse::<i64>(),
                parts[2].parse::<i64>(),
            ) else {
                diagnostics.push(make_diag(
                    &mut diag_idx,
                    line_num + 1,
                    Some(&active.source_file),
                    format!("unparseable BRDA values: '{trimmed}'"),
                ));
                continue;
            };
            let hit_count = if parts[3] == "-" {
                0
            } else {
                parts[3].parse::<i64>().unwrap_or(0)
            };

            active.branches.push(LcovBranchCoverage {
                line: line_no,
                branch_id: block_no.saturating_mul(1000).saturating_add(branch_no),
                hit_count,
            });
        }
    }

    if let Some(existing) = current.take() {
        files.push(existing);
    }

    if files.is_empty() {
        anyhow::bail!(
            "no LCOV file records found in {} (expected at least one SF section)",
            lcov_path.display()
        );
    }

    Ok((files, diagnostics))
}

fn normalize_lcov_path(path: &str) -> String {
    path.trim().replace('\\', "/")
}
