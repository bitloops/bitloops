use std::collections::{HashMap, HashSet};

use crate::app::test_mapping::linker::{
    matched_production_artefacts, scenario_id_suffix,
};
use crate::app::test_mapping::model::{
    DiscoveredTestFile, EnumeratedTestScenario, StructuralMappingStats,
};
use crate::domain::{ArtefactRecord, ProductionArtefact, TestLinkRecord};
use crate::app::test_mapping::model::ProductionIndex;

pub(crate) fn materialize_source_discovery(
    repo_id: &str,
    commit_sha: &str,
    production: &[ProductionArtefact],
    production_index: &ProductionIndex,
    files: &[DiscoveredTestFile],
    artefacts: &mut Vec<ArtefactRecord>,
    links: &mut Vec<TestLinkRecord>,
    link_keys: &mut HashSet<String>,
    stats: &mut StructuralMappingStats,
) {
    for file in files {
        stats.files += 1;
        if file.suites.is_empty() {
            continue;
        }

        let test_file_id = format!("test_file:{commit_sha}:{}", file.relative_path);
        artefacts.push(build_artefact_record(
            &test_file_id,
            repo_id,
            commit_sha,
            &file.relative_path,
            &file.language,
            "file",
            Some("source_file"),
            Some(&file.relative_path),
            None,
            1,
            file.line_count,
            None,
        ));

        for suite in &file.suites {
            stats.suites += 1;
            let suite_id = format!(
                "test_suite:{commit_sha}:{}:{}",
                file.relative_path, suite.start_line
            );
            artefacts.push(build_artefact_record(
                &suite_id,
                repo_id,
                commit_sha,
                &file.relative_path,
                &file.language,
                "test_suite",
                Some("suite_block"),
                Some(&suite.name),
                Some(&test_file_id),
                suite.start_line,
                suite.end_line,
                Some(&suite.name),
            ));

            for scenario in &suite.scenarios {
                stats.scenarios += 1;
                let scenario_id = format!(
                    "test_case:{commit_sha}:{}:{}:{}",
                    file.relative_path,
                    scenario.start_line,
                    scenario_id_suffix(&scenario.name),
                );
                let scenario_fqn = format!("{}.{}", suite.name, scenario.name);
                artefacts.push(build_artefact_record(
                    &scenario_id,
                    repo_id,
                    commit_sha,
                    &file.relative_path,
                    &file.language,
                    "test_scenario",
                    Some("test_block"),
                    Some(&scenario_fqn),
                    Some(&suite_id),
                    scenario.start_line,
                    scenario.end_line,
                    Some(&scenario.name),
                ));

                for production_artefact in
                    matched_production_artefacts(production, production_index, file, scenario)
                {
                    let link_key = format!("{}::{}", scenario_id, production_artefact.artefact_id);
                    if !link_keys.insert(link_key) {
                        continue;
                    }

                    let link_id = format!(
                        "link:{commit_sha}:{}:{}",
                        scenario_id, production_artefact.artefact_id
                    );
                    links.push(build_test_link_record(
                        &link_id,
                        &scenario_id,
                        &production_artefact.artefact_id,
                        commit_sha,
                    ));
                    stats.links += 1;
                }
            }
        }
    }
}

