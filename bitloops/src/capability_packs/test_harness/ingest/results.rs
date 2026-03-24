// Command handler for ingesting external test run results and materializing them
// onto previously discovered test scenario artefacts.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;

use crate::capability_packs::test_harness::storage::TestHarnessRepository;
use crate::host::capability_host::gateways::RelationalGateway;
use crate::models::TestRunRecord;

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

pub fn execute(
    repository: &mut impl TestHarnessRepository,
    relational: &dyn RelationalGateway,
    jest_json_path: &Path,
    commit_sha: &str,
) -> Result<IngestResultsSummary> {
    let repo_id = relational.load_repo_id_for_commit(commit_sha)?;
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

            let Some(test_symbol_id) = scenario_map.get(&key) else {
                eprintln!(
                    "warning: unmatched Jest result (file: {}, suite: {}, test: {})",
                    suite_path, suite_name, assertion.title
                );
                unmatched += 1;
                continue;
            };

            let status = map_jest_status(&assertion.status)?;
            let run_id = format!("run:{commit_sha}:{test_symbol_id}");
            runs.push(TestRunRecord {
                run_id,
                repo_id: repo_id.clone(),
                commit_sha: commit_sha.to_string(),
                test_symbol_id: test_symbol_id.clone(),
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
    scenarios: Vec<crate::models::ResolvedTestScenarioRecord>,
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
    use anyhow::Result;

    use super::{execute, map_jest_status, normalize_test_path};
    use crate::capability_packs::test_harness::storage::TestHarnessRepository;
    use crate::host::capability_host::gateways::RelationalGateway;
    use crate::models::{
        CoverageCaptureRecord, CoverageDiagnosticRecord, CoverageHitRecord,
        ProductionIngestionBatch, ResolvedTestScenarioRecord, TestArtefactCurrentRecord,
        TestArtefactEdgeCurrentRecord, TestDiscoveryDiagnosticRecord, TestDiscoveryRunRecord,
        TestRunRecord,
    };

    #[derive(Default)]
    struct FakeRepository {
        scenarios: Vec<ResolvedTestScenarioRecord>,
        replaced_commit_sha: Option<String>,
        replaced_runs: Vec<TestRunRecord>,
    }

    impl TestHarnessRepository for FakeRepository {
        fn load_test_scenarios(
            &self,
            _commit_sha: &str,
        ) -> Result<Vec<ResolvedTestScenarioRecord>> {
            Ok(self.scenarios.clone())
        }

        fn replace_production_artefacts(
            &mut self,
            _batch: &ProductionIngestionBatch,
        ) -> Result<()> {
            unreachable!("unused in results tests")
        }

        fn replace_test_discovery(
            &mut self,
            _commit_sha: &str,
            _test_artefacts: &[TestArtefactCurrentRecord],
            _test_edges: &[TestArtefactEdgeCurrentRecord],
            _discovery_run: &TestDiscoveryRunRecord,
            _diagnostics: &[TestDiscoveryDiagnosticRecord],
        ) -> Result<()> {
            unreachable!("unused in results tests")
        }

        fn replace_test_runs(&mut self, commit_sha: &str, runs: &[TestRunRecord]) -> Result<()> {
            self.replaced_commit_sha = Some(commit_sha.to_string());
            self.replaced_runs = runs.to_vec();
            Ok(())
        }

        fn insert_coverage_capture(&mut self, _capture: &CoverageCaptureRecord) -> Result<()> {
            unreachable!("unused in results tests")
        }

        fn insert_coverage_hits(&mut self, _hits: &[CoverageHitRecord]) -> Result<()> {
            unreachable!("unused in results tests")
        }

        fn insert_coverage_diagnostics(
            &mut self,
            _diagnostics: &[CoverageDiagnosticRecord],
        ) -> Result<()> {
            unreachable!("unused in results tests")
        }

        fn rebuild_classifications_from_coverage(&mut self, _commit_sha: &str) -> Result<usize> {
            unreachable!("unused in results tests")
        }
    }

    struct FakeRelationalGateway {
        repo_id: String,
    }

    impl RelationalGateway for FakeRelationalGateway {
        fn resolve_checkpoint_id(&self, _repo_id: &str, _checkpoint_ref: &str) -> Result<String> {
            unreachable!("unused in results tests")
        }

        fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
            unreachable!("unused in results tests")
        }

        fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
            Ok(self.repo_id.clone())
        }

        fn load_production_artefacts(
            &self,
            _commit_sha: &str,
        ) -> Result<Vec<crate::models::ProductionArtefact>> {
            unreachable!("unused in results tests")
        }

        fn load_artefacts_for_file_lines(
            &self,
            _commit_sha: &str,
            _file_path: &str,
        ) -> Result<Vec<(String, i64, i64)>> {
            unreachable!("unused in results tests")
        }
    }

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

    #[test]
    fn execute_ingests_matched_runs_using_test_symbol_ids() {
        let mut repository = FakeRepository {
            scenarios: vec![ResolvedTestScenarioRecord {
                scenario_id: "test-symbol:user-service:returns-user".to_string(),
                path: "tests/UserService.test.ts".to_string(),
                suite_name: "UserService".to_string(),
                test_name: "returns user".to_string(),
            }],
            ..Default::default()
        };
        let relational = FakeRelationalGateway {
            repo_id: "repo:test".to_string(),
        };
        let temp = tempfile::NamedTempFile::new().expect("temp jest json");
        std::fs::write(
            temp.path(),
            serde_json::json!({
                "testResults": [
                    {
                        "name": "/tmp/project/tests/UserService.test.ts",
                        "assertionResults": [
                            {
                                "title": "returns user",
                                "status": "passed",
                                "ancestorTitles": ["api", "UserService"],
                                "duration": 7
                            },
                            {
                                "title": "does not match",
                                "status": "failed",
                                "ancestorTitles": ["api", "OtherSuite"],
                                "duration": null
                            }
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("write jest json");

        let summary = execute(&mut repository, &relational, temp.path(), "commit-sha-123")
            .expect("execute results ingest");

        assert_eq!(summary.ingested, 1);
        assert_eq!(summary.unmatched, 1);
        assert_eq!(
            repository.replaced_commit_sha.as_deref(),
            Some("commit-sha-123")
        );
        assert_eq!(repository.replaced_runs.len(), 1);
        let run = &repository.replaced_runs[0];
        assert_eq!(
            run.run_id,
            "run:commit-sha-123:test-symbol:user-service:returns-user"
        );
        assert_eq!(run.repo_id, "repo:test");
        assert_eq!(run.commit_sha, "commit-sha-123");
        assert_eq!(run.test_symbol_id, "test-symbol:user-service:returns-user");
        assert_eq!(run.status, "pass");
        assert_eq!(run.duration_ms, Some(7));
        assert!(!run.ran_at.is_empty());
    }

    #[test]
    fn execute_rejects_unsupported_jest_status() {
        let mut repository = FakeRepository {
            scenarios: vec![ResolvedTestScenarioRecord {
                scenario_id: "test-symbol:user-service:returns-user".to_string(),
                path: "tests/UserService.test.ts".to_string(),
                suite_name: "UserService".to_string(),
                test_name: "returns user".to_string(),
            }],
            ..Default::default()
        };
        let relational = FakeRelationalGateway {
            repo_id: "repo:test".to_string(),
        };
        let temp = tempfile::NamedTempFile::new().expect("temp jest json");
        std::fs::write(
            temp.path(),
            serde_json::json!({
                "testResults": [
                    {
                        "name": "tests/UserService.test.ts",
                        "assertionResults": [
                            {
                                "title": "returns user",
                                "status": "flaky",
                                "ancestorTitles": ["UserService"],
                                "duration": 7
                            }
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("write jest json");

        let error = execute(&mut repository, &relational, temp.path(), "commit-sha-123")
            .expect_err("unsupported status should fail");

        assert!(
            error
                .to_string()
                .contains("unsupported Jest status 'flaky'")
        );
        assert!(repository.replaced_runs.is_empty());
    }
}
