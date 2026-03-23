use std::collections::{HashMap, HashSet};

use crate::capability_packs::test_harness::mapping::linker::{
    matched_production_artefacts, scenario_id_suffix,
};
use crate::capability_packs::test_harness::mapping::model::ProductionIndex;
use crate::capability_packs::test_harness::mapping::model::{
    DiscoveredTestFile, EnumeratedTestScenario, ScenarioDiscoverySource, StructuralMappingStats,
};
use crate::models::{ProductionArtefact, TestLinkRecord, TestScenarioRecord, TestSuiteRecord};

pub(crate) struct MaterializationContext<'a> {
    pub(crate) repo_id: &'a str,
    pub(crate) commit_sha: &'a str,
    pub(crate) production: &'a [ProductionArtefact],
    pub(crate) production_index: &'a ProductionIndex,
    pub(crate) suites: &'a mut Vec<TestSuiteRecord>,
    pub(crate) scenarios: &'a mut Vec<TestScenarioRecord>,
    pub(crate) links: &'a mut Vec<TestLinkRecord>,
    pub(crate) link_keys: &'a mut HashSet<String>,
    pub(crate) stats: &'a mut StructuralMappingStats,
}

struct RecordContext<'a> {
    repo_id: &'a str,
    commit_sha: &'a str,
    path: &'a str,
    language: &'a str,
}

struct TestSuiteSpec<'a> {
    suite_id: &'a str,
    name: &'a str,
    start_line: i64,
    end_line: i64,
    signature: Option<&'a str>,
    discovery_source: ScenarioDiscoverySource,
}

struct TestScenarioSpec<'a> {
    scenario_id: &'a str,
    suite_id: &'a str,
    name: &'a str,
    symbol_fqn: Option<&'a str>,
    start_line: i64,
    end_line: i64,
    signature: Option<&'a str>,
    discovery_source: ScenarioDiscoverySource,
}

pub(crate) fn materialize_source_discovery(
    context: &mut MaterializationContext<'_>,
    files: &[DiscoveredTestFile],
) {
    let repo_id = context.repo_id;
    let commit_sha = context.commit_sha;

    for file in files {
        context.stats.files += 1;
        if file.suites.is_empty() {
            continue;
        }

        let record_context = RecordContext {
            repo_id,
            commit_sha,
            path: &file.relative_path,
            language: &file.language,
        };

        for suite in &file.suites {
            context.stats.suites += 1;
            let suite_id = format!(
                "test_suite:{commit_sha}:{}:{}",
                file.relative_path, suite.start_line
            );
            context.suites.push(build_test_suite_record(
                &record_context,
                &TestSuiteSpec {
                    suite_id: &suite_id,
                    name: &suite.name,
                    start_line: suite.start_line,
                    end_line: suite.end_line,
                    signature: Some(&suite.name),
                    discovery_source: ScenarioDiscoverySource::Source,
                },
            ));

            for scenario in &suite.scenarios {
                context.stats.scenarios += 1;
                let scenario_id = format!(
                    "test_case:{commit_sha}:{}:{}:{}",
                    file.relative_path,
                    scenario.start_line,
                    scenario_id_suffix(&scenario.name),
                );
                let scenario_fqn = format!("{}.{}", suite.name, scenario.name);
                context.scenarios.push(build_test_scenario_record(
                    &record_context,
                    &TestScenarioSpec {
                        scenario_id: &scenario_id,
                        suite_id: &suite_id,
                        name: &scenario.name,
                        symbol_fqn: Some(&scenario_fqn),
                        start_line: scenario.start_line,
                        end_line: scenario.end_line,
                        signature: Some(&scenario.name),
                        discovery_source: scenario.discovery_source,
                    },
                ));

                for production_artefact in matched_production_artefacts(
                    context.production,
                    context.production_index,
                    file,
                    scenario,
                ) {
                    let link_key = format!("{}::{}", scenario_id, production_artefact.artefact_id);
                    if !context.link_keys.insert(link_key) {
                        continue;
                    }

                    let link_id = format!(
                        "link:{commit_sha}:{}:{}",
                        scenario_id, production_artefact.artefact_id
                    );
                    context.links.push(build_test_link_record(&TestLinkSpec {
                        test_link_id: &link_id,
                        repo_id,
                        commit_sha,
                        test_scenario_id: &scenario_id,
                        production_artefact_id: &production_artefact.artefact_id,
                        production_symbol_id: Some(&production_artefact.symbol_id),
                        confidence: 0.6,
                        linkage_status: "resolved",
                    }));
                    context.stats.links += 1;
                }
            }
        }
    }
}

