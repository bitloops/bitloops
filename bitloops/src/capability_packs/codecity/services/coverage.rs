use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use crate::capability_packs::codecity::types::{CodeCityDiagnostic, MetricSource};
use crate::capability_packs::test_harness::storage::TestHarnessQueryRepository;

#[derive(Debug, Clone, PartialEq)]
pub struct CoverageMetric {
    pub coverage: Option<f64>,
    pub covered_lines: Option<u64>,
    pub total_coverable_lines: Option<u64>,
    pub source: MetricSource,
}

impl CoverageMetric {
    pub fn unavailable() -> Self {
        Self {
            coverage: None,
            covered_lines: None,
            total_coverable_lines: None,
            source: MetricSource::Unavailable,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoverageCollection {
    pub by_symbol_id: BTreeMap<String, CoverageMetric>,
    pub coverage_available: bool,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

pub fn collect_coverage_by_symbol(
    store: Option<&dyn TestHarnessQueryRepository>,
    repo_id: &str,
    commit_sha: Option<&str>,
    symbol_ids: impl IntoIterator<Item = String>,
) -> Result<CoverageCollection> {
    let symbol_ids = symbol_ids.into_iter().collect::<BTreeSet<_>>();
    let mut diagnostics = Vec::new();
    let mut by_symbol_id = BTreeMap::new();
    for symbol_id in &symbol_ids {
        by_symbol_id.insert(symbol_id.clone(), CoverageMetric::unavailable());
    }

    let Some(store) = store else {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.health.coverage_store_unavailable".to_string(),
            severity: "info".to_string(),
            message: "Test-harness coverage store is not attached; coverage was excluded from CodeCity health scoring.".to_string(),
            path: None,
            boundary_id: None,
        });
        return Ok(CoverageCollection {
            by_symbol_id,
            coverage_available: false,
            diagnostics,
        });
    };

    let Some(commit_sha) = commit_sha else {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.health.coverage_commit_missing".to_string(),
            severity: "info".to_string(),
            message: "No resolved current commit was available; coverage was excluded from CodeCity health scoring.".to_string(),
            path: None,
            boundary_id: None,
        });
        return Ok(CoverageCollection {
            by_symbol_id,
            coverage_available: false,
            diagnostics,
        });
    };