pub(crate) fn materialize_enumerated_scenarios(
    repo_id: &str,
    commit_sha: &str,
    production: &[ProductionArtefact],
    production_index: &ProductionIndex,
    scenarios: &[EnumeratedTestScenario],
    artefacts: &mut Vec<ArtefactRecord>,
    links: &mut Vec<TestLinkRecord>,
    link_keys: &mut HashSet<String>,
    stats: &mut StructuralMappingStats,
) {
    let mut synthetic_suites = HashMap::new();

    for enumerated in scenarios {
        let suite_key = format!("{}::{}", enumerated.relative_path, enumerated.suite_name);
        let suite_id = synthetic_suites.entry(suite_key.clone()).or_insert_with(|| {
            stats.suites += 1;
            let suite_id = format!(
                "test_suite:{commit_sha}:{}:{}",
                enumerated.relative_path,
                scenario_id_suffix(&enumerated.suite_name),
            );
            artefacts.push(build_artefact_record(
                &suite_id,
                repo_id,
                commit_sha,
                &enumerated.relative_path,
                &enumerated.language,
                "test_suite",
                Some("enumerated_suite"),
                Some(&enumerated.suite_name),
                None,
                1,
                1,
                Some(&enumerated.suite_name),
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
        artefacts.push(build_artefact_record(
            &scenario_id,
            repo_id,
            commit_sha,
            &enumerated.relative_path,
            &enumerated.language,
            "test_scenario",
            Some("enumerated_test"),
            Some(&scenario_fqn),
            Some(suite_id),
            enumerated.start_line,
            enumerated.start_line.max(1),
            Some(&enumerated.scenario_name),
        ));
        stats.scenarios += 1;
        stats.enumerated_scenarios += 1;

        let synthetic_file = DiscoveredTestFile {
            relative_path: enumerated.relative_path.clone(),
            language: enumerated.language.clone(),
            line_count: 1,
            reference_candidates: if enumerated.relative_path.starts_with("__synthetic_tests__/") {
                Vec::new()
            } else {
                vec![crate::app::test_mapping::model::ReferenceCandidate::SourcePath(
                    enumerated.relative_path.clone(),
                )]
            },
            suites: Vec::new(),
        };
        let synthetic_scenario = crate::app::test_mapping::model::DiscoveredTestScenario {
            name: enumerated.scenario_name.clone(),
            start_line: enumerated.start_line,
            end_line: enumerated.start_line.max(1),
            reference_candidates: enumerated.reference_candidates.clone(),
            discovery_source: enumerated.discovery_source,
        };

        for production_artefact in matched_production_artefacts(
            production,
            production_index,
            &synthetic_file,
            &synthetic_scenario,
        ) {
            let link_key = format!("{}::{}", scenario_id, production_artefact.artefact_id);
            if !link_keys.insert(link_key) {
                continue;
            }
            let link_id = format!(
                "link:{commit_sha}:{}:{}",
                scenario_id, production_artefact.artefact_id
            );
            links.push(build_test_link_record(
                &link_id,
                &scenario_id,
                &production_artefact.artefact_id,
                commit_sha,
            ));
            stats.links += 1;
        }
    }
}

pub(crate) fn build_artefact_record(
    artefact_id: &str,
    repo_id: &str,
    commit_sha: &str,
    path: &str,
    language: &str,
    canonical_kind: &str,
    language_kind: Option<&str>,
    symbol_fqn: Option<&str>,
    parent_artefact_id: Option<&str>,
    start_line: i64,
    end_line: i64,
    signature: Option<&str>,
) -> ArtefactRecord {
    ArtefactRecord {
        artefact_id: artefact_id.to_string(),
        repo_id: repo_id.to_string(),
        commit_sha: commit_sha.to_string(),
        path: path.to_string(),
        language: language.to_string(),
        canonical_kind: canonical_kind.to_string(),
        language_kind: language_kind.map(str::to_string),
        symbol_fqn: symbol_fqn.map(str::to_string),
        parent_artefact_id: parent_artefact_id.map(str::to_string),
        start_line,
        end_line,
        signature: signature.map(str::to_string),
    }
}

pub(crate) fn build_test_link_record(
    test_link_id: &str,
    test_artefact_id: &str,
    production_artefact_id: &str,
    commit_sha: &str,
) -> TestLinkRecord {
    TestLinkRecord {
        test_link_id: test_link_id.to_string(),
        test_artefact_id: test_artefact_id.to_string(),
        production_artefact_id: production_artefact_id.to_string(),
        commit_sha: commit_sha.to_string(),
    }
}
