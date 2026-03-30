use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser};
use tree_sitter_typescript::LANGUAGE_TYPESCRIPT;

use crate::capability_packs::test_harness::mapping::file_discovery::{
    normalize_join, normalize_rel_path, read_source_file,
};
use crate::capability_packs::test_harness::mapping::model::{
    DiscoveredTestFile, DiscoveredTestScenario, DiscoveredTestSuite, ReferenceCandidate,
    ScenarioDiscoverySource,
};

pub(crate) struct TypeScriptTestMappingHelper {
    parser: Parser,
}

impl TypeScriptTestMappingHelper {
    pub(crate) fn new() -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_TYPESCRIPT.into())
            .context("failed to load TypeScript parser")?;
        Ok(Self { parser })
    }

    pub(crate) fn supports_path(&self, _absolute_path: &Path, relative_path: &str) -> bool {
        relative_path.ends_with(".test.ts")
            || relative_path.ends_with(".spec.ts")
            || relative_path.contains("/__tests__/")
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

        let reference_candidates = collect_typescript_import_paths(root, bytes, relative_path)
            .into_iter()
            .map(ReferenceCandidate::SourcePath)
            .collect();

        Ok(DiscoveredTestFile {
            relative_path: relative_path.to_string(),
            language: "typescript".to_string(),
            reference_candidates,
            suites: collect_typescript_suites(root, bytes),
        })
    }
}

pub(crate) fn collect_typescript_suites(root: Node<'_>, source: &[u8]) -> Vec<DiscoveredTestSuite> {
    let mut suites = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression"
            && is_call_named(node, source, "describe")
            && let Some(suite_name) = extract_first_string_argument(node, source)
            && let Some(callback_body) = extract_second_callback_body(node)
        {
            let scenarios = collect_typescript_scenarios(callback_body, source);
            suites.push(DiscoveredTestSuite {
                name: suite_name,
                start_line: node.start_position().row as i64 + 1,
                end_line: node.end_position().row as i64 + 1,
                scenarios,
            });
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    suites.sort_by_key(|suite| suite.start_line);
    suites
}

pub(crate) fn extract_import_specifier(import_statement: &str) -> Option<&str> {
    let quote = if import_statement.contains('"') {
        '"'
    } else if import_statement.contains('\'') {
        '\''
    } else {
        return None;
    };

    let first = import_statement.find(quote)?;
    let rest = &import_statement[first + 1..];
    let second = rest.find(quote)?;
    Some(&rest[..second])
}

pub(crate) fn resolve_import_to_repo_path(
    test_relative_path: &str,
    import_specifier: &str,
) -> Option<String> {
    if !import_specifier.starts_with('.') {
        return None;
    }

    let test_path = Path::new(test_relative_path);
    let base = test_path.parent()?;
    let combined = normalize_join(base, Path::new(import_specifier));
    let with_extension = if combined.extension().is_none() {
        combined.with_extension("ts")
    } else {
        combined
    };

    Some(normalize_rel_path(&with_extension))
}

pub(crate) fn unquote(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.len() < 2 {
        return None;
    }

    let starts = trimmed.as_bytes()[0] as char;
    let ends = trimmed.as_bytes()[trimmed.len() - 1] as char;
    if (starts == '\'' || starts == '"' || starts == '`') && starts == ends {
        return Some(trimmed[1..trimmed.len() - 1].to_string());
    }
    None
}

fn collect_typescript_import_paths(
    root: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> HashSet<String> {
    let mut results = HashSet::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "import_statement"
            && let Ok(statement) = node.utf8_text(source)
            && let Some(raw_import) = extract_import_specifier(statement)
            && let Some(resolved) = resolve_import_to_repo_path(relative_path, raw_import)
        {
            results.insert(resolved);
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    results
}

fn collect_typescript_scenarios(scope: Node<'_>, source: &[u8]) -> Vec<DiscoveredTestScenario> {
    let mut scenarios = Vec::new();
    let mut stack = vec![scope];

    while let Some(node) = stack.pop() {
        let mut descend = true;

        if node.kind() == "call_expression" {
            let is_test_call =
                is_call_named(node, source, "it") || is_call_named(node, source, "test");
            let is_describe_call = is_call_named(node, source, "describe");

            if is_test_call && let Some(name) = extract_first_string_argument(node, source) {
                let reference_candidates = extract_second_callback_body(node)
                    .map(|body| {
                        collect_typescript_called_symbols(body, source)
                            .into_iter()
                            .map(ReferenceCandidate::SymbolName)
                            .collect()
                    })
                    .unwrap_or_default();

                scenarios.push(DiscoveredTestScenario {
                    name,
                    start_line: node.start_position().row as i64 + 1,
                    end_line: node.end_position().row as i64 + 1,
                    reference_candidates,
                    discovery_source: ScenarioDiscoverySource::Source,
                });
                descend = false;
            }

            if is_describe_call {
                descend = false;
            }
        }

        if descend {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
    }

    scenarios.sort_by_key(|scenario| scenario.start_line);
    scenarios
}

fn collect_typescript_called_symbols(scope: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut symbols = HashSet::new();
    let mut stack = vec![scope];

    while let Some(node) = stack.pop() {
        match node.kind() {
            "call_expression" => {
                if let Some(function_node) = node.child_by_field_name("function") {
                    match function_node.kind() {
                        "identifier" => {
                            if let Ok(name) = function_node.utf8_text(source) {
                                symbols.insert(name.to_string());
                            }
                        }
                        "member_expression" => {
                            if let Some(property) = function_node.child_by_field_name("property")
                                && let Ok(name) = property.utf8_text(source)
                            {
                                symbols.insert(name.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            "new_expression" => {
                if let Some(constructor_node) = node.child_by_field_name("constructor")
                    && let Ok(name) = constructor_node.utf8_text(source)
                {
                    symbols.insert(name.to_string());
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    symbols
}

fn is_call_named(call_expression: Node<'_>, source: &[u8], name: &str) -> bool {
    call_expression
        .child_by_field_name("function")
        .and_then(|node| node.utf8_text(source).ok())
        .is_some_and(|value| value == name)
}

fn extract_first_string_argument(call_expression: Node<'_>, source: &[u8]) -> Option<String> {
    let args = call_expression.child_by_field_name("arguments")?;
    let arg = args.named_child(0)?;
    let raw = arg.utf8_text(source).ok()?;
    unquote(raw)
}

fn extract_second_callback_body(call_expression: Node<'_>) -> Option<Node<'_>> {
    let args = call_expression.child_by_field_name("arguments")?;
    let callback = args.named_child(1)?;
    callback.child_by_field_name("body")
}
