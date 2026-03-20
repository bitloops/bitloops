// Command handler for ingesting coverage data. Creates one coverage_captures row
// per invocation and N coverage_hits rows. No fan-out through test_links.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::domain::{CoverageCaptureRecord, CoverageFormat, CoverageHitRecord, ScopeKind};
use crate::repository::{TestHarnessRepository, open_sqlite_repository};

#[derive(Debug, Clone)]
pub struct IngestCoverageSummary {
    pub format: CoverageFormat,
    pub scope_kind: ScopeKind,
    pub hits: usize,
    pub classifications: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct IngestCoverageRequest<'a> {
    pub lcov_path: Option<&'a Path>,
    pub input_path: Option<&'a Path>,
    pub commit_sha: &'a str,
    pub scope_str: &'a str,
    pub tool: &'a str,
    pub test_artefact_id: Option<&'a str>,
    pub format_str: Option<&'a str>,
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

pub fn handle(db_path: &Path, request: IngestCoverageRequest<'_>) -> Result<()> {
    let scope_kind = request.scope_str.parse::<ScopeKind>().map_err(|_| {
        anyhow::anyhow!(
            "invalid scope: {} (expected workspace, package, test-scenario, or doctest)",
            request.scope_str
        )
    })?;

    // Determine the coverage file path
    let coverage_path = request
        .lcov_path
        .or(request.input_path)
        .ok_or_else(|| anyhow::anyhow!("either --lcov or --input must be provided"))?;

    // Determine format from explicit flag, file extension, or default
    let format = resolve_format(request.format_str, coverage_path)?;

    // Validate constraints
    if scope_kind == ScopeKind::TestScenario {
        if request.test_artefact_id.is_none() {
            anyhow::bail!("--test-artefact-id is required when scope is test-scenario");
        }
        if format == CoverageFormat::Lcov {
            anyhow::bail!(
                "LCOV format is not supported for scope=test-scenario (too lossy for per-test attribution); use --format llvm-json"
            );
        }
    }

    let mut repository = open_sqlite_repository(db_path)?;
    let summary = execute(
        &mut repository,
        coverage_path,
        request.commit_sha,
        scope_kind,
        request.tool,
        request.test_artefact_id,
        format,
    )?;

    print_summary(request.commit_sha, &summary);
    Ok(())
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

    let hits = match format {
        CoverageFormat::Lcov => ingest_lcov(repository, coverage_path, commit_sha, &capture_id)?,
        CoverageFormat::LlvmJson => crate::app::commands::parse_llvm_json::ingest_llvm_json(
            repository,
            coverage_path,
            commit_sha,
            &capture_id,
        )?,
    };

    repository.insert_coverage_capture(&capture)?;
    repository.insert_coverage_hits(&hits)?;

    let classifications = repository.rebuild_classifications_from_coverage(commit_sha)?;

    Ok(IngestCoverageSummary {
        format,
        scope_kind,
        hits: hits.len(),
        classifications,
    })
}

pub fn print_summary(commit_sha: &str, summary: &IngestCoverageSummary) {
    println!(
        "ingested {} coverage for commit {} (scope: {}, hits: {}, classifications: {})",
        summary.format, commit_sha, summary.scope_kind, summary.hits, summary.classifications
    );
}

fn ingest_lcov(
    repository: &impl TestHarnessRepository,
    lcov_path: &Path,
    commit_sha: &str,
    capture_id: &str,
) -> Result<Vec<CoverageHitRecord>> {
    let report = parse_lcov_report(lcov_path)?;
    let mut hits = Vec::new();

    for file in &report {
        let artefacts = repository.load_artefacts_for_file_lines(commit_sha, &file.source_file)?;
        if artefacts.is_empty() {
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

    Ok(hits)
}

fn resolve_format(format_str: Option<&str>, path: &Path) -> Result<CoverageFormat> {
    if let Some(fmt) = format_str {
        return fmt
            .parse::<CoverageFormat>()
            .map_err(|_| anyhow::anyhow!("unknown format: {fmt} (expected lcov or llvm-json)"));
    }

    // Auto-detect from extension
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "json" => Ok(CoverageFormat::LlvmJson),
        _ => Ok(CoverageFormat::Lcov),
    }
}

fn chrono_now() -> String {
    // Simple ISO-8601-ish timestamp without pulling in chrono crate
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", duration.as_secs())
}

fn parse_lcov_report(lcov_path: &Path) -> Result<Vec<LcovFileCoverage>> {
    let raw = fs::read_to_string(lcov_path)
        .with_context(|| format!("failed to read LCOV file {}", lcov_path.display()))?;

    let mut files = Vec::new();
    let mut current: Option<LcovFileCoverage> = None;

    for line in raw.lines() {
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
            let Some(line_no_raw) = parts.next() else {
                continue;
            };
            let Some(hit_count_raw) = parts.next() else {
                continue;
            };
            let Ok(line_no) = line_no_raw.parse::<i64>() else {
                continue;
            };
            let Ok(hit_count) = hit_count_raw.parse::<i64>() else {
                continue;
            };
            active.line_hits.insert(line_no, hit_count);
            continue;
        }

        if let Some(brda) = trimmed.strip_prefix("BRDA:") {
            let parts: Vec<&str> = brda.split(',').collect();
            if parts.len() != 4 {
                continue;
            }

            let Ok(line_no) = parts[0].parse::<i64>() else {
                continue;
            };
            let Ok(block_no) = parts[1].parse::<i64>() else {
                continue;
            };
            let Ok(branch_no) = parts[2].parse::<i64>() else {
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

    Ok(files)
}

fn normalize_lcov_path(path: &str) -> String {
    path.trim().replace('\\', "/")
}
