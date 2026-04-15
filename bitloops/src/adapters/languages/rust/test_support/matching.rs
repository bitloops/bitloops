use std::collections::HashSet;

use crate::host::language_adapter::ReferenceCandidate;

pub(crate) fn source_scenario_match_keys(
    relative_path: &str,
    suite_name: &str,
    scenario_name: &str,
) -> HashSet<String> {
    let base_name = scenario_base_name(scenario_name);
    let module_segments = rust_module_path_from_relative_path(relative_path);
    let mut keys = HashSet::new();

    let mut variants = Vec::new();
    if !suite_name.is_empty() {
        variants.push(format!("{suite_name}::{base_name}"));
    }
    if !module_segments.is_empty() {
        variants.push(format!("{}::{base_name}", module_segments.join("::")));
        if !suite_name.is_empty() {
            variants.push(format!(
                "{}::{}::{base_name}",
                module_segments.join("::"),
                suite_name
            ));
        }
        for start in 1..module_segments.len() {
            let suffix = module_segments[start..].join("::");
            variants.push(format!("{suffix}::{base_name}"));
            if !suite_name.is_empty() {
                variants.push(format!("{suffix}::{suite_name}::{base_name}"));
            }
        }
    }
    variants.push(base_name.to_string());

    for variant in variants {
        keys.insert(normalized_enumerated_test_key(&variant));
    }
    keys
}

pub(crate) fn doctest_match_keys(
    relative_path: &str,
    scenario_name: &str,
    reference_candidates: &[ReferenceCandidate],
) -> HashSet<String> {
    let mut keys = HashSet::new();
    let item_name = scenario_base_name(scenario_name);
    for candidate in reference_candidates {
        if let ReferenceCandidate::ExplicitTarget { path, start_line } = candidate {
            keys.insert(normalized_enumerated_doctest_key(
                path,
                &item_name,
                *start_line,
            ));
        }
    }
    if keys.is_empty() {
        keys.insert(normalized_enumerated_doctest_key(
            relative_path,
            &item_name,
            0,
        ));
    }
    keys
}

pub(crate) fn scenario_base_name(name: &str) -> String {
    name.split('[').next().unwrap_or(name).trim().to_string()
}

pub(crate) fn normalized_enumerated_test_key(name: &str) -> String {
    name.split(" - ")
        .next()
        .unwrap_or(name)
        .trim()
        .to_ascii_lowercase()
}

pub(crate) fn normalized_enumerated_doctest_key(
    path: &str,
    item_name: &str,
    start_line: i64,
) -> String {
    format!("{}|{}|{}", path, item_name.to_ascii_lowercase(), start_line)
}

fn rust_module_path_from_relative_path(relative_path: &str) -> Vec<String> {
    let path = relative_path.trim_end_matches(".rs");
    let path = path.trim_end_matches("/mod");
    let segments: Vec<&str> = path.split('/').collect();
    let Some(src_index) = segments.iter().position(|segment| *segment == "src") else {
        return Vec::new();
    };
    segments[src_index + 1..]
        .iter()
        .map(|segment| segment.to_string())
        .collect()
}
