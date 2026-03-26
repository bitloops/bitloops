use std::collections::{HashMap, HashSet};

use crate::capability_packs::test_harness::identity::{
    test_edge_id, test_revision_artefact_id, test_structural_symbol_id,
};
use crate::capability_packs::test_harness::mapping::linker::matched_production_artefacts;
use crate::capability_packs::test_harness::mapping::model::ProductionIndex;
use crate::capability_packs::test_harness::mapping::model::{
    DiscoveredTestFile, EnumeratedTestScenario, ScenarioDiscoverySource, StructuralMappingStats,
};
use crate::models::{ProductionArtefact, TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord};

pub(crate) struct MaterializationContext<'a> {
    pub(crate) repo_id: &'a str,
    pub(crate) commit_sha: &'a str,
    pub(crate) production: &'a [ProductionArtefact],
    pub(crate) production_index: &'a ProductionIndex,
    pub(crate) test_artefacts: &'a mut Vec<TestArtefactCurrentRecord>,
    pub(crate) test_edges: &'a mut Vec<TestArtefactEdgeCurrentRecord>,
    pub(crate) link_keys: &'a mut HashSet<String>,
    pub(crate) stats: &'a mut StructuralMappingStats,
}

struct RecordContext<'a> {
    repo_id: &'a str,
    commit_sha: &'a str,
    path: &'a str,
    language: &'a str,
}

impl RecordContext<'_> {
    fn blob_sha(&self) -> String {
        crate::host::devql::deterministic_uuid(&format!("{}|{}", self.commit_sha, self.path))
    }
}

struct TestArtefactSpec<'a> {
    canonical_kind: &'a str,
    language_kind: Option<&'a str>,
    symbol_fqn: Option<&'a str>,
    name: &'a str,
    parent_artefact_id: Option<&'a str>,
    parent_symbol_id: Option<&'a str>,
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
            let suite_record = build_test_artefact_current_record(
                &record_context,
                &TestArtefactSpec {
                    canonical_kind: "test_suite",
                    language_kind: None,
                    symbol_fqn: Some(&suite.name),
                    name: &suite.name,
                    parent_artefact_id: None,
                    parent_symbol_id: None,
                    start_line: suite.start_line,
                    end_line: suite.end_line,
                    signature: Some(&suite.name),
                    discovery_source: ScenarioDiscoverySource::Source,
                },
            );
            let suite_symbol_id = suite_record.symbol_id.clone();
            let suite_artefact_id = suite_record.artefact_id.clone();
            context.test_artefacts.push(suite_record);
            context.stats.test_artefacts += 1;

            for scenario in &suite.scenarios {
                let scenario_fqn = format!("{}.{}", suite.name, scenario.name);
                let scenario_record = build_test_artefact_current_record(
                    &record_context,
                    &TestArtefactSpec {
                        canonical_kind: "test_scenario",
                        language_kind: None,
                        symbol_fqn: Some(&scenario_fqn),
                        name: &scenario.name,
                        parent_artefact_id: Some(&suite_artefact_id),
                        parent_symbol_id: Some(&suite_symbol_id),
                        start_line: scenario.start_line,
                        end_line: scenario.end_line,
                        signature: Some(&scenario.name),
                        discovery_source: scenario.discovery_source,
                    },
                );
                context.stats.test_artefacts += 1;

                for production_artefact in matched_production_artefacts(
                    context.production,
                    context.production_index,
                    file,
                    scenario,
                ) {
                    let link_key = format!(
                        "{}::{}::tests",
                        scenario_record.symbol_id, production_artefact.symbol_id
                    );
                    if !context.link_keys.insert(link_key) {
                        continue;
                    }

                    context
                        .test_edges
                        .push(build_test_artefact_edge_current_record(
                            &record_context,
                            &scenario_record,
                            production_artefact,
                        ));
                    context.stats.test_edges += 1;
                }

                context.test_artefacts.push(scenario_record);
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
    let mut synthetic_suites: HashMap<String, (String, String)> = HashMap::new();

    for enumerated in scenarios_out {
        let record_context = RecordContext {
            repo_id,
            commit_sha,
            path: &enumerated.relative_path,
            language: &enumerated.language,
        };

        let suite_key = format!("{}::{}", enumerated.relative_path, enumerated.suite_name);
        let (suite_symbol_id, suite_artefact_id) = synthetic_suites
            .entry(suite_key)
            .or_insert_with(|| {
                let suite_record = build_test_artefact_current_record(
                    &record_context,
                    &TestArtefactSpec {
                        canonical_kind: "test_suite",
                        language_kind: None,
                        symbol_fqn: Some(&enumerated.suite_name),
                        name: &enumerated.suite_name,
                        parent_artefact_id: None,
                        parent_symbol_id: None,
                        start_line: 1,
                        end_line: 1,
                        signature: Some(&enumerated.suite_name),
                        discovery_source: ScenarioDiscoverySource::Enumeration,
                    },
                );
                let ids = (
                    suite_record.symbol_id.clone(),
                    suite_record.artefact_id.clone(),
                );
                context.test_artefacts.push(suite_record);
                context.stats.test_artefacts += 1;
                ids
            })
            .clone();

        let scenario_fqn = format!("{}.{}", enumerated.suite_name, enumerated.scenario_name);
        let scenario_record = build_test_artefact_current_record(
            &record_context,
            &TestArtefactSpec {
                canonical_kind: "test_scenario",
                language_kind: None,
                symbol_fqn: Some(&scenario_fqn),
                name: &enumerated.scenario_name,
                parent_artefact_id: Some(&suite_artefact_id),
                parent_symbol_id: Some(&suite_symbol_id),
                start_line: enumerated.start_line,
                end_line: enumerated.start_line.max(1),
                signature: Some(&enumerated.scenario_name),
                discovery_source: enumerated.discovery_source,
            },
        );
        context.stats.test_artefacts += 1;
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
            let link_key = format!(
                "{}::{}::tests",
                scenario_record.symbol_id, production_artefact.symbol_id
            );
            if !context.link_keys.insert(link_key) {
                continue;
            }

            context
                .test_edges
                .push(build_test_artefact_edge_current_record(
                    &record_context,
                    &scenario_record,
                    production_artefact,
                ));
            context.stats.test_edges += 1;
        }

        context.test_artefacts.push(scenario_record);
    }
}

