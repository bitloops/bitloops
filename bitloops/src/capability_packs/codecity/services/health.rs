use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use chrono::{SecondsFormat, Utc};

use super::colour::colour_for_health;
use super::complexity::structural_complexity_by_symbol;
use super::config::CodeCityConfig;
use super::coverage::collect_coverage_by_symbol;
use super::history::collect_file_history;
use super::normalise::{finite, min_max_normalise};
use super::scoring::{NormalizedHealthSignals, score_health};
use super::source_graph::CodeCitySourceGraph;
use crate::capability_packs::codecity::types::{
    CodeCityBuildingHealthSummary, CodeCityHealthEvidence, CodeCityHealthMetrics,
    CodeCityHealthOverview, CodeCityHealthWeights, CodeCityWorldPayload, HealthStatus,
    MetricSource,
};
use crate::capability_packs::test_harness::storage::{
    BitloopsTestHarnessRepository, TestHarnessQueryRepository,
};
use crate::host::capability_host::gateways::GitHistoryGateway;

#[derive(Debug, Clone, PartialEq)]
struct RawFloorHealth {
    building_index: usize,
    floor_index: usize,
    churn: Option<f64>,
    complexity: Option<f64>,
    bug_count: Option<f64>,
    coverage: Option<f64>,
    author_concentration: Option<f64>,
    metrics: CodeCityHealthMetrics,
    evidence: CodeCityHealthEvidence,
}

pub fn apply_neutral_health(world: &mut CodeCityWorldPayload, config: &CodeCityConfig) {
    let weights = CodeCityHealthWeights::from(&config.health);
    world.health =
        CodeCityHealthOverview::not_requested(config.health.analysis_window_months, weights);
    world.summary.coverage_available = false;
    world.summary.git_history_available = false;
    world.summary.unhealthy_floor_count = 0;
    world.summary.insufficient_health_data_count = 0;

    for building in &mut world.buildings {
        building.health_risk = None;
        building.health_status = HealthStatus::NotRequested.as_str().to_string();
        building.health_confidence = 0.0;
        building.colour = config.colours.no_data.clone();
        building.health_summary = CodeCityBuildingHealthSummary {
            floor_count: building.floors.len(),
            insufficient_data_floor_count: building.floors.len(),
            ..CodeCityBuildingHealthSummary::default()
        };
        for floor in &mut building.floors {
            floor.health_risk = None;
            floor.health_status = HealthStatus::NotRequested.as_str().to_string();
            floor.health_confidence = 0.0;
            floor.colour = config.colours.no_data.clone();
            floor.health_metrics = CodeCityHealthMetrics::default();
            floor.health_evidence = CodeCityHealthEvidence::default();
        }
    }
}

