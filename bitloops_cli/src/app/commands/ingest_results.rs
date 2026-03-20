// Command handler for ingesting external test run results and materializing them
// onto previously discovered test scenario artefacts.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;

use crate::domain::TestRunRecord;
use crate::repository::{TestHarnessRepository, open_sqlite_repository};

#[derive(Debug, Deserialize)]
struct JestJson {
    #[serde(rename = "testResults")]
    test_results: Vec<JestSuiteResult>,
}

#[derive(Debug, Deserialize)]
struct JestSuiteResult {
    name: String,
    #[serde(rename = "assertionResults")]
    assertion_results: Vec<JestAssertionResult>,
}

#[derive(Debug, Deserialize)]
struct JestAssertionResult {
    title: String,
    status: String,
    #[serde(rename = "ancestorTitles")]
    ancestor_titles: Vec<String>,
    duration: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct IngestResultsSummary {
    pub ingested: usize,
    pub unmatched: usize,
}

pub fn handle(db_path: &Path, jest_json_path: &Path, commit_sha: &str) -> Result<()> {
    let mut repository = open_sqlite_repository(db_path)?;
    let summary = execute(&mut repository, jest_json_path, commit_sha)?;
    print_summary(commit_sha, &summary);
    Ok(())
}

pub fn execute(
    repository: &mut impl TestHarnessRepository,
    jest_json_path: &Path,
    commit_sha: &str,
) -> Result<IngestResultsSummary> {
    let repo_id = repository.load_repo_id_for_commit(commit_sha)?;
    let scenarios = repository.load_test_scenarios(commit_sha)?;
    let scenario_map = build_scenario_map(scenarios);

    let raw = fs::read_to_string(jest_json_path)
        .with_context(|| format!("failed to read Jest JSON file {}", jest_json_path.display()))?;
    let jest: JestJson = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse Jest JSON {}", jest_json_path.display()))?;

    let ran_at = Utc::now().to_rfc3339();
    let mut runs = Vec::new();
    let mut ingested = 0;
    let mut unmatched = 0;

    for suite in &jest.test_results {
        let suite_path = normalize_test_path(&suite.name);
        for assertion in &suite.assertion_results {
            let suite_name = assertion
                .ancestor_titles
                .last()
                .cloned()
                .unwrap_or_default();
            let key = format!("{}|{}|{}", suite_path, suite_name, assertion.title);

            let Some(test_scenario_id) = scenario_map.get(&key) else {
                eprintln!(
                    "warning: unmatched Jest result (file: {}, suite: {}, test: {})",
                    suite_path, suite_name, assertion.title
                );
                unmatched += 1;
                continue;
            };

            let status = map_jest_status(&assertion.status)?;
            let run_id = format!("run:{commit_sha}:{test_scenario_id}");
            runs.push(TestRunRecord {
                run_id,
                repo_id: repo_id.clone(),
                commit_sha: commit_sha.to_string(),
                test_scenario_id: test_scenario_id.clone(),
                status: status.to_string(),
                duration_ms: assertion.duration,
                ran_at: ran_at.clone(),
            });
            ingested += 1;
        }
    }

    repository.replace_test_runs(commit_sha, &runs)?;

    Ok(IngestResultsSummary {
        ingested,
        unmatched,
    })
}

pub fn print_summary(commit_sha: &str, summary: &IngestResultsSummary) {
    println!(
        "ingest-results complete for commit {} (runs ingested: {}, unmatched: {})",
        commit_sha, summary.ingested, summary.unmatched
    );
}

fn build_scenario_map(
    scenarios: Vec<crate::domain::ResolvedTestScenarioRecord>,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for scenario in scenarios {
        let key = format!(
            "{}|{}|{}",
            scenario.path, scenario.suite_name, scenario.test_name
        );
        map.insert(key, scenario.scenario_id);
    }
    map
}

fn normalize_test_path(raw: &str) -> String {
    let normalized = raw.replace('\\', "/");
    if let Some(index) = normalized.find("/tests/") {
        normalized[index + 1..].to_string()
    } else if normalized.starts_with("tests/") {
        normalized
    } else {
        Path::new(&normalized)
            .file_name()
            .map(|item| format!("tests/{}", item.to_string_lossy()))
            .unwrap_or(normalized)
    }
}

fn map_jest_status(status: &str) -> Result<&'static str> {
    match status {
        "passed" => Ok("pass"),
        "failed" => Ok("fail"),
        "pending" | "todo" | "skipped" => Ok("skip"),
        other => anyhow::bail!("unsupported Jest status '{}'", other),
    }
}

#[cfg(test)]
mod tests {
    use super::{map_jest_status, normalize_test_path};

    #[test]
    fn maps_jest_status_values() {
        assert_eq!(map_jest_status("passed").expect("status"), "pass");
        assert_eq!(map_jest_status("failed").expect("status"), "fail");
        assert_eq!(map_jest_status("pending").expect("status"), "skip");
    }

    #[test]
    fn normalizes_absolute_test_path_to_repo_relative_tests_path() {
        let normalized =
            normalize_test_path("/Users/dev/repo/testlens-fixture/tests/UserService.test.ts");
        assert_eq!(normalized, "tests/UserService.test.ts");
    }
}
