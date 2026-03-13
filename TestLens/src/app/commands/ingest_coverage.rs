// Command handler for ingesting coverage data, materializing coverage rows, and
// deriving commit-scoped test classifications.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::domain::TestCoverageRecord;
use crate::repository::{TestHarnessRepository, open_sqlite_repository};

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

#[derive(Debug, Clone, Copy)]
struct IngestStats {
    files: usize,
    rows_inserted: usize,
}

pub fn handle(db_path: &Path, lcov_path: &Path, commit_sha: &str) -> Result<()> {
    let mut repository = open_sqlite_repository(db_path)?;

    let report = parse_lcov_report(lcov_path)?;
    let links = repository.load_test_links_by_production_artefact(commit_sha)?;

    let mut rows_inserted = 0usize;
    let mut coverage_rows = Vec::new();

    for file in &report {
        let targets = repository.load_coverage_targets_for_file(commit_sha, &file.source_file)?;
        if targets.is_empty() {
            continue;
        }

        for target in targets {
            let Some(test_artefacts) = links.get(&target.artefact_id) else {
                continue;
            };

            for (&line_number, &hit_count) in &file.line_hits {
                if line_number < target.start_line || line_number > target.end_line {
                    continue;
                }

                for test_artefact_id in test_artefacts {
                    let coverage_id = format!(
                        "line:{commit_sha}:{test_artefact_id}:{}:{line_number}",
                        target.artefact_id
                    );
                    coverage_rows.push(TestCoverageRecord {
                        coverage_id,
                        repo_id: target.repo_id.clone(),
                        commit_sha: commit_sha.to_string(),
                        test_artefact_id: test_artefact_id.clone(),
                        artefact_id: target.artefact_id.clone(),
                        line: line_number,
                        branch_id: None,
                        covered: hit_count > 0,
                        hit_count,
                    });
                    rows_inserted += 1;
                }
            }

            for branch in &file.branches {
                if branch.line < target.start_line || branch.line > target.end_line {
                    continue;
                }

                for test_artefact_id in test_artefacts {
                    let coverage_id = format!(
                        "branch:{commit_sha}:{test_artefact_id}:{}:{}:{}",
                        target.artefact_id, branch.line, branch.branch_id
                    );
                    coverage_rows.push(TestCoverageRecord {
                        coverage_id,
                        repo_id: target.repo_id.clone(),
                        commit_sha: commit_sha.to_string(),
                        test_artefact_id: test_artefact_id.clone(),
                        artefact_id: target.artefact_id.clone(),
                        line: branch.line,
                        branch_id: Some(branch.branch_id),
                        covered: branch.hit_count > 0,
                        hit_count: branch.hit_count,
                    });
                    rows_inserted += 1;
                }
            }
        }
    }

    repository.replace_test_coverage(commit_sha, &coverage_rows)?;
    let classifications = repository.rebuild_classifications_from_coverage(commit_sha)?;

    let stats = IngestStats {
        files: report.len(),
        rows_inserted,
    };
    println!(
        "ingested LCOV for commit {} (files parsed: {}, coverage rows upserted: {}, classifications derived: {})",
        commit_sha, stats.files, stats.rows_inserted, classifications
    );
    Ok(())
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