pub fn apply_health_overlay(
    world: &mut CodeCityWorldPayload,
    source: &CodeCitySourceGraph,
    config: &CodeCityConfig,
    repo_root: &Path,
    history_gateway: &dyn GitHistoryGateway,
    test_harness_store: Option<&Mutex<BitloopsTestHarnessRepository>>,
) -> Result<()> {
    if !config.include_health {
        apply_neutral_health(world, config);
        return Ok(());
    }

    let commit_sha = world.commit_sha.as_deref();
    let symbol_ids = world
        .buildings
        .iter()
        .flat_map(|building| building.floors.iter())
        .filter_map(|floor| floor.symbol_id.clone())
        .collect::<BTreeSet<_>>();

    let coverage = if let Some(store) = test_harness_store {
        let guard = store
            .lock()
            .map_err(|_| anyhow::anyhow!("test-harness store lock poisoned"))?;
        collect_coverage_by_symbol(
            Some(&*guard as &dyn TestHarnessQueryRepository),
            &world.repo_id,
            commit_sha,
            symbol_ids.iter().cloned(),
        )?
    } else {
        collect_coverage_by_symbol(None, &world.repo_id, commit_sha, symbol_ids.iter().cloned())?
    };

    let paths = world
        .buildings
        .iter()
        .map(|building| building.path.clone())
        .collect::<BTreeSet<_>>();
    let since_unix = analysis_window_since_unix(config.health.analysis_window_months);
    let history = collect_file_history(
        history_gateway,
        repo_root,
        paths.iter().cloned(),
        since_unix,
        commit_sha,
        &config.health.bug_commit_patterns,
    )?;

    let complexity_by_symbol = structural_complexity_by_symbol(&source.artefacts, &source.edges);
    let mut raw = collect_raw_floor_health(
        world,
        &coverage.by_symbol_id,
        &history.by_path,
        &complexity_by_symbol,
    );
    score_raw_floors(world, &mut raw, config);
    aggregate_buildings(world, config);

    world.summary.coverage_available = coverage.coverage_available;
    world.summary.git_history_available = history.git_history_available;
    world.summary.unhealthy_floor_count = world
        .buildings
        .iter()
        .flat_map(|building| building.floors.iter())
        .filter(|floor| floor.health_risk.is_some_and(|risk| risk >= 0.7))
        .count();
    world.summary.insufficient_health_data_count = world
        .buildings
        .iter()
        .flat_map(|building| building.floors.iter())
        .filter(|floor| floor.health_status == HealthStatus::InsufficientData.as_str())
        .count();

    world.health = build_world_health_overview(world, config);
    world.diagnostics.extend(coverage.diagnostics);
    world.diagnostics.extend(history.diagnostics);
    if !source.edges.is_empty() {
        world.diagnostics.push(
            crate::capability_packs::codecity::types::CodeCityDiagnostic {
                code: "codecity.health.complexity_structural_proxy".to_string(),
                severity: "info".to_string(),
                message:
                    "CodeCity health used structural proxy complexity from current artefact edges."
                        .to_string(),
                path: None,
                boundary_id: None,
            },
        );
    }
    sort_health_diagnostics(&mut world.diagnostics);

    Ok(())
}

fn collect_raw_floor_health(
    world: &CodeCityWorldPayload,
    coverage_by_symbol: &BTreeMap<String, super::coverage::CoverageMetric>,
    history_by_path: &BTreeMap<String, super::history::HistoryMetric>,
    complexity_by_symbol: &BTreeMap<String, super::complexity::ComplexityMetric>,
) -> Vec<RawFloorHealth> {
    let mut raw = Vec::new();
    for (building_index, building) in world.buildings.iter().enumerate() {
        let history = history_by_path
            .get(&building.path)
            .cloned()
            .unwrap_or_else(super::history::HistoryMetric::unavailable);
        for (floor_index, floor) in building.floors.iter().enumerate() {
            let coverage = floor
                .symbol_id
                .as_deref()
                .and_then(|symbol_id| coverage_by_symbol.get(symbol_id))
                .cloned()
                .unwrap_or_else(super::coverage::CoverageMetric::unavailable);
            let complexity = floor
                .symbol_id
                .as_deref()
                .and_then(|symbol_id| complexity_by_symbol.get(symbol_id))
                .cloned();

            let history_available = history.source != MetricSource::Unavailable;
            let complexity_available = complexity
                .as_ref()
                .is_some_and(|metric| metric.source != MetricSource::Unavailable);

            let mut missing_signals = Vec::new();
            if !history_available {
                missing_signals.push("churn".to_string());
                missing_signals.push("bugs".to_string());
                missing_signals.push("author_concentration".to_string());
            } else if history.author_concentration.is_none() {
                missing_signals.push("author_concentration".to_string());
            }
            if coverage.coverage.is_none() {
                missing_signals.push("coverage".to_string());
            }
            if !complexity_available {
                missing_signals.push("complexity".to_string());
            }
            missing_signals.sort();
            missing_signals.dedup();

            raw.push(RawFloorHealth {
                building_index,
                floor_index,
                churn: history_available.then_some(history.churn as f64),
                complexity: complexity
                    .as_ref()
                    .filter(|metric| metric.source != MetricSource::Unavailable)
                    .and_then(|metric| finite(metric.value)),
                bug_count: history_available.then_some(history.bug_count as f64),
                coverage: coverage.coverage.and_then(finite),
                author_concentration: history.author_concentration.and_then(finite),
                metrics: CodeCityHealthMetrics {
                    churn: history.churn,
                    complexity: complexity
                        .as_ref()
                        .map(|metric| metric.value)
                        .unwrap_or(0.0),
                    bug_count: history.bug_count,
                    coverage: coverage.coverage,
                    author_concentration: history.author_concentration,
                },
                evidence: CodeCityHealthEvidence {
                    commits_touching: history.churn,
                    bug_fix_commits: history.bug_count,
                    distinct_authors: history.distinct_authors,
                    covered_lines: coverage.covered_lines,
                    total_coverable_lines: coverage.total_coverable_lines,
                    complexity_source: complexity
                        .map(|metric| metric.source.as_str().to_string())
                        .unwrap_or_else(|| MetricSource::Unavailable.as_str().to_string()),
                    coverage_source: coverage.source.as_str().to_string(),
                    git_history_source: history.source.as_str().to_string(),
                    missing_signals,
                },
            });
        }
    }
    raw
}

