use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser};

use super::extraction::using_target_name;
use crate::host::language_adapter::{
    DiscoveredTestFile, DiscoveredTestScenario, DiscoveredTestSuite, LanguageTestSupport,
    ReferenceCandidate, ScenarioDiscoverySource,
};

#[derive(Default)]
pub(crate) struct CSharpTestSupport;

impl LanguageTestSupport for CSharpTestSupport {
    fn language_id(&self) -> &'static str {
        "csharp"
    }

    fn priority(&self) -> u8 {
        50
    }

    fn supports_path(&self, _absolute_path: &Path, relative_path: &str) -> bool {
        supports_csharp_test_path(relative_path)
    }

    fn discover_tests(
        &self,
        absolute_path: &Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        CSharpTestMappingHelper::new()?.discover_tests(absolute_path, relative_path)
    }
}

pub(crate) fn csharp_test_support() -> Arc<dyn LanguageTestSupport> {
    Arc::new(CSharpTestSupport)
}

struct CSharpTestMappingHelper {
    parser: Parser,
}

impl CSharpTestMappingHelper {
    fn new() -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .context("failed to load C# parser")?;
        Ok(Self { parser })
    }

    fn discover_tests(
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

        Ok(DiscoveredTestFile {
            relative_path: relative_path.to_string(),
            language: "csharp".to_string(),
            reference_candidates: collect_csharp_import_paths(root, bytes)
                .into_iter()
                .map(ReferenceCandidate::SourcePath)
                .collect(),
            suites: collect_csharp_test_suites(root, bytes, relative_path),
        })
    }
}

fn supports_csharp_test_path(relative_path: &str) -> bool {
    if !relative_path.ends_with(".cs") {
        return false;
    }
    let file_name = Path::new(relative_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let has_test_name = file_name.contains("Test") || file_name.contains("Spec");
    let in_test_dir = Path::new(relative_path).components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|part| part.ends_with("Tests"))
    });
    has_test_name || in_test_dir
}

fn collect_csharp_test_suites(
    root: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> Vec<DiscoveredTestSuite> {
    let mut suites = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if matches!(
            node.kind(),
            "class_declaration" | "record_declaration" | "struct_declaration"
        ) {
            let scenarios = collect_csharp_test_methods(node, source);
            if !scenarios.is_empty() {
                let suite_name = node
                    .child_by_field_name("name")
                    .and_then(|name| name.utf8_text(source).ok())
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        Path::new(relative_path)
                            .file_stem()
                            .and_then(|stem| stem.to_str())
                            .unwrap_or("csharp_tests")
                            .to_string()
                    });
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

fn collect_csharp_test_methods(
    type_node: Node<'_>,
    source: &[u8],
) -> Vec<DiscoveredTestScenario> {
    let mut scenarios = Vec::new();
    let mut cursor = type_node.walk();
    for child in type_node.named_children(&mut cursor) {
        if child.kind() != "method_declaration" || !has_test_attribute(child, source) {
            continue;
        }
        let name = child
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(str::to_string)
            .unwrap_or_else(|| "test".to_string());
        scenarios.push(DiscoveredTestScenario {
            name,
            start_line: child.start_position().row as i64 + 1,
            end_line: child.end_position().row as i64 + 1,
            reference_candidates: child
                .child_by_field_name("body")
                .map(|body| collect_csharp_called_symbols(body, source))
                .unwrap_or_default()
                .into_iter()
                .map(ReferenceCandidate::SymbolName)
                .collect(),
            discovery_source: ScenarioDiscoverySource::Source,
        });
    }
    scenarios.sort_by_key(|scenario| scenario.start_line);
    scenarios
}

fn has_test_attribute(method_node: Node<'_>, source: &[u8]) -> bool {
    let mut current = method_node.prev_named_sibling();
    while let Some(node) = current {
        if node.kind() != "attribute_list" {
            break;
        }
        let mut stack = vec![node];
        while let Some(candidate) = stack.pop() {
            if candidate.kind() == "attribute"
                && let Some(name_node) = candidate.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(source)
            {
                let bare_name = name.rsplit('.').next().unwrap_or(name).trim();
                if matches!(bare_name, "Fact" | "Test" | "TestMethod") {
                    return true;
                }
            }
            let mut cursor = candidate.walk();
            for child in candidate.named_children(&mut cursor) {
                stack.push(child);
            }
        }
        current = node.prev_named_sibling();
    }
    false
}

fn collect_csharp_called_symbols(scope: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut symbols = HashSet::new();
    let mut stack = vec![scope];

    while let Some(node) = stack.pop() {
        if node.kind() == "invocation_expression"
            && let Some(function_node) = node.child_by_field_name("function")
        {
            match function_node.kind() {
                "identifier_name" => {
                    if let Ok(name) = function_node.utf8_text(source) {
                        symbols.insert(name.to_string());
                    }
                }
                "member_access_expression" => {
                    if let Some(name_node) = function_node.child_by_field_name("name")
                        && let Ok(name) = name_node.utf8_text(source)
                    {
                        symbols.insert(name.to_string());
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

    let mut result = symbols.into_iter().collect::<Vec<_>>();
    result.sort();
    result
}

fn collect_csharp_import_paths(root: Node<'_>, source: &[u8]) -> Vec<String> {
    let content = std::str::from_utf8(source).unwrap_or_default();
    let mut paths = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "using_directive"
            && let Some(name) = using_target_name(node, content)
        {
            let normalized = name.trim().replace('.', "/");
            if !normalized.is_empty() {
                paths.insert(format!("{normalized}.cs"));
            }
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }

    let mut result = paths.into_iter().collect::<Vec<_>>();
    result.sort();
    result
}
