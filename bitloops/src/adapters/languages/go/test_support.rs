use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Deserialize;
use tree_sitter::{Node, Parser};

use crate::host::language_adapter::{
    DiscoveredTestFile, DiscoveredTestScenario, DiscoveredTestSuite, EnumeratedTestScenario,
    EnumerationMode, EnumerationResult, LanguageAdapterContext, LanguageTestSupport,
    ReconciledDiscovery, ReferenceCandidate, ScenarioDiscoverySource,
};

#[derive(Default)]
pub(crate) struct GoLanguageTestSupport;

impl LanguageTestSupport for GoLanguageTestSupport {
    fn language_id(&self) -> &'static str {
        "go"
    }

    fn priority(&self) -> u8 {
        3
    }

    fn supports_path(&self, _absolute_path: &Path, relative_path: &str) -> bool {
        relative_path.ends_with("_test.go")
    }

    fn discover_tests(
        &self,
        absolute_path: &Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        GoTestMappingHelper::new()?.discover_tests(absolute_path, relative_path)
    }

    fn enumerate_tests(&self, ctx: &LanguageAdapterContext) -> EnumerationResult {
        if !ctx.repo_root.join("go.mod").exists() {
            return EnumerationResult::default();
        }

        let output = match ctx.run_command_capture("go", &["test", "-json", "./..."]) {
            Ok(output) => output,
            Err(error) => {
                return EnumerationResult {
                    mode: EnumerationMode::Skipped,
                    scenarios: Vec::new(),
                    notes: vec![format!(
                        "go test enumeration unavailable: {}",
                        error.to_string().replace('\n', " ")
                    )],
                };
            }
        };

        let scenarios = parse_go_test_json_output(&output.combined_output, &ctx.repo_root);
        let mode = if output.success {
            EnumerationMode::Full
        } else if scenarios.is_empty() {
            EnumerationMode::Skipped
        } else {
            EnumerationMode::Partial
        };

        let mut notes = Vec::new();
        if !output.success {
            notes.push("go test returned a non-zero status".to_string());
        }

        EnumerationResult {
            mode,
            scenarios,
            notes,
        }
    }

    fn reconcile(
        &self,
        source_files: &[DiscoveredTestFile],
        enumeration: EnumerationResult,
    ) -> ReconciledDiscovery {
        let mut matched = Vec::new();
        for enumerated in enumeration.scenarios {
            let found = source_files.iter().any(|file| {
                file.language == "go"
                    && file.suites.iter().any(|suite| {
                        suite.scenarios.iter().any(|scenario| {
                            scenario.name == enumerated.scenario_name
                                || format!("{}/{}", scenario.name, enumerated.scenario_name)
                                    == enumerated.scenario_name
                        })
                    })
            });
            if found {
                matched.push(enumerated);
            }
        }
        ReconciledDiscovery {
            enumerated_scenarios: matched,
        }
    }
}

pub(crate) fn go_test_support() -> Arc<dyn LanguageTestSupport> {
    Arc::new(GoLanguageTestSupport)
}

pub(crate) struct GoTestMappingHelper {
    parser: Parser,
}

impl GoTestMappingHelper {
    pub(crate) fn new() -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .context("failed to load Go parser")?;
        Ok(Self { parser })
    }

    pub(crate) fn discover_tests(
        &mut self,
        absolute_path: &Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        let source = std::fs::read_to_string(absolute_path)
            .with_context(|| format!("failed reading test file {}", absolute_path.display()))?;
        let tree = self
            .parser
            .parse(&source, None)
            .with_context(|| format!("failed parsing test file {}", absolute_path.display()))?;
        let root = tree.root_node();
        let bytes = source.as_bytes();

        let reference_candidates = collect_go_import_paths(absolute_path, root, bytes)
            .into_iter()
            .map(ReferenceCandidate::SourcePath)
            .collect();
        let suites = vec![DiscoveredTestSuite {
            name: file_suite_name(relative_path),
            start_line: 1,
            end_line: root.end_position().row as i64 + 1,
            scenarios: collect_go_scenarios(root, bytes),
        }];

        Ok(DiscoveredTestFile {
            relative_path: relative_path.to_string(),
            language: "go".to_string(),
            reference_candidates,
            suites,
        })
    }
}