fn score_raw_floors(
    world: &mut CodeCityWorldPayload,
    raw: &mut [RawFloorHealth],
    config: &CodeCityConfig,
) {
    let churn = normalise_options(raw.iter().map(|floor| floor.churn).collect());
    let complexity = normalise_options(raw.iter().map(|floor| floor.complexity).collect());
    let bugs = normalise_options(raw.iter().map(|floor| floor.bug_count).collect());
    let coverage_risk = normalise_options(
        raw.iter()
            .map(|floor| floor.coverage.map(|coverage| 1.0 - coverage))
            .collect(),
    );
    let author_concentration =
        normalise_options(raw.iter().map(|floor| floor.author_concentration).collect());

    for (idx, raw_floor) in raw.iter_mut().enumerate() {
        let score = score_health(
            NormalizedHealthSignals {
                churn: churn[idx],
                complexity: complexity[idx],
                bugs: bugs[idx],
                coverage_risk: coverage_risk[idx],
                author_concentration: author_concentration[idx],
            },
            &config.health,
        );

        raw_floor.evidence.missing_signals = score
            .missing_signals
            .iter()
            .map(|signal| signal.as_str().to_string())
            .collect();
        let floor = &mut world.buildings[raw_floor.building_index].floors[raw_floor.floor_index];
        floor.health_risk = score.health_risk;
        floor.health_status = score.status.as_str().to_string();
        floor.health_confidence = score.confidence;
        floor.colour = colour_for_health(score.health_risk, score.status, &config.colours);
        floor.health_metrics = raw_floor.metrics.clone();
        floor.health_evidence = raw_floor.evidence.clone();
    }
}

fn normalise_options(values: Vec<Option<f64>>) -> Vec<Option<f64>> {
    let present = values
        .iter()
        .enumerate()
        .filter_map(|(idx, value)| value.and_then(finite).map(|value| (idx, value)))
        .collect::<Vec<_>>();
    let normalised_values =
        min_max_normalise(&present.iter().map(|(_, value)| *value).collect::<Vec<_>>());
    let mut out = vec![None; values.len()];
    for ((idx, _), normalised) in present.into_iter().zip(normalised_values) {
        out[idx] = Some(normalised);
    }
    out
}

