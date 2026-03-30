mod attributes;
mod doctests;
pub(crate) mod enumeration;
pub(crate) mod imports;
pub(crate) mod macros;
mod rstest;
pub(crate) mod scenarios;

use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Parser;
use tree_sitter_rust::LANGUAGE as LANGUAGE_RUST;

use crate::capability_packs::test_harness::mapping::file_discovery::{
    looks_like_inline_rust_test_source, read_source_file,
};
use crate::capability_packs::test_harness::mapping::linker::{
    doctest_match_keys, normalized_enumerated_doctest_key, normalized_enumerated_test_key,
    source_scenario_match_keys,
};
use crate::capability_packs::test_harness::mapping::model::{
    DiscoveredTestFile, EnumerationResult, ReconciledDiscovery, ReferenceCandidate,
    ScenarioDiscoverySource,
};

pub(crate) struct RustTestMappingHelper {
    parser: Parser,
}

impl RustTestMappingHelper {
    pub(crate) fn new() -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_RUST.into())
            .context("failed to load Rust parser")?;
        Ok(Self { parser })
    }

    pub(crate) fn supports_path(&self, absolute_path: &Path, relative_path: &str) -> bool {
        relative_path.ends_with(".test.rs")
            || relative_path.ends_with(".spec.rs")
            || ((relative_path.starts_with("tests/") || relative_path.contains("/tests/"))
                && relative_path.ends_with(".rs"))
            || looks_like_inline_rust_test_source(absolute_path, relative_path)
    }

    pub(crate) fn discover_tests(
        &mut self,
        absolute_path: &Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        let source = read_source_file(absolute_path)?;
        let tree = self
            .parser
            .parse(&source, None)
            .with_context(|| format!("failed parsing test file {}", absolute_path.display()))?;
        let bytes = source.as_bytes();
        let root = tree.root_node();

        let mut reference_candidates: Vec<ReferenceCandidate> =
            imports::collect_rust_import_paths_for(root, bytes, relative_path)
                .into_iter()
                .map(ReferenceCandidate::SourcePath)
                .collect();
        reference_candidates.extend(
            imports::collect_rust_scoped_call_import_paths_for(root, bytes, relative_path)
                .into_iter()
                .map(ReferenceCandidate::SourcePath),
        );
        reference_candidates.extend(
            imports::rust_test_context_source_paths(relative_path)
                .into_iter()
                .map(ReferenceCandidate::SourcePath),
        );

        Ok(DiscoveredTestFile {
            relative_path: relative_path.to_string(),
            language: "rust".to_string(),
            reference_candidates,
            suites: scenarios::collect_rust_suites(root, &source, relative_path),
        })
    }

    pub(crate) fn reconcile(
        &self,
        source_files: &[DiscoveredTestFile],
        enumeration: EnumerationResult,
    ) -> ReconciledDiscovery {
        let mut source_scenario_keys = std::collections::HashSet::new();
        let mut source_doctest_keys = std::collections::HashSet::new();

        for file in source_files {
            if file.language != "rust" {
                continue;
            }

            for suite in &file.suites {
                for scenario in &suite.scenarios {
                    source_scenario_keys.extend(source_scenario_match_keys(
                        &file.relative_path,
                        &suite.name,
                        &scenario.name,
                    ));
                    if scenario.discovery_source == ScenarioDiscoverySource::Doctest {
                        source_doctest_keys.extend(doctest_match_keys(
                            &file.relative_path,
                            &scenario.name,
                            &scenario.reference_candidates,
                        ));
                    }
                }
            }
        }

        let enumerated_scenarios = enumeration
            .scenarios
            .into_iter()
            .filter(|scenario| {
                let normalized_key =
                    if scenario.discovery_source == ScenarioDiscoverySource::Doctest {
                        scenario
                            .reference_candidates
                            .iter()
                            .find_map(|candidate| match candidate {
                                ReferenceCandidate::ExplicitTarget { path, start_line } => {
                                    Some(normalized_enumerated_doctest_key(
                                        path,
                                        &scenario.scenario_name,
                                        *start_line,
                                    ))
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| {
                                normalized_enumerated_doctest_key(
                                    &scenario.relative_path,
                                    &scenario.scenario_name,
                                    0,
                                )
                            })
                    } else {
                        normalized_enumerated_test_key(&format!(
                            "{}::{}",
                            scenario.suite_name, scenario.scenario_name
                        ))
                    };

                if scenario.discovery_source == ScenarioDiscoverySource::Doctest {
                    !source_doctest_keys.contains(&normalized_key)
                } else {
                    !source_scenario_keys.contains(&normalized_key)
                }
            })
            .collect();

        ReconciledDiscovery {
            enumerated_scenarios,
        }
    }
}