fn build_test_artefact_current_record(
    context: &RecordContext<'_>,
    spec: &TestArtefactSpec<'_>,
) -> TestArtefactCurrentRecord {
    let blob_sha = context.blob_sha();
    let symbol_id = test_structural_symbol_id(
        context.path,
        spec.canonical_kind,
        spec.language_kind,
        spec.parent_symbol_id,
        spec.name,
        spec.signature,
    );
    let artefact_id = test_revision_artefact_id(context.repo_id, &blob_sha, &symbol_id);

    TestArtefactCurrentRecord {
        artefact_id,
        symbol_id,
        repo_id: context.repo_id.to_string(),
        commit_sha: context.commit_sha.to_string(),
        blob_sha,
        path: context.path.to_string(),
        language: context.language.to_string(),
        canonical_kind: spec.canonical_kind.to_string(),
        language_kind: spec.language_kind.map(str::to_string),
        symbol_fqn: spec.symbol_fqn.map(str::to_string),
        name: spec.name.to_string(),
        parent_artefact_id: spec.parent_artefact_id.map(str::to_string),
        parent_symbol_id: spec.parent_symbol_id.map(str::to_string),
        start_line: spec.start_line,
        end_line: spec.end_line,
        start_byte: None,
        end_byte: None,
        signature: spec.signature.map(str::to_string),
        modifiers: "[]".to_string(),
        docstring: None,
        content_hash: None,
        discovery_source: spec.discovery_source.as_str().to_string(),
        revision_kind: "commit".to_string(),
        revision_id: context.commit_sha.to_string(),
    }
}

fn build_test_artefact_edge_current_record(
    context: &RecordContext<'_>,
    from: &TestArtefactCurrentRecord,
    production_artefact: &ProductionArtefact,
) -> TestArtefactEdgeCurrentRecord {
    let metadata =
        r#"{"confidence":0.6,"link_source":"static_analysis","linkage_status":"resolved"}"#
            .to_string();

    TestArtefactEdgeCurrentRecord {
        edge_id: test_edge_id(
            context.repo_id,
            &from.symbol_id,
            "tests",
            &production_artefact.symbol_id,
        ),
        repo_id: context.repo_id.to_string(),
        commit_sha: context.commit_sha.to_string(),
        blob_sha: context.blob_sha(),
        path: context.path.to_string(),
        from_artefact_id: from.artefact_id.clone(),
        from_symbol_id: from.symbol_id.clone(),
        to_artefact_id: Some(production_artefact.artefact_id.clone()),
        to_symbol_id: Some(production_artefact.symbol_id.clone()),
        to_symbol_ref: None,
        edge_kind: "tests".to_string(),
        language: context.language.to_string(),
        start_line: Some(from.start_line),
        end_line: Some(from.end_line),
        metadata,
        revision_kind: "commit".to_string(),
        revision_id: context.commit_sha.to_string(),
    }
}
