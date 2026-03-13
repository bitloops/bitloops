// Read-side workflow for composing the test harness view returned by `query`
// and `list` commands.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::domain::{CoverageSummaryRecord, QueriedArtefactRecord};
use crate::repository::{TestHarnessQueryRepository, open_sqlite_repository};

#[derive(Debug, Serialize)]
struct QueryResponse {
    artefact: ArtefactOutput,
    covering_tests: Vec<CoveringTestOutput>,
    coverage: Option<CoverageOutput>,
    summary: SummaryOutput,
}

#[derive(Debug, Serialize)]
struct ArtefactOutput {
    artefact_id: String,
    name: String,
    kind: String,
    file_path: String,
    start_line: i64,
    end_line: i64,
}

#[derive(Debug, Serialize)]
struct CoveringTestOutput {
    test_id: String,
    test_name: String,
    suite_name: Option<String>,
    file_path: String,
    classification: String,
    classification_source: String,
    confidence: f64,
    strength: f64,
    last_run: Option<LastRunOutput>,
}

#[derive(Debug, Serialize)]
struct LastRunOutput {
    status: String,
    duration_ms: Option<i64>,
    commit_sha: String,
}

#[derive(Debug, Serialize)]
struct CoverageOutput {
    line_coverage_pct: f64,
    branch_coverage_pct: f64,
    branches: Vec<BranchOutput>,
}