fn aggregate_buildings(world: &mut CodeCityWorldPayload, config: &CodeCityConfig) {
    for building in &mut world.buildings {
        let valid_risks = building
            .floors
            .iter()
            .filter(|floor| floor.health_status != HealthStatus::InsufficientData.as_str())
            .filter_map(|floor| floor.health_risk)
            .collect::<Vec<_>>();
        let high_risk_floor_count = valid_risks.iter().filter(|risk| **risk >= 0.7).count();
        let insufficient_data_floor_count = building
            .floors
            .iter()
            .filter(|floor| floor.health_status == HealthStatus::InsufficientData.as_str())
            .count();
        let average_risk = (!valid_risks.is_empty())
            .then(|| valid_risks.iter().sum::<f64>() / valid_risks.len() as f64);
        let max_risk = valid_risks.iter().copied().reduce(f64::max);
        let confidence = if building.floors.is_empty() {
            0.0
        } else {
            building
                .floors
                .iter()
                .map(|floor| floor.health_confidence)
                .sum::<f64>()
                / building.floors.len() as f64
        };
        let status = if building.floors.is_empty()
            || insufficient_data_floor_count == building.floors.len()
        {
            HealthStatus::InsufficientData
        } else if building
            .floors
            .iter()
            .any(|floor| floor.health_status != HealthStatus::Ok.as_str())
        {
            HealthStatus::Partial
        } else {
            HealthStatus::Ok
        };
        let mut missing_signals = building
            .floors
            .iter()
            .flat_map(|floor| floor.health_evidence.missing_signals.iter().cloned())
            .collect::<Vec<_>>();
        missing_signals.sort();
        missing_signals.dedup();

        building.health_risk = max_risk;
        building.health_status = status.as_str().to_string();
        building.health_confidence = confidence;
        building.colour = colour_for_health(max_risk, status, &config.colours);
        building.health_summary = CodeCityBuildingHealthSummary {
            floor_count: building.floors.len(),
            high_risk_floor_count,
            insufficient_data_floor_count,
            average_risk,
            max_risk,
            missing_signals,
        };
    }
}

fn build_world_health_overview(
    world: &CodeCityWorldPayload,
    config: &CodeCityConfig,
) -> CodeCityHealthOverview {
    let floors = world
        .buildings
        .iter()
        .flat_map(|building| building.floors.iter())
        .collect::<Vec<_>>();
    let status = if floors.is_empty()
        || floors
            .iter()
            .all(|floor| floor.health_status == HealthStatus::InsufficientData.as_str())
    {
        HealthStatus::InsufficientData
    } else if floors
        .iter()
        .any(|floor| floor.health_status != HealthStatus::Ok.as_str())
    {
        HealthStatus::Partial
    } else {
        HealthStatus::Ok
    };
    let confidence = if floors.is_empty() {
        0.0
    } else {
        floors
            .iter()
            .map(|floor| floor.health_confidence)
            .sum::<f64>()
            / floors.len() as f64
    };
    let mut missing_signals = floors
        .iter()
        .flat_map(|floor| floor.health_evidence.missing_signals.iter().cloned())
        .collect::<Vec<_>>();
    missing_signals.sort();
    missing_signals.dedup();

    CodeCityHealthOverview {
        status: status.as_str().to_string(),
        analysis_window_months: config.health.analysis_window_months,
        generated_at: Some(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)),
        confidence,
        missing_signals,
        coverage_available: world.summary.coverage_available,
        git_history_available: world.summary.git_history_available,
        weights: CodeCityHealthWeights::from(&config.health),
    }
}

fn analysis_window_since_unix(months: u32) -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    now.saturating_sub(months as i64 * 30 * 24 * 60 * 60)
}

