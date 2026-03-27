use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser};
use tree_sitter_python::LANGUAGE as LANGUAGE_PYTHON;

use crate::capability_packs::test_harness::mapping::file_discovery::{
    normalize_join, normalize_rel_path, read_source_file,
};
use crate::capability_packs::test_harness::mapping::model::{
    DiscoveredTestFile, DiscoveredTestScenario, DiscoveredTestSuite, ReferenceCandidate,
    ScenarioDiscoverySource,
};
use crate::capability_packs::test_harness::mapping::registry::LanguageProvider;

pub(crate) struct PythonLanguageProvider {
    parser: Parser,
}

impl PythonLanguageProvider {
    pub(crate) fn new() -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_PYTHON.into())
            .context("failed to load Python parser")?;
        Ok(Self { parser })
    }
}

impl LanguageProvider for PythonLanguageProvider {
    fn language_id(&self) -> &'static str {
        "python"
    }

    fn priority(&self) -> u8 {
        2
    }

    fn supports_path(&self, _absolute_path: &Path, relative_path: &str) -> bool {
        let file_name = Path::new(relative_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        relative_path.ends_with(".py")
            && (file_name.starts_with("test_")
                || file_name.ends_with("_test.py")
                || relative_path.starts_with("tests/")
                || relative_path.contains("/tests/"))
    }

    fn discover_tests(
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

        let reference_candidates = collect_python_import_paths(root, bytes, relative_path)
            .into_iter()
            .map(ReferenceCandidate::SourcePath)
            .collect();

        Ok(DiscoveredTestFile {
            relative_path: relative_path.to_string(),
            language: self.language_id().to_string(),
            reference_candidates,
            suites: collect_python_suites(root, bytes, relative_path),
        })
    }
}

