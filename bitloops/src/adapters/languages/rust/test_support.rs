mod attributes;
mod doctests;
pub(crate) mod enumeration;
pub(crate) mod imports;
pub(crate) mod macros;
mod matching;
mod rstest;
pub(crate) mod scenarios;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::Parser;
use tree_sitter_rust::LANGUAGE as LANGUAGE_RUST;

use self::enumeration::{parse_enumerated_doctests, parse_enumerated_host_tests};
use self::matching::{
    doctest_match_keys, normalized_enumerated_doctest_key, normalized_enumerated_test_key,
    scenario_base_name, source_scenario_match_keys,
};
use crate::host::language_adapter::{
    DiscoveredTestFile, EnumerationMode, EnumerationResult, LanguageAdapterContext,
    LanguageTestSupport, ReconciledDiscovery, ReferenceCandidate, ScenarioDiscoverySource,
};

#[derive(Default)]
pub(crate) struct RustLanguageTestSupport;

impl LanguageTestSupport for RustLanguageTestSupport {
    fn language_id(&self) -> &'static str {
        "rust"
    }

    fn priority(&self) -> u8 {
        0
    }

    fn supports_path(&self, absolute_path: &std::path::Path, relative_path: &str) -> bool {
        RustTestMappingHelper::supports_path(absolute_path, relative_path)
    }

    fn discover_tests(
        &self,
        absolute_path: &std::path::Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        RustTestMappingHelper::new()?.discover_tests(absolute_path, relative_path)
    }

    fn enumerate_tests(&self, ctx: &LanguageAdapterContext) -> EnumerationResult {
        if !ctx.repo_root.join("Cargo.toml").exists() {
            return EnumerationResult::default();
        }

        let host_output =
            ctx.run_command_capture("cargo", &["test", "--workspace", "--", "--list"]);
        let doc_output =
            ctx.run_command_capture("cargo", &["test", "--workspace", "--doc", "--", "--list"]);

        let mut result = EnumerationResult::default();
        let mut full_success = true;

        match host_output {
            Ok(output) if output.success => {
                result
                    .scenarios
                    .extend(parse_enumerated_host_tests(&output.combined_output));
            }
            Ok(output) => {
                full_success = false;
                result.notes.push(format!(
                    "host enumeration unavailable: {}",
                    output.combined_output.replace('\n', " ")
                ));
            }
            Err(error) => {
                full_success = false;
                result.notes.push(format!(
                    "host enumeration unavailable: {}",
                    error.to_string().replace('\n', " ")
                ));
            }
        }

        match doc_output {
            Ok(output) if output.success => {
                result
                    .scenarios
                    .extend(parse_enumerated_doctests(&output.combined_output));
            }
            Ok(output) => {
                full_success = false;
                result.notes.push(format!(
                    "doctest enumeration unavailable: {}",
                    output.combined_output.replace('\n', " ")
                ));
            }
            Err(error) => {
                full_success = false;
                result.notes.push(format!(
                    "doctest enumeration unavailable: {}",
                    error.to_string().replace('\n', " ")
                ));
            }
        }

        result.mode = if result.notes.is_empty() && full_success {
            EnumerationMode::Full
        } else if !result.scenarios.is_empty() {
            EnumerationMode::Partial
        } else {
            EnumerationMode::Skipped
        };
        result
    }

    fn reconcile(
        &self,
        source_files: &[DiscoveredTestFile],
        enumeration: EnumerationResult,
    ) -> ReconciledDiscovery {
        RustTestMappingHelper::reconcile(source_files, enumeration)
    }
}