#[derive(Debug, Clone, Serialize)]
struct BranchOutput {
    line: i64,
    description: String,
    covered: bool,
    covering_test_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SummaryOutput {
    verification_level: String,
    total_covering_tests: usize,
    unit_count: usize,
    integration_count: usize,
    e2e_count: usize,
    untested_branch_count: usize,
}

#[derive(Debug, Serialize)]
struct ListArtefactOutput {
    artefact_id: String,
    symbol_fqn: Option<String>,
    kind: String,
    file_path: String,
    start_line: i64,
    end_line: i64,
}

pub fn query_artefact_harness(
    db_path: &Path,
    artefact_query: &str,
    commit_sha: &str,
    classification_filter: Option<&str>,
) -> Result<()> {
    let repository = open_sqlite_repository(db_path)?;
    let artefact = repository.find_artefact(commit_sha, artefact_query)?;
    let response =
        build_tests_query_response(&repository, &artefact, commit_sha, classification_filter)?;
    let json = serde_json::to_string_pretty(&response).context("failed serializing query JSON")?;
    println!("{json}");
    Ok(())
}

pub fn list_artefacts(db_path: &Path, commit_sha: &str, kind: Option<&str>) -> Result<()> {
    let repository = open_sqlite_repository(db_path)?;
    let artefacts = repository.list_artefacts(commit_sha, kind)?;
    let output: Vec<ListArtefactOutput> = artefacts
        .into_iter()
        .map(|artefact| ListArtefactOutput {
            artefact_id: artefact.artefact_id,
            symbol_fqn: artefact.symbol_fqn,
            kind: artefact.kind,
            file_path: artefact.file_path,
            start_line: artefact.start_line,
            end_line: artefact.end_line,
        })
        .collect();

    let json = serde_json::to_string_pretty(&output).context("failed serializing list JSON")?;
    println!("{json}");
    Ok(())
}

fn build_tests_query_response<R: TestHarnessQueryRepository>(
    repository: &R,
    artefact: &QueriedArtefactRecord,
    commit_sha: &str,
    classification_filter: Option<&str>,
) -> Result<QueryResponse> {
    let linked_tests = repository.load_covering_tests(commit_sha, &artefact.artefact_id)?;
    let coverage_exists_for_commit = repository.coverage_exists_for_commit(commit_sha)?;
    let fallback_fan_out = repository.load_linked_fan_out_by_test(commit_sha)?;

    let mut covering_tests = Vec::new();
    for linked_test in linked_tests {
        let pair_stats = repository.load_coverage_pair_stats(
            commit_sha,
            &linked_test.test_id,
            &artefact.artefact_id,
        )?;
        let has_covered = pair_stats.covered_rows > 0;
        let confidence = round2(compute_confidence(
            has_covered,
            coverage_exists_for_commit,
            pair_stats.total_rows > 0,
        ));

        let classification = linked_test
            .classification
            .clone()
            .unwrap_or_else(|| "unit".to_string());
        let classification_source = linked_test
            .classification_source
            .clone()
            .unwrap_or_else(|| "static_analysis".to_string());
        let fan_out = linked_test
            .fan_out
            .or_else(|| fallback_fan_out.get(&linked_test.test_id).copied())
            .unwrap_or(1);
        let strength = round2(compute_strength(fan_out, &classification));
        let last_run = repository
            .load_latest_test_run(commit_sha, &linked_test.test_id)?
            .map(|run| LastRunOutput {
                status: run.status,
                duration_ms: run.duration_ms,
                commit_sha: run.commit_sha,
            });

        let test_name = linked_test
            .test_symbol_fqn
            .as_deref()
            .map(simple_symbol_name)
            .or_else(|| linked_test.test_signature.clone())
            .unwrap_or_else(|| linked_test.test_id.clone());

        covering_tests.push(CoveringTestOutput {
            test_id: linked_test.test_id,
            test_name,
            suite_name: linked_test.suite_name,
            file_path: linked_test.test_path,
            classification,
            classification_source,
            confidence,
            strength,
            last_run,
        });
    }

    if let Some(filter) = classification_filter {
        covering_tests.retain(|test| test.classification.eq_ignore_ascii_case(filter));
    }

    let coverage_details = repository.load_coverage_summary(commit_sha, &artefact.artefact_id)?;
    let coverage = coverage_details.as_ref().map(build_coverage_output);

    let unit_count = covering_tests
        .iter()
        .filter(|test| test.classification == "unit")
        .count();
    let integration_count = covering_tests
        .iter()
        .filter(|test| test.classification == "integration")
        .count();
    let e2e_count = covering_tests
        .iter()
        .filter(|test| test.classification == "e2e")
        .count();
    let untested_branch_count = coverage_details
        .as_ref()
        .map(|details| details.branch_total.saturating_sub(details.branch_covered))
        .unwrap_or(0);

    let verification_level = if covering_tests.is_empty() {
        "untested".to_string()
    } else if let Some(details) = coverage_details.as_ref() {
        if ratio_pct(details.branch_covered, details.branch_total) < 50.0 {
            "partially_tested".to_string()
        } else {
            "well_tested".to_string()
        }
    } else {
        "partially_tested".to_string()
    };

    let total_covering_tests = covering_tests.len();

    Ok(QueryResponse {
        artefact: ArtefactOutput {
            artefact_id: artefact.artefact_id.clone(),
            name: artefact
                .symbol_fqn
                .as_deref()
                .map(simple_symbol_name)
                .unwrap_or_else(|| simple_symbol_name(&artefact.path)),
            kind: artefact.canonical_kind.clone(),
            file_path: artefact.path.clone(),
            start_line: artefact.start_line,
            end_line: artefact.end_line,
        },
        covering_tests,
        coverage,
        summary: SummaryOutput {
            verification_level,
            total_covering_tests,
            unit_count,
            integration_count,
            e2e_count,
            untested_branch_count,
        },
    })
}

fn build_coverage_output(details: &CoverageSummaryRecord) -> CoverageOutput {
    CoverageOutput {
        line_coverage_pct: round1(ratio_pct(details.line_covered, details.line_total)),
        branch_coverage_pct: round1(ratio_pct(details.branch_covered, details.branch_total)),
        branches: details
            .branches
            .iter()
            .map(|branch| BranchOutput {
                line: branch.line,
                description: format!("branch {}", branch.branch_id),
                covered: branch.covered,
                covering_test_ids: branch.covering_test_ids.clone(),
            })
            .collect(),
    }
}

fn simple_symbol_name(symbol: &str) -> String {
    let trimmed = symbol.trim();
    if trimmed.is_empty() {
        return "unknown".to_string();
    }

    if let Some(last) = trimmed.rsplit('.').next()
        && !last.is_empty()
    {
        return last.to_string();
    }

    Path::new(trimmed)
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| trimmed.to_string())
}

fn compute_confidence(
    has_covered_rows: bool,
    coverage_exists_for_commit: bool,
    pair_has_rows: bool,
) -> f64 {
    if has_covered_rows {
        0.95
    } else if coverage_exists_for_commit && pair_has_rows {
        0.4
    } else if coverage_exists_for_commit {
        0.45
    } else {
        0.6
    }
}

fn compute_strength(fan_out: i64, classification: &str) -> f64 {
    if fan_out <= 0 {
        return 0.0;
    }
    let weight = match classification {
        "unit" => 1.0,
        "integration" => 0.7,
        "e2e" => 0.4,
        _ => 0.7,
    };
    weight / fan_out as f64
}

fn ratio_pct(covered: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    covered as f64 / total as f64 * 100.0
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