fn sort_health_diagnostics(
    diagnostics: &mut [crate::capability_packs::codecity::types::CodeCityDiagnostic],
) {
    diagnostics.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.message.cmp(&right.message))
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::Result;

    use super::apply_health_overlay;
    use crate::capability_packs::codecity::services::config::CodeCityConfig;
    use crate::capability_packs::codecity::services::source_graph::{
        CodeCitySourceArtefact, CodeCitySourceEdge, CodeCitySourceFile, CodeCitySourceGraph,
    };
    use crate::capability_packs::codecity::services::world::build_codecity_world;
    use crate::capability_packs::codecity::types::HealthStatus;
    use crate::host::capability_host::gateways::{
        EmptyGitHistoryGateway, FileHistoryEvent, GitHistoryGateway, GitHistoryRequest,
    };

    struct FakeHistoryGateway {
        events: Vec<FileHistoryEvent>,
    }

    impl GitHistoryGateway for FakeHistoryGateway {
        fn available(&self) -> bool {
            true
        }

        fn resolve_head(&self, _repo_root: &Path) -> Result<Option<String>> {
            Ok(Some("commit-1".to_string()))
        }

        fn load_file_history(
            &self,
            _repo_root: &Path,
            _request: GitHistoryRequest<'_>,
        ) -> Result<Vec<FileHistoryEvent>> {
            Ok(self.events.clone())
        }
    }

    fn source() -> CodeCitySourceGraph {
        CodeCitySourceGraph {
            project_path: None,
            files: vec![CodeCitySourceFile {
                path: "src/lib.rs".to_string(),
                language: "rust".to_string(),
                effective_content_id: "content-1".to_string(),
                included: true,
                exclusion_reason: None,
            }],
            artefacts: vec![CodeCitySourceArtefact {
                artefact_id: "artefact-1".to_string(),
                symbol_id: "symbol-1".to_string(),
                path: "src/lib.rs".to_string(),
                symbol_fqn: Some("crate::work".to_string()),
                canonical_kind: Some("function".to_string()),
                language_kind: None,
                parent_artefact_id: None,
                parent_symbol_id: None,
                signature: None,
                start_line: 1,
                end_line: 10,
            }],
            edges: vec![CodeCitySourceEdge {
                edge_id: "edge-1".to_string(),
                from_path: "src/lib.rs".to_string(),
                to_path: "src/other.rs".to_string(),
                from_symbol_id: "symbol-1".to_string(),
                from_artefact_id: "artefact-1".to_string(),
                to_symbol_id: Some("symbol-2".to_string()),
                to_artefact_id: Some("artefact-2".to_string()),
                to_symbol_ref: Some("crate::other".to_string()),
                edge_kind: "calls".to_string(),
                language: "rust".to_string(),
                start_line: Some(5),
                end_line: Some(5),
                metadata: "{}".to_string(),
            }],
            external_dependency_hints: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn no_optional_data_keeps_floors_insufficient() -> Result<()> {
        let config = CodeCityConfig::default();
        let source = source();
        let mut world =
            build_codecity_world(&source, "repo-1", None, config.clone(), Path::new("."))?;
        apply_health_overlay(
            &mut world,
            &source,
            &config,
            Path::new("."),
            &EmptyGitHistoryGateway,
            None,
        )?;

        let floor = &world.buildings[0].floors[0];
        assert_eq!(floor.health_status, HealthStatus::InsufficientData.as_str());
        assert_eq!(floor.colour, "#888888");
        assert!(
            world
                .health
                .missing_signals
                .contains(&"coverage".to_string())
        );
        Ok(())
    }

    #[test]
    fn git_history_only_scores_partial_with_file_fallback() -> Result<()> {
        let config = CodeCityConfig::default();
        let source = source();
        let mut world = build_codecity_world(
            &source,
            "repo-1",
            Some("commit-1".to_string()),
            config.clone(),
            Path::new("."),
        )?;
        let history = FakeHistoryGateway {
            events: vec![
                FileHistoryEvent {
                    path: "src/lib.rs".to_string(),
                    commit_sha: "a".to_string(),
                    author_name: Some("A".to_string()),
                    author_email: Some("a@example.com".to_string()),
                    committed_at_unix: 1,
                    message: "fix login".to_string(),
                    is_bug_fix: true,
                    changed_ranges: Vec::new(),
                },
                FileHistoryEvent {
                    path: "src/lib.rs".to_string(),
                    commit_sha: "b".to_string(),
                    author_name: Some("B".to_string()),
                    author_email: Some("b@example.com".to_string()),
                    committed_at_unix: 2,
                    message: "feature".to_string(),
                    is_bug_fix: false,
                    changed_ranges: Vec::new(),
                },
            ],
        };

        apply_health_overlay(&mut world, &source, &config, Path::new("."), &history, None)?;

        let floor = &world.buildings[0].floors[0];
        assert_eq!(floor.health_metrics.churn, 2);
        assert_eq!(floor.health_metrics.bug_count, 1);
        assert_eq!(floor.health_evidence.distinct_authors, 2);
        assert_eq!(
            floor.health_evidence.git_history_source,
            "file_level_fallback"
        );
        assert_eq!(floor.health_status, HealthStatus::Partial.as_str());
        assert_eq!(
            world.buildings[0].health_summary.max_risk,
            floor.health_risk
        );
        Ok(())
    }
}