pub(crate) fn materialize_enumerated_scenarios(
    context: &mut MaterializationContext<'_>,
    scenarios_out: &[EnumeratedTestScenario],
) {
    let repo_id = context.repo_id;
    let commit_sha = context.commit_sha;
    let mut synthetic_suites = HashMap::new();

    for enumerated in scenarios_out {
        let suite_key = format!("{}::{}", enumerated.relative_path, enumerated.suite_name);
        let suite_id = synthetic_suites
            .entry(suite_key.clone())
            .or_insert_with(|| {
                context.stats.suites += 1;
                let suite_id = format!(
                    "test_suite:{commit_sha}:{}:{}",
                    enumerated.relative_path,
                    scenario_id_suffix(&enumerated.suite_name),
                );
                let record_context = RecordContext {
                    repo_id,
                    commit_sha,
                    path: &enumerated.relative_path,
                    language: &enumerated.language,
                };
                context.suites.push(build_test_suite_record(
                    &record_context,
                    &TestSuiteSpec {
                        suite_id: &suite_id,
                        name: &enumerated.suite_name,
                        start_line: 1,
                        end_line: 1,
                        signature: Some(&enumerated.suite_name),
                        discovery_source: ScenarioDiscoverySource::Enumeration,
                    },
                ));
                suite_id
            });

        let scenario_id = format!(
            "test_case:{commit_sha}:{}:{}:{}",
            enumerated.relative_path,
            enumerated.start_line,
            scenario_id_suffix(&enumerated.scenario_name),
        );
        let scenario_fqn = format!("{}.{}", enumerated.suite_name, enumerated.scenario_name);
        let record_context = RecordContext {
            repo_id,
            commit_sha,
            path: &enumerated.relative_path,
            language: &enumerated.language,
        };
        context.scenarios.push(build_test_scenario_record(
            &record_context,
            &TestScenarioSpec {
                scenario_id: &scenario_id,
                suite_id,
                name: &enumerated.scenario_name,
                symbol_fqn: Some(&scenario_fqn),
                start_line: enumerated.start_line,
                end_line: enumerated.start_line.max(1),
                signature: Some(&enumerated.scenario_name),
                discovery_source: enumerated.discovery_source,
            },
        ));
        context.stats.scenarios += 1;
        context.stats.enumerated_scenarios += 1;

        let synthetic_file = DiscoveredTestFile {
            relative_path: enumerated.relative_path.clone(),
            language: enumerated.language.clone(),
            reference_candidates: if enumerated.relative_path.starts_with("__synthetic_tests__/") {
                Vec::new()
            } else {
                vec![
                    crate::capability_packs::test_harness::mapping::model::ReferenceCandidate::SourcePath(
                        enumerated.relative_path.clone(),
                    ),
                ]
            },
            suites: Vec::new(),
        };
        let synthetic_scenario =
            crate::capability_packs::test_harness::mapping::model::DiscoveredTestScenario {
                name: enumerated.scenario_name.clone(),
                start_line: enumerated.start_line,
                end_line: enumerated.start_line.max(1),
                reference_candidates: enumerated.reference_candidates.clone(),
                discovery_source: enumerated.discovery_source,
            };

        for production_artefact in matched_production_artefacts(
            context.production,
            context.production_index,
            &synthetic_file,
            &synthetic_scenario,
        ) {
            let link_key = format!("{}::{}", scenario_id, production_artefact.artefact_id);
            if !context.link_keys.insert(link_key) {
                continue;
            }

            let link_id = format!(
                "link:{commit_sha}:{}:{}",
                scenario_id, production_artefact.artefact_id
            );
            context.links.push(build_test_link_record(&TestLinkSpec {
                test_link_id: &link_id,
                repo_id,
                commit_sha,
                test_scenario_id: &scenario_id,
                production_artefact_id: &production_artefact.artefact_id,
                production_symbol_id: Some(&production_artefact.symbol_id),
                confidence: 0.6,
                linkage_status: "resolved",
            }));
            context.stats.links += 1;
        }
    }
}

fn build_test_suite_record(
    context: &RecordContext<'_>,
    spec: &TestSuiteSpec<'_>,
) -> TestSuiteRecord {
    TestSuiteRecord {
        suite_id: spec.suite_id.to_string(),
        repo_id: context.repo_id.to_string(),
        commit_sha: context.commit_sha.to_string(),
        language: context.language.to_string(),
        path: context.path.to_string(),
        name: spec.name.to_string(),
        symbol_fqn: Some(spec.name.to_string()),
        start_line: spec.start_line,
        end_line: spec.end_line,
        start_byte: None,
        end_byte: None,
        signature: spec.signature.map(str::to_string),
        discovery_source: spec.discovery_source.as_str().to_string(),
    }
}

fn build_test_scenario_record(
    context: &RecordContext<'_>,
    spec: &TestScenarioSpec<'_>,
) -> TestScenarioRecord {
    TestScenarioRecord {
        scenario_id: spec.scenario_id.to_string(),
        suite_id: spec.suite_id.to_string(),
        repo_id: context.repo_id.to_string(),
        commit_sha: context.commit_sha.to_string(),
        language: context.language.to_string(),
        path: context.path.to_string(),
        name: spec.name.to_string(),
        symbol_fqn: spec.symbol_fqn.map(str::to_string),
        start_line: spec.start_line,
        end_line: spec.end_line,
        start_byte: None,
        end_byte: None,
        signature: spec.signature.map(str::to_string),
        discovery_source: spec.discovery_source.as_str().to_string(),
    }
}

struct TestLinkSpec<'a> {
    test_link_id: &'a str,
    repo_id: &'a str,
    commit_sha: &'a str,
    test_scenario_id: &'a str,
    production_artefact_id: &'a str,
    production_symbol_id: Option<&'a str>,
    confidence: f64,
    linkage_status: &'a str,
}

fn build_test_link_record(spec: &TestLinkSpec<'_>) -> TestLinkRecord {
    TestLinkRecord {
        test_link_id: spec.test_link_id.to_string(),
        repo_id: spec.repo_id.to_string(),
        commit_sha: spec.commit_sha.to_string(),
        test_scenario_id: spec.test_scenario_id.to_string(),
        production_artefact_id: spec.production_artefact_id.to_string(),
        production_symbol_id: spec.production_symbol_id.map(str::to_string),
        link_source: "static_analysis".to_string(),
        evidence_json: "{}".to_string(),
        confidence: spec.confidence,
        linkage_status: spec.linkage_status.to_string(),
    }
}
