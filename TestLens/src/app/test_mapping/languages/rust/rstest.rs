use tree_sitter::Node;

use super::attributes::{
    display_rstest_argument, extract_leading_rust_attributes, extract_rust_apply_template_name,
    extract_rust_attribute_args, extract_rust_parameter_name, rust_attribute_name,
    rust_function_attributes, split_top_level_arguments, summarize_rstest_values,
};
use super::scenarios::RustScenarioSeed;

pub(crate) fn collect_rust_rstest_templates(
    root: Node<'_>,
    source: &str,
) -> std::collections::HashMap<String, Vec<RustScenarioSeed>> {
    let mut templates = std::collections::HashMap::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "function_item" {
            let attributes = rust_function_attributes(node);
            let raw_attributes: Vec<String> = attributes
                .iter()
                .filter_map(|attribute| {
                    attribute
                        .utf8_text(source.as_bytes())
                        .ok()
                        .map(str::to_string)
                })
                .collect();

            if raw_attributes
                .iter()
                .any(|attribute| rust_attribute_name(attribute).as_deref() == Some("template"))
                && let Some(name) = node
                    .child_by_field_name("name")
                    .and_then(|name| name.utf8_text(source.as_bytes()).ok())
            {
                let seeds =
                    build_rust_rstest_cases(name, node, source, &raw_attributes, 0, true);
                if !seeds.is_empty() {
                    templates.insert(name.to_string(), seeds);
                }
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    templates
}

pub(crate) fn build_rust_rstest_cases(
    function_name: &str,
    function_node: Node<'_>,
    source: &str,
    raw_attributes: &[String],
    function_end_line: i64,
    allow_template_output: bool,
) -> Vec<RustScenarioSeed> {
    let has_rstest = raw_attributes
        .iter()
        .any(|attribute| rust_attribute_name(attribute).as_deref() == Some("rstest"));
    let is_template = raw_attributes
        .iter()
        .any(|attribute| rust_attribute_name(attribute).as_deref() == Some("template"));
    if !has_rstest && !is_template {
        return Vec::new();
    }
    if is_template && !allow_template_output {
        return Vec::new();
    }

    let mut cases = Vec::new();
    for attribute in rust_function_attributes(function_node) {
        let Ok(raw_attribute) = attribute.utf8_text(source.as_bytes()) else {
            continue;
        };
        if rust_attribute_name(raw_attribute).as_deref() != Some("case") {
            continue;
        }

        let body = extract_rust_attribute_args(raw_attribute).unwrap_or_default();
        let display = summarize_rstest_values(&split_top_level_arguments(body));
        let name = if display.is_empty() {
            format!("{function_name}[case]")
        } else {
            format!("{function_name}[{display}]")
        };
        cases.push(RustScenarioSeed::plain(
            name,
            attribute.start_position().row as i64 + 1,
            function_end_line,
        ));
    }
    if !cases.is_empty() {
        return cases;
    }

    let parameter_expansions = extract_rstest_parameter_expansions(function_node, source.as_bytes());
    if !parameter_expansions.is_empty() {
        if parameter_expansions.iter().all(|expansion| expansion.statically_visible) {
            let combinations = cross_product_rstest_values(&parameter_expansions);
            if !combinations.is_empty() {
                return combinations
                    .into_iter()
                    .map(|labels| {
                        RustScenarioSeed::plain(
                            format!("{function_name}[{}]", labels.join(", ")),
                            function_node.start_position().row as i64 + 1,
                            function_end_line,
                        )
                    })
                    .collect();
            }
        }

        return vec![RustScenarioSeed::plain(
            function_name.to_string(),
            function_node.start_position().row as i64 + 1,
            function_end_line,
        )];
    }

    if has_rstest {
        return vec![RustScenarioSeed::plain(
            function_name.to_string(),
            function_node.start_position().row as i64 + 1,
            function_end_line,
        )];
    }

    Vec::new()
}

pub(crate) fn applied_template_name(raw_attributes: &[String]) -> Option<String> {
    extract_rust_apply_template_name(raw_attributes)
}

#[derive(Debug, Clone)]
struct RstestParameterExpansion {
    name: String,
    values: Vec<String>,
    statically_visible: bool,
}

fn extract_rstest_parameter_expansions(
    function_node: Node<'_>,
    source: &[u8],
) -> Vec<RstestParameterExpansion> {
    let Some(parameters_node) = function_node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let Ok(parameters_raw) = parameters_node.utf8_text(source) else {
        return Vec::new();
    };
    let parameters_raw = parameters_raw
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')');
    let mut expansions = Vec::new();

    for parameter in split_top_level_arguments(parameters_raw) {
        let attributes = extract_leading_rust_attributes(parameter);
        if attributes.is_empty() {
            continue;
        }
        let Some(name) = extract_rust_parameter_name(parameter) else {
            continue;
        };

        for attribute in attributes {
            match rust_attribute_name(attribute).as_deref() {
                Some("values") => expansions.push(RstestParameterExpansion {
                    name: name.clone(),
                    values: split_top_level_arguments(
                        extract_rust_attribute_args(attribute).unwrap_or_default(),
                    )
                    .into_iter()
                    .map(display_rstest_argument)
                    .collect(),
                    statically_visible: true,
                }),
                Some("files") => {
                    let values: Vec<String> = split_top_level_arguments(
                        extract_rust_attribute_args(attribute).unwrap_or_default(),
                    )
                    .into_iter()
                    .map(display_rstest_argument)
                    .collect();
                    let statically_visible =
                        values.iter().all(|value| !value.contains('*') && !value.contains('?'));
                    expansions.push(RstestParameterExpansion {
                        name: name.clone(),
                        values,
                        statically_visible,
                    });
                }
                _ => {}
            }
        }
    }

    expansions
}

fn cross_product_rstest_values(expansions: &[RstestParameterExpansion]) -> Vec<Vec<String>> {
    let mut rows = vec![Vec::new()];
    for expansion in expansions {
        let mut next = Vec::new();
        for row in &rows {
            for value in &expansion.values {
                let mut extended = row.clone();
                extended.push(format!("{}={}", expansion.name, value));
                next.push(extended);
            }
        }
        rows = next;
    }
    rows
}