fn file_suite_name(relative_path: &str) -> String {
    Path::new(relative_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("go_tests")
        .to_string()
}

fn collect_go_scenarios(root: Node<'_>, source: &[u8]) -> Vec<DiscoveredTestScenario> {
    let mut scenarios = Vec::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if child.kind() != "function_declaration" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let Ok(name) = name_node.utf8_text(source) else {
            continue;
        };
        let discovery_source = if name.starts_with("Benchmark") {
            Some(ScenarioDiscoverySource::Source)
        } else if name.starts_with("Fuzz") {
            Some(ScenarioDiscoverySource::Source)
        } else if name.starts_with("Example") {
            Some(ScenarioDiscoverySource::Source)
        } else if name.starts_with("Test") {
            Some(ScenarioDiscoverySource::Source)
        } else {
            None
        };
        let Some(discovery_source) = discovery_source else {
            continue;
        };

        let reference_candidates = child
            .child_by_field_name("body")
            .map(|body| collect_go_called_symbols(body, source))
            .unwrap_or_default()
            .into_iter()
            .map(ReferenceCandidate::SymbolName)
            .collect::<Vec<_>>();
        scenarios.push(DiscoveredTestScenario {
            name: name.to_string(),
            start_line: child.start_position().row as i64 + 1,
            end_line: child.end_position().row as i64 + 1,
            reference_candidates,
            discovery_source,
        });
        scenarios.extend(collect_go_subtests(child, source, name));
    }
    scenarios.sort_by_key(|scenario| scenario.start_line);
    scenarios
}

