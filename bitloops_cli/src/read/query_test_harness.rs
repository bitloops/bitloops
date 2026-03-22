// Read-side workflow for composing the test harness view returned by `query`
// and `list` commands.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::capability_packs::test_harness::storage::TestHarnessQueryRepository;
use crate::models::{CoverageSummaryRecord, QueriedArtefactRecord};
use crate::read::query_view::QueryViewArg;

const DEFAULT_MIN_STRENGTH: f64 = 0.3;
const WELL_TESTED_MIN_BRANCH_COVERAGE_PCT: f64 = 50.0;

const CONFIDENCE_COVERAGE_VERIFIED: f64 = 0.95;
const CONFIDENCE_STATIC_ONLY_WITH_COVERAGE_FOR_PAIR: f64 = 0.4;
const CONFIDENCE_STATIC_ONLY_WITH_COVERAGE_FOR_COMMIT: f64 = 0.45;
const CONFIDENCE_STATIC_ONLY_WITHOUT_COVERAGE: f64 = 0.6;

const UNIT_STRENGTH_WEIGHT: f64 = 1.0;
const INTEGRATION_STRENGTH_WEIGHT: f64 = 0.7;
const E2E_STRENGTH_WEIGHT: f64 = 0.4;

#[derive(Debug, Clone, Serialize)]
struct QueryResponse {
    artefact: ArtefactOutput,
    covering_tests: Vec<CoveringTestOutput>,
    coverage: Option<CoverageOutput>,
    summary: SummaryOutput,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum QueryPayload {
    Full(QueryResponse),
    Summary(SummaryResponse),
    Tests(TestsResponse),
    Coverage(CoverageResponse),
}

#[derive(Debug, Clone, Serialize)]
struct SummaryResponse {
    artefact: ArtefactOutput,
    summary: SummaryOutput,
}

#[derive(Debug, Clone, Serialize)]
struct TestsResponse {
    artefact: ArtefactOutput,
    covering_tests: Vec<CoveringTestOutput>,
    summary: SummaryOutput,
}

#[derive(Debug, Clone, Serialize)]
struct CoverageResponse {
    artefact: ArtefactOutput,
    coverage: Option<CoverageOutput>,
}

#[derive(Debug, Clone, Serialize)]
struct ArtefactOutput {
    artefact_id: String,
    name: String,
    kind: String,
    file_path: String,
    start_line: i64,
    end_line: i64,
}

#[derive(Debug, Clone, Serialize)]
struct CoveringTestOutput {
    test_id: String,
    test_name: String,
    suite_name: Option<String>,
    file_path: String,
    classification: String,
    classification_source: String,
    confidence: f64,
    strength: f64,
    evidence: String,
    last_run: Option<LastRunOutput>,
}

#[derive(Debug, Clone, Serialize)]
struct LastRunOutput {
    status: String,
    duration_ms: Option<i64>,
    commit_sha: String,
}

#[derive(Debug, Clone, Serialize)]
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
}