pub(crate) fn rust_test_support() -> Arc<dyn LanguageTestSupport> {
    Arc::new(RustLanguageTestSupport)
}

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

    pub(crate) fn supports_path(absolute_path: &Path, relative_path: &str) -> bool {
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
                        let item_name = scenario_base_name(&scenario.scenario_name);
                        scenario
                            .reference_candidates
                            .iter()
                            .find_map(|candidate| match candidate {
                                ReferenceCandidate::ExplicitTarget { path, start_line } => {
                                    Some(normalized_enumerated_doctest_key(
                                        path,
                                        &item_name,
                                        *start_line,
                                    ))
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| {
                                normalized_enumerated_doctest_key(
                                    &scenario.relative_path,
                                    &item_name,
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

fn looks_like_inline_rust_test_source(absolute_path: &Path, relative_path: &str) -> bool {
    if !relative_path.ends_with(".rs") {
        return false;
    }
    if !(relative_path.starts_with("src/") || relative_path.contains("/src/")) {
        return false;
    }

    let Ok(source) = fs::read_to_string(absolute_path) else {
        return false;
    };

    rust_source_contains_test_markers(&source) || rust_source_contains_doctest_markers(&source)
}

fn rust_source_contains_test_markers(source: &str) -> bool {
    source.contains("#[cfg(test)]")
        || source.contains("#[test")
        || source.contains("::test")
        || source.contains("#[test_case")
        || source.contains("::test_case")
        || source.contains("#[rstest")
        || source.contains("::rstest")
        || source.contains("#[wasm_bindgen_test")
        || source.contains("::wasm_bindgen_test")
        || source.contains("#[quickcheck")
        || source.contains("::quickcheck")
        || source.contains("proptest!")
}

pub(crate) fn rust_source_contains_doctest_markers(source: &str) -> bool {
    let mut in_block_doc = false;

    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("/// ```") || trimmed.starts_with("//! ```") {
            return true;
        }

        if trimmed.starts_with("/**") || trimmed.starts_with("/*!") {
            in_block_doc = true;
            if trimmed.contains("```") {
                return true;
            }
        } else if in_block_doc && trimmed.contains("```") {
            return true;
        }

        if in_block_doc && trimmed.contains("*/") {
            in_block_doc = false;
        }
    }

    false
}

fn read_source_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed reading test file {}", path.display()))
}

fn normalize_rel_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::RustTestMappingHelper;
    use crate::host::language_adapter::{
        DiscoveredTestFile, DiscoveredTestScenario, DiscoveredTestSuite, EnumerationMode,
        EnumerationResult, EnumeratedTestScenario, ReferenceCandidate, ScenarioDiscoverySource,
    };

    #[test]
    fn reconcile_dedupes_enumerated_doctest_when_source_doctest_exists() {
        let source_files = vec![DiscoveredTestFile {
            relative_path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            reference_candidates: Vec::new(),
            suites: vec![DiscoveredTestSuite {
                name: "crate::doctests".to_string(),
                start_line: 1,
                end_line: 1,
                scenarios: vec![DiscoveredTestScenario {
                    name: "sample::documented_increment[doctest:12]".to_string(),
                    start_line: 12,
                    end_line: 15,
                    reference_candidates: vec![ReferenceCandidate::ExplicitTarget {
                        path: "src/lib.rs".to_string(),
                        start_line: 12,
                    }],
                    discovery_source: ScenarioDiscoverySource::Doctest,
                }],
            }],
        }];

        let enumeration = EnumerationResult {
            mode: EnumerationMode::Full,
            scenarios: vec![EnumeratedTestScenario {
                language: "rust".to_string(),
                suite_name: "src::lib.rs::doctests".to_string(),
                scenario_name: "sample::documented_increment[doctest:12]".to_string(),
                relative_path: "src/lib.rs".to_string(),
                start_line: 12,
                reference_candidates: vec![ReferenceCandidate::ExplicitTarget {
                    path: "src/lib.rs".to_string(),
                    start_line: 12,
                }],
                discovery_source: ScenarioDiscoverySource::Doctest,
            }],
            notes: Vec::new(),
        };

        let reconciled = RustTestMappingHelper::reconcile(&source_files, enumeration);

        assert!(
            reconciled.enumerated_scenarios.is_empty(),
            "expected source doctest to suppress the matching enumerated doctest, got {:?}",
            reconciled.enumerated_scenarios
        );
    }
}