fn collect_go_subtests(
    function_node: Node<'_>,
    source: &[u8],
    parent_name: &str,
) -> Vec<DiscoveredTestScenario> {
    let mut scenarios = Vec::new();
    let mut stack = vec![function_node];
    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression"
            && let Some(function_expr) = node.child_by_field_name("function")
            && function_expr.kind() == "selector_expression"
            && let Some(field) = function_expr.child_by_field_name("field")
            && field.utf8_text(source).ok() == Some("Run")
            && let Some(arguments) = node.child_by_field_name("arguments")
            && let Some(name_node) = arguments.named_child(0)
            && let Some(name) = string_literal_value(name_node, source)
        {
            scenarios.push(DiscoveredTestScenario {
                name: format!("{parent_name}/{name}"),
                start_line: node.start_position().row as i64 + 1,
                end_line: node.end_position().row as i64 + 1,
                reference_candidates: Vec::new(),
                discovery_source: ScenarioDiscoverySource::Source,
            });
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    scenarios
}

fn collect_go_called_symbols(scope: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut calls = Vec::new();
    let mut stack = vec![scope];
    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression"
            && let Some(function_node) = node.child_by_field_name("function")
        {
            match function_node.kind() {
                "identifier" => {
                    if let Ok(name) = function_node.utf8_text(source) {
                        calls.push(name.to_string());
                    }
                }
                "selector_expression" => {
                    if let Some(field_node) = function_node.child_by_field_name("field")
                        && let Ok(name) = field_node.utf8_text(source)
                    {
                        calls.push(name.to_string());
                    }
                }
                _ => {}
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    calls.sort();
    calls.dedup();
    calls
}

fn collect_go_import_paths(absolute_path: &Path, root: Node<'_>, source: &[u8]) -> Vec<String> {
    let Some((module_root, module_path)) = find_go_module_root(absolute_path) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "import_spec"
            && let Some(path_node) = node.child_by_field_name("path")
            && let Some(import_path) = string_literal_value(path_node, source)
            && import_path.starts_with(&module_path)
            && let Some(relative_package) = import_path
                .strip_prefix(&module_path)
                .map(|value| value.trim_start_matches('/'))
        {
            let package_dir = module_root.join(relative_package);
            if let Ok(relative_dir) = package_dir.strip_prefix(&module_root) {
                let candidate = relative_dir.join("package.go");
                paths.push(candidate.to_string_lossy().to_string());
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn string_literal_value(node: Node<'_>, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?.trim();
    Some(text.trim_matches(['"', '`']).to_string())
}

fn find_go_module_root(path: &Path) -> Option<(std::path::PathBuf, String)> {
    let mut current = path.parent()?;
    loop {
        let go_mod = current.join("go.mod");
        if let Ok(contents) = std::fs::read_to_string(&go_mod) {
            for line in contents.lines() {
                let trimmed = line.trim();
                if let Some(module_path) = trimmed.strip_prefix("module ") {
                    return Some((current.to_path_buf(), module_path.trim().to_string()));
                }
            }
        }
        current = current.parent()?;
    }
}

#[derive(Debug, Deserialize)]
struct GoTestEvent {
    #[serde(rename = "Action")]
    action: Option<String>,
    #[serde(rename = "Package")]
    package: Option<String>,
    #[serde(rename = "Test")]
    test: Option<String>,
}

fn parse_go_test_json_output(output: &str, repo_root: &Path) -> Vec<EnumeratedTestScenario> {
    let package_map = go_package_directory_map(repo_root);
    let mut seen = std::collections::HashSet::new();
    let mut scenarios = Vec::new();
    for line in output.lines() {
        let Ok(event) = serde_json::from_str::<GoTestEvent>(line) else {
            continue;
        };
        let Some(test_name) = event.test else {
            continue;
        };
        let Some(action) = event.action else {
            continue;
        };
        if action != "run" {
            continue;
        }
        let relative_path = event
            .package
            .as_deref()
            .and_then(|package| package_map.get(package))
            .cloned()
            .unwrap_or_default();
        let key = format!("{relative_path}|{test_name}");
        if !seen.insert(key) {
            continue;
        }
        scenarios.push(EnumeratedTestScenario {
            language: "go".to_string(),
            suite_name: relative_path.clone(),
            scenario_name: test_name.clone(),
            relative_path,
            start_line: 0,
            reference_candidates: test_reference_candidates(&test_name),
            discovery_source: ScenarioDiscoverySource::Enumeration,
        });
    }
    scenarios
}

fn go_package_directory_map(repo_root: &Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Some((module_root, module_path)) = find_go_module_root(&repo_root.join("go.mod")) else {
        return map;
    };
    for entry in walkdir::WalkDir::new(&module_root) {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_file() || entry.file_name() != "go.mod" {
            continue;
        }
        let current_root = entry.path().parent().unwrap_or(module_root.as_path());
        if let Some(relative) = current_root.strip_prefix(&module_root).ok() {
            let package_name = if relative.as_os_str().is_empty() {
                module_path.clone()
            } else {
                format!("{}/{}", module_path, relative.to_string_lossy())
            };
            map.insert(package_name, relative.to_string_lossy().to_string());
        }
    }
    map.insert(module_path, String::new());
    map
}

fn test_reference_candidates(test_name: &str) -> Vec<ReferenceCandidate> {
    let stripped = test_name
        .trim_start_matches("Test")
        .trim_start_matches("Benchmark")
        .trim_start_matches("Fuzz")
        .trim_start_matches("Example")
        .split('/')
        .next()
        .unwrap_or("")
        .trim();
    if stripped.is_empty() {
        Vec::new()
    } else {
        vec![ReferenceCandidate::SymbolName(stripped.to_string())]
    }
}

#[cfg(test)]
mod tests {
    use super::{GoTestMappingHelper, parse_go_test_json_output, test_reference_candidates};

    #[test]
    fn go_test_support_discovers_test_benchmark_fuzz_example_and_subtests() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("go.mod"),
            "module github.com/acme/project\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("service_test.go"),
            r#"package service_test

import "testing"

func TestRun(t *testing.T) {
    t.Run("happy", func(t *testing.T) {})
}

func BenchmarkRun(b *testing.B) {}
func FuzzRun(f *testing.F) {}
func ExampleRun() {}
"#,
        )
        .unwrap();

        let discovered = GoTestMappingHelper::new()
            .unwrap()
            .discover_tests(&dir.path().join("service_test.go"), "service_test.go")
            .unwrap();

        let scenario_names = discovered.suites[0]
            .scenarios
            .iter()
            .map(|scenario| scenario.name.as_str())
            .collect::<Vec<_>>();

        assert!(scenario_names.contains(&"TestRun"));
        assert!(scenario_names.contains(&"TestRun/happy"));
        assert!(scenario_names.contains(&"BenchmarkRun"));
        assert!(scenario_names.contains(&"FuzzRun"));
        assert!(scenario_names.contains(&"ExampleRun"));
    }

    #[test]
    fn go_test_json_parser_extracts_enumerated_scenarios() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("go.mod"),
            "module github.com/acme/project\n",
        )
        .unwrap();
        let output = r#"{"Time":"2026-03-31T00:00:00Z","Action":"run","Package":"github.com/acme/project","Test":"TestRun"}
{"Time":"2026-03-31T00:00:01Z","Action":"run","Package":"github.com/acme/project","Test":"BenchmarkRun"}
"#;

        let scenarios = parse_go_test_json_output(output, dir.path());

        assert_eq!(scenarios.len(), 2);
        assert_eq!(scenarios[0].scenario_name, "TestRun");
        assert_eq!(
            scenarios[0].reference_candidates,
            test_reference_candidates("TestRun")
        );
        assert_eq!(scenarios[1].scenario_name, "BenchmarkRun");
    }
}
