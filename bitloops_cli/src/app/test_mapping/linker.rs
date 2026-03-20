use std::collections::HashSet;

use crate::app::test_mapping::model::{
    DiscoveredTestFile, DiscoveredTestScenario, ProductionIndex, ReferenceCandidate,
};
use crate::domain::ProductionArtefact;

pub(crate) fn build_production_index(production: &[ProductionArtefact]) -> ProductionIndex {
    let mut index = ProductionIndex::default();

    for (position, artefact) in production.iter().enumerate() {
        index
            .by_simple_symbol
            .entry(symbol_match_key(&artefact.symbol_fqn))
            .or_default()
            .push(position);
        index
            .by_explicit_target
            .insert((artefact.path.clone(), artefact.start_line), position);
    }

    index
}

pub(crate) fn matched_production_artefacts<'a>(
    production: &'a [ProductionArtefact],
    production_index: &ProductionIndex,
    test_file: &DiscoveredTestFile,
    scenario: &DiscoveredTestScenario,
) -> Vec<&'a ProductionArtefact> {
    let imported_paths = source_paths(&test_file.reference_candidates);
    let called_symbols = symbol_candidates(&scenario.reference_candidates);
    let explicit_targets = explicit_target_keys(&scenario.reference_candidates);

    let mut matched_indexes = HashSet::new();
    matched_indexes.extend(match_called_production_artefacts(
        production,
        production_index,
        &imported_paths,
        &called_symbols,
    ));
    matched_indexes.extend(match_explicit_production_targets(
        production_index,
        &explicit_targets,
    ));

    let mut matched_indexes: Vec<usize> = matched_indexes.into_iter().collect();
    matched_indexes.sort_unstable();
    matched_indexes
        .into_iter()
        .map(|index| &production[index])
        .collect()
}

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

pub(crate) fn scenario_id_suffix(name: &str) -> String {
    let normalized = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let collapsed = normalized
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if collapsed.is_empty() {
        "scenario".to_string()
    } else {
        collapsed
    }
}

pub(crate) fn imported_path_matches_production_path(
    imported_path: &str,
    production_path: &str,
) -> bool {
    if imported_path == production_path {
        return true;
    }

    let Some(module_prefix) = imported_module_prefix(imported_path) else {
        return false;
    };

    production_path.starts_with(module_prefix)
        && production_path
            .as_bytes()
            .get(module_prefix.len())
            .is_some_and(|byte| *byte == b'/')
}

pub(crate) fn symbol_match_key(symbol: &str) -> String {
    let simple = symbol
        .rsplit("::")
        .next()
        .unwrap_or(symbol)
        .rsplit('.')
        .next()
        .unwrap_or(symbol);

    let mut normalized = String::new();
    let chars: Vec<char> = simple.chars().collect();

    for (idx, ch) in chars.iter().enumerate() {
        if !ch.is_ascii_alphanumeric() && *ch != '_' {
            continue;
        }

        if ch.is_ascii_uppercase() {
            let prev = idx.checked_sub(1).and_then(|i| chars.get(i)).copied();
            let next = chars.get(idx + 1).copied();
            let needs_separator = idx > 0
                && !normalized.ends_with('_')
                && prev.is_some_and(|prev| {
                    prev.is_ascii_lowercase()
                        || prev.is_ascii_digit()
                        || (prev.is_ascii_uppercase()
                            && next.is_some_and(|next| next.is_ascii_lowercase()))
                });
            if needs_separator {
                normalized.push('_');
            }
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(ch.to_ascii_lowercase());
        }
    }

    normalized.trim_matches('_').to_string()
}

fn source_paths(candidates: &[ReferenceCandidate]) -> HashSet<String> {
    candidates
        .iter()
        .filter_map(|candidate| match candidate {
            ReferenceCandidate::SourcePath(path) => Some(path.clone()),
            _ => None,
        })
        .collect()
}

fn symbol_candidates(candidates: &[ReferenceCandidate]) -> HashSet<String> {
    candidates
        .iter()
        .filter_map(|candidate| match candidate {
            ReferenceCandidate::SymbolName(symbol) | ReferenceCandidate::ScopedSymbol(symbol) => {
                Some(symbol.clone())
            }
            _ => None,
        })
        .collect()
}

fn explicit_target_keys(candidates: &[ReferenceCandidate]) -> HashSet<(String, i64)> {
    candidates
        .iter()
        .filter_map(|candidate| match candidate {
            ReferenceCandidate::ExplicitTarget { path, start_line } => {
                Some((path.clone(), *start_line))
            }
            _ => None,
        })
        .collect()
}

fn match_called_production_artefacts(
    production: &[ProductionArtefact],
    production_index: &ProductionIndex,
    imported_paths: &HashSet<String>,
    called_symbols: &HashSet<String>,
) -> HashSet<usize> {
    let mut matches = HashSet::new();

    for symbol in called_symbols {
        let normalized_called = symbol_match_key(symbol);
        if normalized_called.is_empty() {
            continue;
        }

        let Some(candidate_indexes) = production_index.by_simple_symbol.get(&normalized_called)
        else {
            continue;
        };

        for index in candidate_indexes {
            let artefact = &production[*index];
            if !imported_paths.is_empty()
                && !import_path_set_matches_production_path(imported_paths, &artefact.path)
            {
                continue;
            }
            matches.insert(*index);
        }
    }

    matches
}

fn match_explicit_production_targets(
    production_index: &ProductionIndex,
    explicit_targets: &HashSet<(String, i64)>,
) -> HashSet<usize> {
    explicit_targets
        .iter()
        .filter_map(|target| production_index.by_explicit_target.get(target).copied())
        .collect()
}

fn import_path_set_matches_production_path(
    imported_paths: &HashSet<String>,
    production_path: &str,
) -> bool {
    imported_paths
        .iter()
        .any(|imported_path| imported_path_matches_production_path(imported_path, production_path))
}

fn imported_module_prefix(imported_path: &str) -> Option<&str> {
    imported_path
        .strip_suffix("/mod.rs")
        .or_else(|| imported_path.strip_suffix(".rs"))
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
