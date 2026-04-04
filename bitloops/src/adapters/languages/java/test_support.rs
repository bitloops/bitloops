use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use regex::Regex;
use tree_sitter::{Node, Parser};

use crate::host::language_adapter::{
    DiscoveredTestFile, DiscoveredTestScenario, DiscoveredTestSuite, EnumeratedTestScenario,
    EnumerationMode, EnumerationResult, LanguageAdapterContext, LanguageTestSupport,
    ReconciledDiscovery, ReferenceCandidate, ScenarioDiscoverySource,
};

#[derive(Default)]
pub(crate) struct JavaLanguageTestSupport;

impl LanguageTestSupport for JavaLanguageTestSupport {
    fn language_id(&self) -> &'static str {
        "java"
    }

    fn priority(&self) -> u8 {
        4
    }

    fn supports_path(&self, _absolute_path: &Path, relative_path: &str) -> bool {
        relative_path.ends_with("Test.java")
            || relative_path.ends_with("Tests.java")
            || relative_path.ends_with("IT.java")
            || relative_path.contains("/src/test/java/")
    }

    fn discover_tests(
        &self,
        absolute_path: &Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        JavaTestMappingHelper::new()?.discover_tests(absolute_path, relative_path)
    }

    fn enumerate_tests(&self, ctx: &LanguageAdapterContext) -> EnumerationResult {
        if ctx.repo_root.join("pom.xml").exists() {
            return enumerate_with_maven(ctx);
        }
        if ctx.repo_root.join("gradlew").exists()
            || ctx.repo_root.join("build.gradle").exists()
            || ctx.repo_root.join("build.gradle.kts").exists()
        {
            return enumerate_with_gradle(ctx);
        }
        EnumerationResult {
            mode: EnumerationMode::Skipped,
            scenarios: Vec::new(),
            notes: vec![
                "java test enumeration unavailable: no Maven or Gradle build detected".to_string(),
            ],
        }
    }

    fn reconcile(
        &self,
        source_files: &[DiscoveredTestFile],
        enumeration: EnumerationResult,
    ) -> ReconciledDiscovery {
        let source_keys = source_files
            .iter()
            .filter(|file| file.language == "java")
            .flat_map(|file| {
                let relative_path = file.relative_path.clone();
                file.suites.iter().flat_map(move |suite| {
                    let suite_name = suite.name.clone();
                    suite.scenarios.iter().map({
                        let relative_path = relative_path.clone();
                        move |scenario| {
                            normalized_java_test_key(&relative_path, &suite_name, &scenario.name)
                        }
                    })
                })
            })
            .collect::<HashSet<_>>();

        let enumerated_scenarios = enumeration
            .scenarios
            .into_iter()
            .filter(|scenario| {
                !source_keys.contains(&normalized_java_test_key(
                    &scenario.relative_path,
                    &scenario.suite_name,
                    &scenario.scenario_name,
                ))
            })
            .collect();

        ReconciledDiscovery {
            enumerated_scenarios,
        }
    }
}

pub(crate) fn java_test_support() -> Arc<dyn LanguageTestSupport> {
    Arc::new(JavaLanguageTestSupport)
}

pub(crate) struct JavaTestMappingHelper {
    parser: Parser,
}

impl JavaTestMappingHelper {
    pub(crate) fn new() -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .context("failed to load Java parser")?;
        Ok(Self { parser })
    }

    pub(crate) fn discover_tests(
        &mut self,
        absolute_path: &Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        let source = fs::read_to_string(absolute_path)
            .with_context(|| format!("failed reading test file {}", absolute_path.display()))?;
        let tree = self
            .parser
            .parse(&source, None)
            .with_context(|| format!("failed parsing test file {}", absolute_path.display()))?;
        let root = tree.root_node();
        let bytes = source.as_bytes();

        let reference_candidates = collect_java_import_paths(root, bytes, relative_path)
            .into_iter()
            .map(ReferenceCandidate::SourcePath)
            .collect();

        Ok(DiscoveredTestFile {
            relative_path: relative_path.to_string(),
            language: "java".to_string(),
            reference_candidates,
            suites: collect_java_suites(root, bytes),
        })
    }
}

