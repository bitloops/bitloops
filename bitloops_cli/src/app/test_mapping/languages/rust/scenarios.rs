use tree_sitter::Node;

use crate::app::test_mapping::model::{
    DiscoveredTestScenario, DiscoveredTestSuite, ReferenceCandidate, ScenarioDiscoverySource,
};

use super::attributes::{
    build_rust_parameterized_test_case, rust_attribute_is_parameterized_test,
    rust_attribute_is_test, rust_attribute_name, rust_function_attributes,
};
use super::doctests::collect_rust_doctest_scenarios;
use super::macros::{collect_rust_macro_generated_scenarios, collect_rust_proptest_scenarios};
use super::rstest::{
    applied_template_name, build_rust_rstest_cases, collect_rust_rstest_templates,
};

#[derive(Debug, Clone)]
pub(crate) struct RustDiscoveredScenario {
    pub(crate) suite_name: String,
    pub(crate) suite_start_line: i64,
    pub(crate) suite_end_line: i64,
    pub(crate) scenario: DiscoveredTestScenario,
}

#[derive(Debug, Clone)]
pub(crate) struct RustScenarioSeed {
    pub(crate) name: String,
    pub(crate) start_line: i64,
    pub(crate) end_line: i64,
    pub(crate) reference_candidates: Vec<ReferenceCandidate>,
    pub(crate) discovery_source: ScenarioDiscoverySource,
}

impl RustScenarioSeed {
    pub(crate) fn plain(name: String, start_line: i64, end_line: i64) -> Self {
        Self {
            name,
            start_line,
            end_line,
            reference_candidates: Vec::new(),
            discovery_source: ScenarioDiscoverySource::Source,
        }
    }
}

pub(crate) fn collect_rust_suites(
    root: Node<'_>,
    source: &str,
    relative_path: &str,
) -> Vec<DiscoveredTestSuite> {
    let discovered = collect_rust_test_scenarios(root, source, relative_path);
    let mut grouped: std::collections::BTreeMap<String, DiscoveredTestSuite> =
        std::collections::BTreeMap::new();

    for item in discovered {
        let entry = grouped
            .entry(item.suite_name.clone())
            .or_insert_with(|| DiscoveredTestSuite {
                name: item.suite_name.clone(),
                start_line: item.suite_start_line,
                end_line: item.suite_end_line,
                scenarios: Vec::new(),
            });

        entry.start_line = entry.start_line.min(item.suite_start_line);
        entry.end_line = entry.end_line.max(item.suite_end_line);
        entry.scenarios.push(item.scenario);
    }

    let mut suites: Vec<DiscoveredTestSuite> = grouped.into_values().collect();
    for suite in &mut suites {
        suite.scenarios.sort_by_key(|scenario| scenario.start_line);
    }
    suites.sort_by(|a, b| a.start_line.cmp(&b.start_line).then(a.name.cmp(&b.name)));
    suites
}