    if !store.coverage_exists_for_commit(commit_sha)? {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.health.coverage_not_indexed".to_string(),
            severity: "info".to_string(),
            message: "No test coverage data is available for this commit; coverage was excluded from CodeCity health scoring.".to_string(),
            path: None,
            boundary_id: None,
        });
        return Ok(CoverageCollection {
            by_symbol_id,
            coverage_available: false,
            diagnostics,
        });
    }

    let mut covered_symbol_count = 0usize;
    for symbol_id in &symbol_ids {
        let rows = store.load_stage_line_coverage(repo_id, symbol_id, Some(commit_sha))?;
        if rows.is_empty() {
            continue;
        }
        let covered_lines = rows.iter().filter(|row| row.covered).count() as u64;
        let total_coverable_lines = rows.len() as u64;
        covered_symbol_count += 1;
        by_symbol_id.insert(
            symbol_id.clone(),
            CoverageMetric {
                coverage: Some(covered_lines as f64 / total_coverable_lines as f64),
                covered_lines: Some(covered_lines),
                total_coverable_lines: Some(total_coverable_lines),
                source: MetricSource::ArtefactLevel,
            },
        );
    }

    if covered_symbol_count == 0 && !symbol_ids.is_empty() {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.health.coverage_symbol_missing".to_string(),
            severity: "debug".to_string(),
            message: "Coverage exists for the commit, but no CodeCity floor symbols had line coverage rows.".to_string(),
            path: None,
            boundary_id: None,
        });
    }

    Ok(CoverageCollection {
        by_symbol_id,
        coverage_available: covered_symbol_count > 0,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use anyhow::Result;

    use super::collect_coverage_by_symbol;
    use crate::capability_packs::test_harness::storage::TestHarnessQueryRepository;
    use crate::models::{
        CoveragePairStats, CoverageSummaryRecord, CoveringTestRecord, LatestTestRunRecord,
        StageBranchCoverageRecord, StageCoverageMetadataRecord, StageCoveringTestRecord,
        StageLineCoverageRecord, TestHarnessCommitCounts,
    };

    #[derive(Default)]
    struct FakeCoverageStore {
        exists: bool,
        rows: BTreeMap<String, Vec<StageLineCoverageRecord>>,
    }

    impl TestHarnessQueryRepository for FakeCoverageStore {
        fn load_covering_tests(
            &self,
            _commit_sha: &str,
            _production_symbol_id: &str,
        ) -> Result<Vec<CoveringTestRecord>> {
            unreachable!()
        }

        fn load_linked_fan_out_by_test(
            &self,
            _commit_sha: &str,
        ) -> Result<std::collections::HashMap<String, i64>> {
            unreachable!()
        }

        fn coverage_exists_for_commit(&self, _commit_sha: &str) -> Result<bool> {
            Ok(self.exists)
        }

        fn load_coverage_pair_stats(
            &self,
            _commit_sha: &str,
            _test_symbol_id: &str,
            _production_symbol_id: &str,
        ) -> Result<CoveragePairStats> {
            unreachable!()
        }

        fn load_latest_test_run(
            &self,
            _commit_sha: &str,
            _test_symbol_id: &str,
        ) -> Result<Option<LatestTestRunRecord>> {
            unreachable!()
        }

        fn load_coverage_summary(
            &self,
            _commit_sha: &str,
            _production_symbol_id: &str,
        ) -> Result<Option<CoverageSummaryRecord>> {
            unreachable!()
        }

        fn load_test_harness_commit_counts(
            &self,
            _commit_sha: &str,
        ) -> Result<TestHarnessCommitCounts> {
            unreachable!()
        }

        fn load_stage_covering_tests(
            &self,
            _repo_id: &str,
            _production_symbol_id: &str,
            _commit_sha: Option<&str>,
            _min_confidence: Option<f64>,
            _linkage_source: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<StageCoveringTestRecord>> {
            unreachable!()
        }

        fn load_stage_line_coverage(
            &self,
            _repo_id: &str,
            production_symbol_id: &str,
            _commit_sha: Option<&str>,
        ) -> Result<Vec<StageLineCoverageRecord>> {
            Ok(self
                .rows
                .get(production_symbol_id)
                .cloned()
                .unwrap_or_default())
        }

        fn load_stage_branch_coverage(
            &self,
            _repo_id: &str,
            _production_symbol_id: &str,
            _commit_sha: Option<&str>,
        ) -> Result<Vec<StageBranchCoverageRecord>> {
            Ok(Vec::new())
        }

        fn load_stage_coverage_metadata(
            &self,
            _repo_id: &str,
            _commit_sha: Option<&str>,
        ) -> Result<Option<StageCoverageMetadataRecord>> {
            Ok(None)
        }
    }

    #[test]
    fn coverage_maps_symbol_rows_to_ratio() -> Result<()> {
        let mut store = FakeCoverageStore {
            exists: true,
            ..FakeCoverageStore::default()
        };
        store.rows.insert(
            "symbol-a".to_string(),
            vec![
                StageLineCoverageRecord {
                    line: 1,
                    covered: true,
                },
                StageLineCoverageRecord {
                    line: 2,
                    covered: true,
                },
                StageLineCoverageRecord {
                    line: 3,
                    covered: false,
                },
            ],
        );

        let result = collect_coverage_by_symbol(
            Some(&store),
            "repo-1",
            Some("commit-1"),
            ["symbol-a".to_string()],
        )?;

        let metric = &result.by_symbol_id["symbol-a"];
        assert_eq!(metric.covered_lines, Some(2));
        assert_eq!(metric.total_coverable_lines, Some(3));
        assert_eq!(metric.coverage, Some(2.0 / 3.0));
        assert!(result.coverage_available);
        Ok(())
    }

    #[test]
    fn missing_store_and_empty_rows_are_unavailable() -> Result<()> {
        let no_store =
            collect_coverage_by_symbol(None, "repo-1", Some("commit-1"), ["symbol-a".to_string()])?;
        assert!(!no_store.coverage_available);
        assert_eq!(no_store.by_symbol_id["symbol-a"].coverage, None);

        let store = FakeCoverageStore {
            exists: true,
            ..FakeCoverageStore::default()
        };
        let missing_symbol = collect_coverage_by_symbol(
            Some(&store),
            "repo-1",
            Some("commit-1"),
            ["symbol-a".to_string()],
        )?;
        assert_eq!(missing_symbol.by_symbol_id["symbol-a"].coverage, None);
        assert!(
            missing_symbol
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "codecity.health.coverage_symbol_missing")
        );
        Ok(())
    }
}