#[derive(Debug, Clone, Serialize)]
struct SummaryOutput {
    verification_level: String,
    total_covering_tests: usize,
    unit_count: usize,
    integration_count: usize,
    e2e_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    line_coverage_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch_coverage_pct: Option<f64>,
    untested_branch_count: usize,
    coverage_mode: String,
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

pub fn render_query_artefact_harness<R: TestHarnessQueryRepository>(
    repository: &R,
    artefact_query: &str,
    commit_sha: &str,
    classification_filter: Option<&str>,
    view: QueryViewArg,
    min_strength: Option<f64>,
) -> Result<String> {
    validate_min_strength(min_strength)?;
    let artefact = repository.find_artefact(commit_sha, artefact_query)?;
    let response = build_query_payload(
        repository,
        &artefact,
        commit_sha,
        classification_filter,
        view,
        min_strength,
    )?;
    serde_json::to_string_pretty(&response).context("failed serializing query JSON")
}

pub fn render_list_artefacts<R: TestHarnessQueryRepository>(
    repository: &R,
    commit_sha: &str,
    kind: Option<&str>,
) -> Result<String> {
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

    serde_json::to_string_pretty(&output).context("failed serializing list JSON")
}

fn build_query_payload<R: TestHarnessQueryRepository>(
    repository: &R,
    artefact: &QueriedArtefactRecord,
    commit_sha: &str,
    classification_filter: Option<&str>,
    view: QueryViewArg,
    min_strength: Option<f64>,
) -> Result<QueryPayload> {
    let linked_tests = repository.load_covering_tests(commit_sha, &artefact.artefact_id)?;
    let coverage_exists_for_commit = repository.coverage_exists_for_commit(commit_sha)?;
    let fallback_fan_out = repository.load_linked_fan_out_by_test(commit_sha)?;

    let mut all_covering_tests = Vec::new();
    for linked_test in linked_tests {
        let pair_stats = repository.load_coverage_pair_stats(
            commit_sha,
            &linked_test.test_id,
            &artefact.artefact_id,
        )?;
        let has_covered = pair_stats.covered_rows > 0;
        let evidence = if has_covered {
            "isolated_line_hit"
        } else {
            "static_only"
        };
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
            .test_signature
            .clone()
            .or_else(|| {
                linked_test
                    .test_symbol_fqn
                    .as_deref()
                    .map(simple_symbol_name)
            })
            .unwrap_or_else(|| linked_test.test_id.clone());

        all_covering_tests.push(CoveringTestOutput {
            test_id: linked_test.test_id,
            test_name,
            suite_name: linked_test.suite_name,
            file_path: linked_test.test_path,
            classification,
            classification_source,
            confidence,
            strength,
            evidence: evidence.to_string(),
            last_run,
        });
    }

    let has_isolated = all_covering_tests
        .iter()
        .any(|t| t.evidence == "isolated_line_hit");

    let filtered_by_classification =
        apply_classification_filter(all_covering_tests, classification_filter);

    let coverage_details = repository.load_coverage_summary(commit_sha, &artefact.artefact_id)?;
    let coverage = coverage_details.as_ref().map(build_coverage_output);

    let unit_count = filtered_by_classification
        .iter()
        .filter(|test| test.classification == "unit")
        .count();
    let integration_count = filtered_by_classification
        .iter()
        .filter(|test| test.classification == "integration")
        .count();
    let e2e_count = filtered_by_classification
        .iter()
        .filter(|test| test.classification == "e2e")
        .count();
    let untested_branch_count = coverage_details
        .as_ref()
        .map(|details| details.branch_total.saturating_sub(details.branch_covered))
        .unwrap_or(0);

    let verification_level =
        derive_verification_level(filtered_by_classification.len(), coverage_details.as_ref())
            .to_string();

    let artefact_output = ArtefactOutput {
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
    };
    let coverage_mode = if !coverage_exists_for_commit {
        "none"
    } else if has_isolated
        && coverage_details
            .as_ref()
            .is_some_and(|d| d.branch_total > 0)
    {
        "per_test_branch"
    } else if has_isolated {
        "per_test_line"
    } else {
        "artefact_only"
    };

    let summary = SummaryOutput {
        verification_level,
        total_covering_tests: filtered_by_classification.len(),
        unit_count,
        integration_count,
        e2e_count,
        line_coverage_pct: coverage_details
            .as_ref()
            .map(|details| round1(ratio_pct(details.line_covered, details.line_total))),
        branch_coverage_pct: coverage_details
            .as_ref()
            .map(|details| round1(ratio_pct(details.branch_covered, details.branch_total))),
        untested_branch_count,
        coverage_mode: coverage_mode.to_string(),
    };
    let visible_covering_tests = apply_min_strength_filter(
        filtered_by_classification,
        effective_min_strength(view, min_strength),
    );

    match view {
        QueryViewArg::Full => Ok(QueryPayload::Full(QueryResponse {
            artefact: artefact_output,
            covering_tests: visible_covering_tests,
            coverage,
            summary,
        })),
        QueryViewArg::Summary => Ok(QueryPayload::Summary(SummaryResponse {
            artefact: artefact_output,
            summary,
        })),
        QueryViewArg::Tests => Ok(QueryPayload::Tests(TestsResponse {
            artefact: artefact_output,
            covering_tests: visible_covering_tests,
            summary,
        })),
        QueryViewArg::Coverage => Ok(QueryPayload::Coverage(CoverageResponse {
            artefact: artefact_output,
            coverage,
        })),
    }
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

fn apply_classification_filter(
    covering_tests: Vec<CoveringTestOutput>,
    classification_filter: Option<&str>,
) -> Vec<CoveringTestOutput> {
    let Some(filter) = classification_filter else {
        return covering_tests;
    };

    covering_tests
        .into_iter()
        .filter(|test| test.classification.eq_ignore_ascii_case(filter))
        .collect()
}

fn effective_min_strength(view: QueryViewArg, min_strength: Option<f64>) -> Option<f64> {
    match view {
        QueryViewArg::Full | QueryViewArg::Tests => {
            Some(min_strength.unwrap_or(DEFAULT_MIN_STRENGTH))
        }
        QueryViewArg::Summary | QueryViewArg::Coverage => None,
    }
}

fn apply_min_strength_filter(
    covering_tests: Vec<CoveringTestOutput>,
    min_strength: Option<f64>,
) -> Vec<CoveringTestOutput> {
    let Some(min_strength) = min_strength else {
        return covering_tests;
    };

    covering_tests
        .into_iter()
        .filter(|test| test.strength >= min_strength)
        .collect()
}

fn validate_min_strength(min_strength: Option<f64>) -> Result<()> {
    let Some(value) = min_strength else {
        return Ok(());
    };

    if !(0.0..=1.0).contains(&value) {
        anyhow::bail!("min_strength must be between 0.0 and 1.0");
    }

    Ok(())
}

fn compute_confidence(
    has_covered_rows: bool,
    coverage_exists_for_commit: bool,
    pair_has_rows: bool,
) -> f64 {
    if has_covered_rows {
        CONFIDENCE_COVERAGE_VERIFIED
    } else if coverage_exists_for_commit && pair_has_rows {
        CONFIDENCE_STATIC_ONLY_WITH_COVERAGE_FOR_PAIR
    } else if coverage_exists_for_commit {
        CONFIDENCE_STATIC_ONLY_WITH_COVERAGE_FOR_COMMIT
    } else {
        CONFIDENCE_STATIC_ONLY_WITHOUT_COVERAGE
    }
}

fn compute_strength(fan_out: i64, classification: &str) -> f64 {
    if fan_out <= 0 {
        return 0.0;
    }
    let weight = match classification {
        "unit" => UNIT_STRENGTH_WEIGHT,
        "integration" => INTEGRATION_STRENGTH_WEIGHT,
        "e2e" => E2E_STRENGTH_WEIGHT,
        _ => INTEGRATION_STRENGTH_WEIGHT,
    };
    weight / fan_out as f64
}

fn derive_verification_level(
    covering_test_count: usize,
    coverage_details: Option<&CoverageSummaryRecord>,
) -> &'static str {
    if covering_test_count == 0 {
        return "untested";
    }

    let Some(details) = coverage_details else {
        return "partially_tested";
    };

    if ratio_pct(details.branch_covered, details.branch_total) < WELL_TESTED_MIN_BRANCH_COVERAGE_PCT
    {
        "partially_tested"
    } else {
        "well_tested"
    }
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

#[cfg(test)]
mod tests {
    use super::{
        CONFIDENCE_COVERAGE_VERIFIED, CONFIDENCE_STATIC_ONLY_WITHOUT_COVERAGE,
        DEFAULT_MIN_STRENGTH, E2E_STRENGTH_WEIGHT, INTEGRATION_STRENGTH_WEIGHT,
        WELL_TESTED_MIN_BRANCH_COVERAGE_PCT, apply_min_strength_filter, compute_confidence,
        compute_strength, derive_verification_level, effective_min_strength, validate_min_strength,
    };
    use crate::models::CoverageSummaryRecord;
    use crate::read::query_view::QueryViewArg;

    #[test]
    fn applies_default_strength_only_to_full_and_tests_views() {
        assert_eq!(
            effective_min_strength(QueryViewArg::Full, None),
            Some(DEFAULT_MIN_STRENGTH)
        );
        assert_eq!(
            effective_min_strength(QueryViewArg::Tests, None),
            Some(DEFAULT_MIN_STRENGTH)
        );
        assert_eq!(effective_min_strength(QueryViewArg::Summary, None), None);
        assert_eq!(effective_min_strength(QueryViewArg::Coverage, None), None);
    }

    #[test]
    fn validates_min_strength_range() {
        assert!(validate_min_strength(Some(0.0)).is_ok());
        assert!(validate_min_strength(Some(1.0)).is_ok());
        assert!(validate_min_strength(Some(-0.1)).is_err());
        assert!(validate_min_strength(Some(1.1)).is_err());
    }

    #[test]
    fn verification_level_is_untested_without_covering_tests() {
        let details = CoverageSummaryRecord {
            line_total: 10,
            line_covered: 0,
            branch_total: 4,
            branch_covered: 0,
            branches: vec![],
        };
        assert_eq!(derive_verification_level(0, Some(&details)), "untested");
    }

    #[test]
    fn verification_level_is_partial_without_coverage_or_below_threshold() {
        assert_eq!(derive_verification_level(2, None), "partially_tested");

        let details = CoverageSummaryRecord {
            line_total: 10,
            line_covered: 7,
            branch_total: 4,
            branch_covered: 1,
            branches: vec![],
        };
        assert!(
            (super::ratio_pct(details.branch_covered, details.branch_total)
                < WELL_TESTED_MIN_BRANCH_COVERAGE_PCT)
        );
        assert_eq!(
            derive_verification_level(2, Some(&details)),
            "partially_tested"
        );
    }

    #[test]
    fn verification_level_is_well_tested_at_threshold() {
        let details = CoverageSummaryRecord {
            line_total: 10,
            line_covered: 9,
            branch_total: 2,
            branch_covered: 1,
            branches: vec![],
        };
        assert_eq!(
            super::ratio_pct(details.branch_covered, details.branch_total),
            WELL_TESTED_MIN_BRANCH_COVERAGE_PCT
        );
        assert_eq!(derive_verification_level(1, Some(&details)), "well_tested");
    }

    #[test]
    fn confidence_distinguishes_coverage_verified_from_static_only() {
        assert_eq!(
            compute_confidence(true, true, true),
            CONFIDENCE_COVERAGE_VERIFIED
        );
        assert_eq!(
            compute_confidence(false, false, false),
            CONFIDENCE_STATIC_ONLY_WITHOUT_COVERAGE
        );
    }

    #[test]
    fn strength_uses_classification_weights() {
        assert_eq!(compute_strength(1, "unit"), 1.0);
        assert_eq!(
            compute_strength(1, "integration"),
            INTEGRATION_STRENGTH_WEIGHT
        );
        assert_eq!(compute_strength(1, "e2e"), E2E_STRENGTH_WEIGHT);
        assert_eq!(compute_strength(1, "unknown"), INTEGRATION_STRENGTH_WEIGHT);
        assert_eq!(compute_strength(0, "unit"), 0.0);
    }

    #[test]
    fn min_strength_filter_keeps_only_strong_enough_tests() {
        let tests = vec![
            super::CoveringTestOutput {
                test_id: "t1".to_string(),
                test_name: "one".to_string(),
                suite_name: None,
                file_path: "tests/one".to_string(),
                classification: "unit".to_string(),
                classification_source: "static_analysis".to_string(),
                confidence: 1.0,
                strength: 0.5,
                evidence: "static_only".to_string(),
                last_run: None,
            },
            super::CoveringTestOutput {
                test_id: "t2".to_string(),
                test_name: "two".to_string(),
                suite_name: None,
                file_path: "tests/two".to_string(),
                classification: "integration".to_string(),
                classification_source: "static_analysis".to_string(),
                confidence: 1.0,
                strength: 0.2,
                evidence: "static_only".to_string(),
                last_run: None,
            },
        ];

        let filtered = apply_min_strength_filter(tests, Some(0.3));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].test_id, "t1");
    }
}