pub(crate) fn collect_python_suites(
    root: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> Vec<DiscoveredTestSuite> {
    let mut suites = Vec::new();
    let mut module_scenarios = Vec::new();
    let module_name = Path::new(relative_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("module")
        .to_string();

    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        let node = unwrap_python_definition(child);
        match node.kind() {
            "function_definition" => {
                if is_python_test_function(node, source) {
                    module_scenarios.push(build_python_scenario(node, source));
                }
            }
            "class_definition" => {
                let scenarios = collect_python_class_scenarios(node, source);
                if scenarios.is_empty() {
                    continue;
                }
                let suite_name = node
                    .child_by_field_name("name")
                    .and_then(|name| name.utf8_text(source).ok())
                    .map(str::to_string)
                    .unwrap_or_else(|| "TestCase".to_string());
                suites.push(DiscoveredTestSuite {
                    name: suite_name,
                    start_line: node.start_position().row as i64 + 1,
                    end_line: node.end_position().row as i64 + 1,
                    scenarios,
                });
            }
            _ => {}
        }
    }

    if !module_scenarios.is_empty() {
        suites.push(DiscoveredTestSuite {
            name: module_name,
            start_line: 1,
            end_line: root.end_position().row as i64 + 1,
            scenarios: module_scenarios,
        });
    }

    suites.sort_by_key(|suite| suite.start_line);
    suites
}

pub(crate) fn collect_python_import_paths(
    root: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> HashSet<String> {
    let mut results = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        match node.kind() {
            "import_statement" => {
                let mut cursor = node.walk();
                for child in node.children_by_field_name("name", &mut cursor) {
                    let imported = if child.kind() == "aliased_import" {
                        child.child_by_field_name("name")
                    } else {
                        Some(child)
                    };
                    let Some(imported) = imported else {
                        continue;
                    };
                    let Ok(raw_import) = imported.utf8_text(source) else {
                        continue;
                    };
                    if let Some(resolved) =
                        resolve_python_import_to_repo_path(relative_path, raw_import)
                    {
                        results.insert(resolved);
                    }
                }
            }
            "import_from_statement" => {
                let Some(module_name) = node.child_by_field_name("module_name") else {
                    continue;
                };
                let Ok(raw_import) = module_name.utf8_text(source) else {
                    continue;
                };
                if let Some(resolved) =
                    resolve_python_import_to_repo_path(relative_path, raw_import)
                {
                    results.insert(resolved);
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    results
}

pub(crate) fn resolve_python_import_to_repo_path(
    test_relative_path: &str,
    import_specifier: &str,
) -> Option<String> {
    let import_specifier = import_specifier.trim();
    if import_specifier.is_empty() || import_specifier == "__future__" {
        return None;
    }

    if import_specifier.starts_with('.') {
        return resolve_relative_python_import_to_repo_path(test_relative_path, import_specifier);
    }

    let normalized = import_specifier.replace('.', "/");
    if normalized.is_empty() {
        return None;
    }
    Some(format!("{normalized}.py"))
}

fn resolve_relative_python_import_to_repo_path(
    test_relative_path: &str,
    import_specifier: &str,
) -> Option<String> {
    let leading_dots = import_specifier.chars().take_while(|ch| *ch == '.').count();
    if leading_dots == 0 {
        return None;
    }
    let remainder = import_specifier[leading_dots..].replace('.', "/");
    let test_path = Path::new(test_relative_path);
    let mut base = test_path.parent()?.to_path_buf();
    for _ in 1..leading_dots {
        base.pop();
    }
    let relative = if remainder.is_empty() {
        Path::new("__init__.py").to_path_buf()
    } else {
        Path::new(&format!("{remainder}.py")).to_path_buf()
    };
    Some(normalize_rel_path(&normalize_join(&base, &relative)))
}

fn collect_python_class_scenarios(
    class_node: Node<'_>,
    source: &[u8],
) -> Vec<DiscoveredTestScenario> {
    let Some(body) = class_node.child_by_field_name("body") else {
        return Vec::new();
    };
    let mut scenarios = Vec::new();
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        let node = unwrap_python_definition(child);
        if node.kind() != "function_definition" || !is_python_test_function(node, source) {
            continue;
        }
        scenarios.push(build_python_scenario(node, source));
    }
    scenarios.sort_by_key(|scenario| scenario.start_line);
    scenarios
}

fn build_python_scenario(node: Node<'_>, source: &[u8]) -> DiscoveredTestScenario {
    let name = node
        .child_by_field_name("name")
        .and_then(|name| name.utf8_text(source).ok())
        .map(str::to_string)
        .unwrap_or_else(|| "test".to_string());
    let reference_candidates = node
        .child_by_field_name("body")
        .map(|body| {
            collect_python_called_symbols(body, source)
                .into_iter()
                .map(ReferenceCandidate::SymbolName)
                .collect()
        })
        .unwrap_or_default();

    DiscoveredTestScenario {
        name,
        start_line: node.start_position().row as i64 + 1,
        end_line: node.end_position().row as i64 + 1,
        reference_candidates,
        discovery_source: ScenarioDiscoverySource::Source,
    }
}

fn collect_python_called_symbols(scope: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut symbols = HashSet::new();
    let mut stack = vec![scope];

    while let Some(node) = stack.pop() {
        if node.kind() == "call"
            && let Some(function_node) = node.child_by_field_name("function")
        {
            match function_node.kind() {
                "identifier" => {
                    if let Ok(name) = function_node.utf8_text(source) {
                        symbols.insert(name.to_string());
                    }
                }
                "attribute" => {
                    if let Some(attribute) = function_node.child_by_field_name("attribute")
                        && let Ok(name) = attribute.utf8_text(source)
                    {
                        symbols.insert(name.to_string());
                    }
                }
                _ => {}
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    symbols
}

fn unwrap_python_definition(node: Node<'_>) -> Node<'_> {
    if node.kind() == "decorated_definition" {
        node.child_by_field_name("definition").unwrap_or(node)
    } else {
        node
    }
}

fn is_python_test_function(node: Node<'_>, source: &[u8]) -> bool {
    node.child_by_field_name("name")
        .and_then(|name| name.utf8_text(source).ok())
        .is_some_and(|name| name.starts_with("test_"))
}