fn collect_java_suites(root: Node<'_>, source: &[u8]) -> Vec<DiscoveredTestSuite> {
    let mut suites = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if matches!(node.kind(), "class_declaration" | "enum_declaration")
            && let Some(body) = node.child_by_field_name("body")
        {
            let scenarios = collect_java_scenarios(body, source);
            if !scenarios.is_empty() {
                let suite_name = node
                    .child_by_field_name("name")
                    .and_then(|name| name.utf8_text(source).ok())
                    .map(str::to_string)
                    .unwrap_or_else(|| "JavaTest".to_string());
                suites.push(DiscoveredTestSuite {
                    name: suite_name,
                    start_line: node.start_position().row as i64 + 1,
                    end_line: node.end_position().row as i64 + 1,
                    scenarios,
                });
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    suites.sort_by_key(|suite| suite.start_line);
    suites
}

fn collect_java_scenarios(scope: Node<'_>, source: &[u8]) -> Vec<DiscoveredTestScenario> {
    let mut scenarios = Vec::new();
    let mut cursor = scope.walk();
    for child in scope.named_children(&mut cursor) {
        if child.kind() != "method_declaration" || !java_method_is_test(child, source) {
            continue;
        }
        let name = child
            .child_by_field_name("name")
            .and_then(|name_node| name_node.utf8_text(source).ok())
            .map(str::to_string)
            .unwrap_or_else(|| "test".to_string());
        let reference_candidates = child
            .child_by_field_name("body")
            .map(|body| {
                collect_java_called_symbols(body, source)
                    .into_iter()
                    .map(ReferenceCandidate::SymbolName)
                    .collect()
            })
            .unwrap_or_default();
        scenarios.push(DiscoveredTestScenario {
            name,
            start_line: child.start_position().row as i64 + 1,
            end_line: child.end_position().row as i64 + 1,
            reference_candidates,
            discovery_source: ScenarioDiscoverySource::Source,
        });
    }
    scenarios.sort_by_key(|scenario| scenario.start_line);
    scenarios
}

fn java_method_is_test(node: Node<'_>, source: &[u8]) -> bool {
    let method_name = node
        .child_by_field_name("name")
        .and_then(|name| name.utf8_text(source).ok())
        .unwrap_or_default();
    if method_name.starts_with("test") {
        return true;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut inner_cursor = child.walk();
            for annotation in child.named_children(&mut inner_cursor) {
                if !matches!(annotation.kind(), "annotation" | "marker_annotation") {
                    continue;
                }
                let Some(name_node) = annotation.child_by_field_name("name") else {
                    continue;
                };
                let Ok(name) = name_node.utf8_text(source) else {
                    continue;
                };
                let annotation_name = name.rsplit('.').next().unwrap_or(name);
                if matches!(
                    annotation_name,
                    "Test" | "ParameterizedTest" | "RepeatedTest" | "TestFactory" | "TestTemplate"
                ) {
                    return true;
                }
            }
        }
    }

    false
}

fn collect_java_called_symbols(scope: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut symbols = HashSet::new();
    let mut stack = vec![scope];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "method_invocation" => {
                if let Some(name_node) = node.child_by_field_name("name")
                    && let Ok(name) = name_node.utf8_text(source)
                {
                    symbols.insert(name.to_string());
                }
            }
            "object_creation_expression" => {
                if let Some(type_node) = node.child_by_field_name("type")
                    && let Ok(name) = type_node.utf8_text(source)
                {
                    let short = name.rsplit('.').next().unwrap_or(name).to_string();
                    if !short.is_empty() {
                        symbols.insert(short);
                    }
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    symbols
}

fn collect_java_import_paths(root: Node<'_>, source: &[u8], relative_path: &str) -> Vec<String> {
    let mut results = HashSet::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "import_declaration"
            && let Ok(statement) = node.utf8_text(source)
            && let Some(import_path) = parse_import_path(statement)
        {
            for candidate in resolve_java_import_to_repo_paths(relative_path, &import_path) {
                results.insert(candidate);
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    let mut results = results.into_iter().collect::<Vec<_>>();
    results.sort();
    results
}

fn parse_import_path(statement: &str) -> Option<String> {
    let trimmed = statement.trim().trim_end_matches(';').trim();
    let rest = trimmed.strip_prefix("import")?.trim();
    let rest = rest.strip_prefix("static ").unwrap_or(rest).trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

fn resolve_java_import_to_repo_paths(
    test_relative_path: &str,
    import_specifier: &str,
) -> Vec<String> {
    if import_specifier.is_empty() || import_specifier.ends_with(".*") {
        return Vec::new();
    }

    let package_path = import_specifier.replace('.', "/");
    let mut results = vec![
        format!("src/main/java/{package_path}.java"),
        format!("src/test/java/{package_path}.java"),
    ];

    if test_relative_path.contains("/src/test/java/") {
        let file_name = Path::new(test_relative_path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        if !file_name.is_empty() {
            results.push(format!("src/test/java/{package_path}/{file_name}"));
        }
    }

    results.sort();
    results.dedup();
    results
}

fn enumerate_with_maven(ctx: &LanguageAdapterContext) -> EnumerationResult {
    let output = match ctx.run_command_capture("mvn", &["-q", "-DfailIfNoTests=false", "test"]) {
        Ok(output) => output,
        Err(error) => {
            return EnumerationResult {
                mode: EnumerationMode::Skipped,
                scenarios: Vec::new(),
                notes: vec![format!(
                    "java maven enumeration unavailable: {}",
                    error.to_string().replace('\n', " ")
                )],
            };
        }
    };

    let scenarios = collect_junit_report_scenarios(
        &ctx.repo_root,
        &["target/surefire-reports", "target/failsafe-reports"],
    );
    EnumerationResult {
        mode: if output.success {
            EnumerationMode::Full
        } else if scenarios.is_empty() {
            EnumerationMode::Skipped
        } else {
            EnumerationMode::Partial
        },
        scenarios,
        notes: (!output.success)
            .then(|| "maven test command returned a non-zero status".to_string())
            .into_iter()
            .collect(),
    }
}

fn enumerate_with_gradle(ctx: &LanguageAdapterContext) -> EnumerationResult {
    let (program, args) = if ctx.repo_root.join("gradlew").exists() {
        ("./gradlew", vec!["test", "--console=plain"])
    } else {
        ("gradle", vec!["test", "--console=plain"])
    };
    let output = match ctx.run_command_capture(program, &args) {
        Ok(output) => output,
        Err(error) => {
            return EnumerationResult {
                mode: EnumerationMode::Skipped,
                scenarios: Vec::new(),
                notes: vec![format!(
                    "java gradle enumeration unavailable: {}",
                    error.to_string().replace('\n', " ")
                )],
            };
        }
    };

    let scenarios = collect_junit_report_scenarios(&ctx.repo_root, &["build/test-results/test"]);
    EnumerationResult {
        mode: if output.success {
            EnumerationMode::Full
        } else if scenarios.is_empty() {
            EnumerationMode::Skipped
        } else {
            EnumerationMode::Partial
        },
        scenarios,
        notes: (!output.success)
            .then(|| "gradle test command returned a non-zero status".to_string())
            .into_iter()
            .collect(),
    }
}

fn collect_junit_report_scenarios(
    repo_root: &Path,
    report_dirs: &[&str],
) -> Vec<EnumeratedTestScenario> {
    let testcase_pattern = Regex::new(r#"<testcase\b[^>]*classname="([^"]+)"[^>]*name="([^"]+)""#)
        .expect("java testcase regex should compile");
    let testcase_pattern_name_first =
        Regex::new(r#"<testcase\b[^>]*name="([^"]+)"[^>]*classname="([^"]+)""#)
            .expect("java testcase regex should compile");
    let mut seen = HashSet::new();
    let mut scenarios = Vec::new();

    for report_dir in report_dirs {
        let root = repo_root.join(report_dir);
        if !root.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|ext| ext.to_str()) != Some("xml")
            {
                continue;
            }
            let Ok(contents) = fs::read_to_string(entry.path()) else {
                continue;
            };

            for captures in testcase_pattern.captures_iter(&contents) {
                let Some(classname) = captures.get(1).map(|value| value.as_str()) else {
                    continue;
                };
                let Some(name) = captures.get(2).map(|value| value.as_str()) else {
                    continue;
                };
                push_enumerated_java_scenario(&mut scenarios, &mut seen, classname, name);
            }
            for captures in testcase_pattern_name_first.captures_iter(&contents) {
                let Some(name) = captures.get(1).map(|value| value.as_str()) else {
                    continue;
                };
                let Some(classname) = captures.get(2).map(|value| value.as_str()) else {
                    continue;
                };
                push_enumerated_java_scenario(&mut scenarios, &mut seen, classname, name);
            }
        }
    }

    scenarios.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then(left.suite_name.cmp(&right.suite_name))
            .then(left.scenario_name.cmp(&right.scenario_name))
    });
    scenarios
}

fn push_enumerated_java_scenario(
    scenarios: &mut Vec<EnumeratedTestScenario>,
    seen: &mut HashSet<String>,
    classname: &str,
    scenario_name: &str,
) {
    let suite_name = classname
        .rsplit('.')
        .next()
        .unwrap_or(classname)
        .to_string();
    let relative_path = format!("src/test/java/{}.java", classname.replace('.', "/"));
    let key = normalized_java_test_key(&relative_path, &suite_name, scenario_name);
    if !seen.insert(key) {
        return;
    }
    scenarios.push(EnumeratedTestScenario {
        language: "java".to_string(),
        suite_name,
        scenario_name: scenario_name.to_string(),
        relative_path,
        start_line: 0,
        reference_candidates: test_reference_candidates(scenario_name),
        discovery_source: ScenarioDiscoverySource::Enumeration,
    });
}

fn test_reference_candidates(test_name: &str) -> Vec<ReferenceCandidate> {
    let stripped = test_name
        .trim_start_matches("test")
        .trim_start_matches("should")
        .trim_start_matches("when")
        .split('_')
        .next()
        .unwrap_or("")
        .trim();
    if stripped.is_empty() {
        Vec::new()
    } else {
        vec![ReferenceCandidate::SymbolName(stripped.to_string())]
    }
}

fn normalized_java_test_key(relative_path: &str, suite_name: &str, scenario_name: &str) -> String {
    format!(
        "{}::{}::{}",
        relative_path.trim().to_ascii_lowercase(),
        suite_name.trim().to_ascii_lowercase(),
        scenario_name.trim().to_ascii_lowercase()
    )
}

#[allow(dead_code)]
fn normalize_rel_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::{
        JavaLanguageTestSupport, JavaTestMappingHelper, collect_junit_report_scenarios,
        resolve_java_import_to_repo_paths,
    };
    use crate::host::language_adapter::LanguageTestSupport;

    #[test]
    fn java_test_support_recognizes_java_test_paths() {
        let support = JavaLanguageTestSupport;
        assert!(support.supports_path(
            std::path::Path::new(""),
            "src/test/java/com/acme/GreeterTest.java"
        ));
        assert!(support.supports_path(std::path::Path::new(""), "app/GreeterIT.java"));
        assert!(!support.supports_path(
            std::path::Path::new(""),
            "src/main/java/com/acme/Greeter.java"
        ));
    }

    #[test]
    fn java_test_support_discovers_junit_source_scenarios() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("GreeterTest.java");
        std::fs::write(
            &file_path,
            r#"package com.acme;

import org.junit.jupiter.api.Test;

class GreeterTest {
    @Test
    void greets() {
        helper();
    }

    void helper() {}
}
"#,
        )
        .unwrap();

        let discovered = JavaTestMappingHelper::new()
            .unwrap()
            .discover_tests(&file_path, "src/test/java/com/acme/GreeterTest.java")
            .unwrap();

        assert_eq!(discovered.language, "java");
        assert_eq!(discovered.suites.len(), 1);
        assert_eq!(discovered.suites[0].name, "GreeterTest");
        assert_eq!(discovered.suites[0].scenarios[0].name, "greets");
        assert_eq!(
            discovered.suites[0].scenarios[0].reference_candidates,
            vec![
                crate::host::language_adapter::ReferenceCandidate::SymbolName("helper".to_string())
            ]
        );
    }

    #[test]
    fn resolve_java_import_to_repo_paths_prefers_main_and_test_layouts() {
        let paths = resolve_java_import_to_repo_paths(
            "src/test/java/com/acme/GreeterTest.java",
            "com.acme.Greeter",
        );

        assert!(paths.contains(&"src/main/java/com/acme/Greeter.java".to_string()));
        assert!(paths.contains(&"src/test/java/com/acme/Greeter.java".to_string()));
    }

    #[test]
    fn java_junit_report_parser_extracts_scenarios() {
        let dir = tempfile::tempdir().unwrap();
        let report_dir = dir.path().join("build/test-results/test");
        std::fs::create_dir_all(&report_dir).unwrap();
        std::fs::write(
            report_dir.join("TEST-com.acme.GreeterTest.xml"),
            r#"<testsuite tests="1">
  <testcase classname="com.acme.GreeterTest" name="greets"/>
</testsuite>"#,
        )
        .unwrap();

        let scenarios = collect_junit_report_scenarios(dir.path(), &["build/test-results/test"]);
        assert_eq!(scenarios.len(), 1);
        assert_eq!(scenarios[0].suite_name, "GreeterTest");
        assert_eq!(scenarios[0].scenario_name, "greets");
        assert_eq!(
            scenarios[0].relative_path,
            "src/test/java/com/acme/GreeterTest.java"
        );
    }
}