fn collect_rust_test_scenarios(
    root: Node<'_>,
    source: &str,
    relative_path: &str,
) -> Vec<RustDiscoveredScenario> {
    let bytes = source.as_bytes();
    let rstest_templates = collect_rust_rstest_templates(root, source);
    let mut scenarios = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "function_item" {
            let scenario_seeds = rust_test_scenarios_for_function(node, source, &rstest_templates);
            if scenario_seeds.is_empty() {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    stack.push(child);
                }
                continue;
            }

            let body_candidates = node
                .child_by_field_name("body")
                .map(|body| collect_rust_called_symbols(body, bytes))
                .unwrap_or_default();

            let (suite_name, suite_start_line, suite_end_line) =
                rust_suite_for_function(node, bytes, relative_path);

            for seed in scenario_seeds {
                let mut reference_candidates = body_candidates.clone();
                reference_candidates.extend(seed.reference_candidates);

                scenarios.push(RustDiscoveredScenario {
                    suite_name: suite_name.clone(),
                    suite_start_line,
                    suite_end_line,
                    scenario: DiscoveredTestScenario {
                        name: seed.name,
                        start_line: seed.start_line,
                        end_line: seed.end_line,
                        reference_candidates,
                        discovery_source: seed.discovery_source,
                    },
                });
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    scenarios.extend(collect_rust_macro_generated_scenarios(
        root,
        bytes,
        relative_path,
    ));
    scenarios.extend(collect_rust_proptest_scenarios(root, source, relative_path));
    scenarios.extend(collect_rust_doctest_scenarios(root, source, relative_path));
    scenarios.sort_by(|a, b| {
        a.suite_start_line
            .cmp(&b.suite_start_line)
            .then(a.scenario.start_line.cmp(&b.scenario.start_line))
    });
    scenarios
}

fn rust_test_scenarios_for_function(
    function_node: Node<'_>,
    source: &str,
    rstest_templates: &std::collections::HashMap<String, Vec<RustScenarioSeed>>,
) -> Vec<RustScenarioSeed> {
    let source_bytes = source.as_bytes();
    if function_node.kind() != "function_item" {
        return Vec::new();
    }

    let Some(function_name) = function_node
        .child_by_field_name("name")
        .and_then(|name| name.utf8_text(source_bytes).ok())
        .map(str::to_string)
    else {
        return Vec::new();
    };

    let function_start_line = function_node.start_position().row as i64 + 1;
    let function_end_line = function_node.end_position().row as i64 + 1;

    let attributes = rust_function_attributes(function_node);
    let mut has_plain_test = false;
    let mut cases = Vec::new();
    let raw_attributes: Vec<String> = attributes
        .iter()
        .filter_map(|attribute| attribute.utf8_text(source_bytes).ok().map(str::to_string))
        .collect();
    if raw_attributes
        .iter()
        .any(|attribute| rust_attribute_name(attribute).as_deref() == Some("template"))
    {
        return Vec::new();
    }

    for attribute in attributes {
        if rust_attribute_is_test(attribute, source_bytes) {
            has_plain_test = true;
        }

        if rust_attribute_is_parameterized_test(attribute, source_bytes) {
            cases.push(build_rust_parameterized_test_case(
                &function_name,
                attribute,
                source_bytes,
                function_end_line,
            ));
        }
    }

    if let Some(template_name) = applied_template_name(&raw_attributes) {
        if let Some(template_cases) = rstest_templates.get(&template_name) {
            return template_cases
                .iter()
                .map(|seed| RustScenarioSeed {
                    name: seed.name.replacen(&template_name, &function_name, 1),
                    start_line: function_start_line,
                    end_line: function_end_line,
                    reference_candidates: seed.reference_candidates.clone(),
                    discovery_source: ScenarioDiscoverySource::Source,
                })
                .collect();
        }

        return vec![RustScenarioSeed::plain(
            function_name,
            function_start_line,
            function_end_line,
        )];
    }

    let rstest_cases = build_rust_rstest_cases(
        &function_name,
        function_node,
        source,
        &raw_attributes,
        function_end_line,
        false,
    );
    if !rstest_cases.is_empty() {
        return rstest_cases;
    }

    if !cases.is_empty() {
        return cases;
    }

    if has_plain_test {
        return vec![RustScenarioSeed::plain(
            function_name,
            function_start_line,
            function_end_line,
        )];
    }

    Vec::new()
}

pub(crate) fn rust_suite_for_node(
    node: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> (String, i64, i64) {
    let mut module_names = Vec::new();
    let mut suite_range: Option<(i64, i64)> = None;

    let mut parent = node.parent();
    while let Some(current) = parent {
        if current.kind() == "mod_item" {
            if let Some(name) = current
                .child_by_field_name("name")
                .and_then(|name| name.utf8_text(source).ok())
            {
                module_names.push(name.to_string());
            }

            suite_range.get_or_insert((
                current.start_position().row as i64 + 1,
                current.end_position().row as i64 + 1,
            ));
        }
        parent = current.parent();
    }

    if module_names.is_empty() {
        let fallback_name = std::path::Path::new(relative_path)
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("rust_tests")
            .to_string();
        (
            fallback_name,
            node.start_position().row as i64 + 1,
            node.end_position().row as i64 + 1,
        )
    } else {
        module_names.reverse();
        let (start_line, end_line) = suite_range.unwrap_or((
            node.start_position().row as i64 + 1,
            node.end_position().row as i64 + 1,
        ));
        (module_names.join("::"), start_line, end_line)
    }
}

fn rust_suite_for_function(
    node: Node<'_>,
    source: &[u8],
    relative_path: &str,
) -> (String, i64, i64) {
    rust_suite_for_node(node, source, relative_path)
}

fn collect_rust_called_symbols(scope: Node<'_>, source: &[u8]) -> Vec<ReferenceCandidate> {
    let mut references = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![scope];

    while let Some(node) = stack.pop() {
        if node.kind() == "call_expression"
            && let Some(function_node) = node.child_by_field_name("function")
            && let Some(symbol) = extract_rust_callable_symbol(function_node, source)
            && seen.insert(symbol.clone())
        {
            references.push(ReferenceCandidate::SymbolName(symbol));
        }
        if node.kind() == "macro_invocation"
            && let Ok(raw_invocation) = node.utf8_text(source)
            && let Some(body) = super::macros::extract_rust_macro_invocation_body(raw_invocation)
        {
            for symbol in body
                .split(|ch: char| {
                    !ch.is_ascii_alphanumeric() && ch != '_' && ch != ':' && ch != '.'
                })
                .collect::<Vec<_>>()
            {
                let simple = symbol
                    .rsplit("::")
                    .next()
                    .unwrap_or(symbol)
                    .rsplit('.')
                    .next()
                    .unwrap_or(symbol)
                    .trim();
                if !simple.is_empty()
                    && simple
                        .chars()
                        .next()
                        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
                    && seen.insert(simple.to_string())
                {
                    references.push(ReferenceCandidate::SymbolName(simple.to_string()));
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    references
}

fn extract_rust_callable_symbol(function_node: Node<'_>, source: &[u8]) -> Option<String> {
    let raw = function_node.utf8_text(source).ok()?.trim();
    if raw.is_empty() {
        return None;
    }

    let symbol = match function_node.kind() {
        "identifier" => raw.to_string(),
        "field_expression" => function_node
            .child_by_field_name("field")
            .and_then(|field| field.utf8_text(source).ok())
            .map(str::to_string)
            .or_else(|| raw.rsplit('.').next().map(str::to_string))
            .unwrap_or_else(|| raw.to_string()),
        "scoped_identifier" | "scoped_type_identifier" => {
            raw.rsplit("::").next().unwrap_or(raw).to_string()
        }
        _ => raw
            .rsplit("::")
            .next()
            .unwrap_or(raw)
            .rsplit('.')
            .next()
            .unwrap_or(raw)
            .to_string(),
    };

    if symbol.is_empty() {
        None
    } else {
        Some(symbol)
    }
}
