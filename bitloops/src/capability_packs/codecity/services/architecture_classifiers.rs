use std::collections::{BTreeMap, BTreeSet};

use super::community_detection::CodeCityCommunityResult;
use super::config::CodeCityConfig;
use super::graph_metrics::{
    FileGraph, articulation_component_increase, collapse_strongly_connected_components,
    compute_depth_scores, infer_graph_layering, strongly_connected_components,
};
use super::source_graph::CodeCitySourceGraph;
use crate::capability_packs::codecity::types::{
    CodeCityArchitectureEvidence, CodeCityArchitecturePattern, CodeCityArchitectureScores,
    CodeCityBoundary, CodeCityBoundaryArchitectureReport, CodeCityBoundaryGraphMetrics,
    CodeCityDiagnostic,
};

pub fn classify_boundary_architecture(
    boundary: &CodeCityBoundary,
    graph: &FileGraph,
    metrics: &CodeCityBoundaryGraphMetrics,
    communities: &CodeCityCommunityResult,
    source: &CodeCitySourceGraph,
    config: &CodeCityConfig,
) -> CodeCityBoundaryArchitectureReport {
    let depths = compute_depth_scores(graph);
    let sccs = strongly_connected_components(graph);
    let collapsed = collapse_strongly_connected_components(graph, &sccs);
    let layering = infer_graph_layering(graph, &sccs, &collapsed);

    let mut diagnostics = Vec::new();
    let layered = layered_score(metrics, &layering);
    let hexagonal = hexagonal_score(graph, &depths, source, boundary, &mut diagnostics);
    let modular = modular_score(graph, communities);
    let event_driven = event_driven_score(graph, metrics, source, boundary, config);
    let pipe_and_filter = pipe_and_filter_score(metrics);
    let ball_of_mud = mud_score(metrics);

    let scores = CodeCityArchitectureScores {
        layered,
        hexagonal,
        modular,
        event_driven,
        pipe_and_filter,
        ball_of_mud,
    };

    let constructive = vec![
        (CodeCityArchitecturePattern::Layered, layered),
        (CodeCityArchitecturePattern::Hexagonal, hexagonal),
        (CodeCityArchitecturePattern::Modular, modular),
        (CodeCityArchitecturePattern::EventDriven, event_driven),
        (CodeCityArchitecturePattern::PipeAndFilter, pipe_and_filter),
    ];

    let mut ranked = constructive.clone();
    ranked.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let (primary_pattern, primary_score) = if ranked[0].1 < 0.3 {
        if ball_of_mud > config.architecture.mud_warning_threshold {
            (CodeCityArchitecturePattern::BallOfMud, ball_of_mud)
        } else {
            (CodeCityArchitecturePattern::Unclassified, 0.0)
        }
    } else {
        (ranked[0].0, ranked[0].1)
    };

    let secondary_pattern = ranked
        .iter()
        .skip(1)
        .find(|(_, score)| *score > config.architecture.secondary_pattern_threshold)
        .map(|(pattern, _)| *pattern);
    let secondary_score = ranked
        .iter()
        .skip(1)
        .find(|(_, score)| *score > config.architecture.secondary_pattern_threshold)
        .map(|(_, score)| *score);

    let evidence = vec![
        CodeCityArchitectureEvidence {
            name: "layer_clarity".to_string(),
            value: layering.layer_clarity,
            description: "Share of inter-layer edges that move exactly one layer.".to_string(),
        },
        CodeCityArchitectureEvidence {
            name: "core_periphery_score".to_string(),
            value: metrics.core_periphery_score,
            description: "Directional separation between outer and inner files.".to_string(),
        },
        CodeCityArchitectureEvidence {
            name: "modularity".to_string(),
            value: metrics.modularity,
            description: "Community separation score for the boundary.".to_string(),
        },
        CodeCityArchitectureEvidence {
            name: "mud_score".to_string(),
            value: ball_of_mud,
            description: "Composite density, cycle, and weak-structure warning score.".to_string(),
        },
    ];

    if ball_of_mud > config.architecture.mud_warning_threshold {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.architecture.mud_warning".to_string(),
            severity: "warning".to_string(),
            message: format!(
                "Boundary `{}` exceeded the mud-warning threshold ({:.2}).",
                boundary.id, ball_of_mud
            ),
            path: None,
            boundary_id: Some(boundary.id.clone()),
        });
    }

    CodeCityBoundaryArchitectureReport {
        boundary_id: boundary.id.clone(),
        primary_pattern,
        primary_score,
        secondary_pattern,
        secondary_score,
        scores,
        metrics: metrics.clone(),
        evidence,
        diagnostics,
    }
}

fn layered_score(
    metrics: &CodeCityBoundaryGraphMetrics,
    layering: &super::graph_metrics::GraphLayering,
) -> f64 {
    let valid_layer_count_bonus = if (2..=6).contains(&layering.layer_count) {
        1.0
    } else {
        0.3
    };
    ((1.0 - metrics.back_edge_ratio) * 0.6
        + layering.layer_clarity * 0.3
        + valid_layer_count_bonus * 0.1)
        .clamp(0.0, 1.0)
}

fn hexagonal_score(
    graph: &FileGraph,
    depths: &BTreeMap<String, f64>,
    source: &CodeCitySourceGraph,
    boundary: &CodeCityBoundary,
    diagnostics: &mut Vec<CodeCityDiagnostic>,
) -> f64 {
    let mut compliant = 0usize;
    for &(from, to) in &graph.edges {
        let left = depths.get(&graph.paths[from]).copied().unwrap_or(0.5);
        let right = depths.get(&graph.paths[to]).copied().unwrap_or(0.5);
        if left <= right {
            compliant += 1;
        }
    }
    let radial_compliance = if graph.edges.is_empty() {
        0.5
    } else {
        compliant as f64 / graph.edges.len() as f64
    };

    let core_paths = depths
        .iter()
        .filter_map(|(path, depth)| (*depth > 0.7).then_some(path.clone()))
        .collect::<BTreeSet<_>>();
    let periphery_paths = depths
        .iter()
        .filter_map(|(path, depth)| (*depth < 0.3).then_some(path.clone()))
        .collect::<BTreeSet<_>>();

    let core_isolation = if core_paths.is_empty() || periphery_paths.is_empty() {
        0.3
    } else {
        let core_hints = source
            .external_dependency_hints
            .iter()
            .filter(|hint| core_paths.contains(&hint.from_path))
            .count();
        if core_hints == 0 {
            diagnostics.push(CodeCityDiagnostic {
                code: "codecity.architecture.core_isolation_partial".to_string(),
                severity: "info".to_string(),
                message: format!(
                    "Boundary `{}` had no external dependency evidence for core-isolation scoring.",
                    boundary.id
                ),
                path: None,
                boundary_id: Some(boundary.id.clone()),
            });
            0.5
        } else {
            1.0 / (core_hints as f64 + 1.0)
        }
    };

    let core_periphery_score = {
        let core_count = core_paths.len() as f64;
        let periphery_count = periphery_paths.len() as f64;
        if core_count > 0.0 && periphery_count > 0.0 {
            1.0
        } else {
            0.4
        }
    };

    (radial_compliance * 0.5 + core_periphery_score * 0.3 + core_isolation * 0.2).clamp(0.0, 1.0)
}

fn modular_score(graph: &FileGraph, communities: &CodeCityCommunityResult) -> f64 {
    let community_by_path = &communities.community_by_path;
    let mut cross_participants = BTreeSet::new();
    let mut bridge_counts = BTreeMap::<String, BTreeSet<String>>::new();

    for &(from, to) in &graph.edges {
        let from_path = &graph.paths[from];
        let to_path = &graph.paths[to];
        let Some(from_community) = community_by_path.get(from_path) else {
            continue;
        };
        let Some(to_community) = community_by_path.get(to_path) else {
            continue;
        };
        if from_community == to_community {
            continue;
        }
        cross_participants.insert(from_path.clone());
        cross_participants.insert(to_path.clone());
        bridge_counts
            .entry(from_path.clone())
            .or_default()
            .insert(to_community.clone());
        bridge_counts
            .entry(to_path.clone())
            .or_default()
            .insert(from_community.clone());
    }

    let interface_narrowness = if graph.paths.is_empty() {
        0.0
    } else {
        1.0 - cross_participants.len() as f64 / graph.paths.len() as f64
    };
    let bridge_file_ratio = if graph.paths.is_empty() {
        0.0
    } else {
        bridge_counts
            .values()
            .filter(|targets| targets.len() >= 3)
            .count() as f64
            / graph.paths.len() as f64
    };
    let community_count_bonus = if (3..=12).contains(&communities.communities.len()) {
        1.0
    } else {
        0.5
    };

    (communities.modularity * 0.4
        + interface_narrowness * 0.3
        + (1.0 - bridge_file_ratio) * 0.2
        + community_count_bonus * 0.1)
        .clamp(0.0, 1.0)
}

fn event_driven_score(
    graph: &FileGraph,
    metrics: &CodeCityBoundaryGraphMetrics,
    source: &CodeCitySourceGraph,
    boundary: &CodeCityBoundary,
    config: &CodeCityConfig,
) -> f64 {
    let fan_in = incoming_counts(graph);
    let articulation = articulation_component_increase(graph);
    let median_fan_in = median(&fan_in);
    let hubs = graph
        .paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            ((fan_in[index] as f64 > (median_fan_in * 5.0).max(1.0)) || articulation[index] >= 3)
                .then_some(path.clone())
        })
        .collect::<Vec<_>>();

    let hub_type_density_normalized = if hubs.is_empty() {
        0.0
    } else {
        let mut density = 0.0_f64;
        for hub in &hubs {
            let hub_artefacts = source
                .artefacts
                .iter()
                .filter(|artefact| artefact.path == *hub);
            let mut type_like = 0usize;
            let mut callable_like = 0usize;
            for artefact in hub_artefacts {
                match artefact.canonical_kind.as_deref() {
                    Some("type") | Some("interface") | Some("enum") | Some("module") => {
                        type_like += 1
                    }
                    Some("callable") | Some("function") | Some("method") => callable_like += 1,
                    _ => {}
                }
            }
            density += type_like as f64 / callable_like.max(1) as f64;
        }
        (density / hubs.len() as f64).clamp(0.0, 1.0)
    };

    let message_infra_signal = if hubs.is_empty() {
        0.0
    } else {
        let text = source
            .external_dependency_hints
            .iter()
            .filter(|hint| {
                hint.from_path.starts_with(boundary.root_path.as_str()) || boundary.root_path == "."
            })
            .map(|hint| {
                format!(
                    "{} {}",
                    hint.to_symbol_ref.as_deref().unwrap_or(""),
                    hint.metadata
                )
                .to_ascii_lowercase()
            })
            .collect::<Vec<_>>();
        let hits = text
            .iter()
            .filter(|entry| {
                config
                    .architecture
                    .message_infra_libraries
                    .iter()
                    .any(|needle| entry.contains(needle))
            })
            .count();
        (hits as f64 / text.len().max(1) as f64).clamp(0.0, 1.0)
    };

    let hub_centrality = (hubs.len() as f64 / graph.paths.len().max(1) as f64).clamp(0.0, 1.0);
    let mut score = (1.0 - metrics.direct_coupling_ratio) * 0.35
        + hub_type_density_normalized * 0.25
        + message_infra_signal * 0.25
        + hub_centrality * 0.15;
    if hubs.is_empty() {
        score = score.min(0.4);
    }
    score.clamp(0.0, 1.0)
}

fn pipe_and_filter_score(metrics: &CodeCityBoundaryGraphMetrics) -> f64 {
    let branching_component = (1.0 / metrics.branching_factor.max(1.0)).min(1.0);
    let mut score = branching_component * 0.4
        + metrics.chain_dominance * 0.35
        + (1.0 - metrics.clustering_coefficient) * 0.25;
    if metrics.node_count < 4 || metrics.longest_path_len < 3 {
        score *= 0.65;
    }
    if !(metrics.branching_factor < 1.5 && metrics.chain_dominance > 0.5) {
        score *= 0.6;
    }
    score.clamp(0.0, 1.0)
}

fn mud_score(metrics: &CodeCityBoundaryGraphMetrics) -> f64 {
    let density_normalized = (metrics.density / 0.30).min(1.0);
    let largest_scc_ratio = if metrics.node_count == 0 {
        0.0
    } else {
        metrics.largest_scc_size as f64 / metrics.node_count as f64
    };
    let mut score = (density_normalized * 0.25
        + largest_scc_ratio * 0.25
        + (1.0 - metrics.modularity.clamp(0.0, 1.0)) * 0.20
        + metrics.back_edge_ratio * 0.15
        + (1.0 - metrics.core_periphery_score) * 0.15)
        .clamp(0.0, 1.0);
    if metrics.node_count < 4 {
        score *= 0.35;
    }
    score.clamp(0.0, 1.0)
}

fn incoming_counts(graph: &FileGraph) -> Vec<usize> {
    let mut counts = vec![0usize; graph.paths.len()];
    for &(_, to) in &graph.edges {
        counts[to] += 1;
    }
    counts
}

fn median(values: &[usize]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) as f64 / 2.0
    } else {
        sorted[middle] as f64
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::classify_boundary_architecture;
    use crate::capability_packs::codecity::services::community_detection::detect_communities;
    use crate::capability_packs::codecity::services::config::CodeCityConfig;
    use crate::capability_packs::codecity::services::graph_metrics::{
        FileGraph, compute_boundary_metrics,
    };
    use crate::capability_packs::codecity::services::source_graph::CodeCitySourceGraph;
    use crate::capability_packs::codecity::types::{
        CodeCityArchitecturePattern, CodeCityBoundary, CodeCityBoundaryKind, CodeCityBoundarySource,
    };

    fn graph(paths: &[&str], edges: &[(usize, usize)]) -> FileGraph {
        FileGraph {
            paths: paths.iter().map(|path| (*path).to_string()).collect(),
            index_by_path: paths
                .iter()
                .enumerate()
                .map(|(index, path)| ((*path).to_string(), index))
                .collect::<BTreeMap<_, _>>(),
            edges: edges.to_vec(),
        }
    }

    fn boundary() -> CodeCityBoundary {
        CodeCityBoundary {
            id: "boundary:apps/api".to_string(),
            name: "api".to_string(),
            root_path: "apps/api".to_string(),
            kind: CodeCityBoundaryKind::Explicit,
            ecosystem: Some("node".to_string()),
            parent_boundary_id: None,
            source: CodeCityBoundarySource::Manifest,
            file_count: 3,
            artefact_count: 3,
            dependency_count: 2,
            entry_points: Vec::new(),
            shared_library: false,
            atomic: true,
            architecture: None,
            layout: None,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn layered_classifier_scores_clean_dag_high() {
        let graph = graph(&["a", "b", "c"], &[(0, 1), (1, 2)]);
        let communities = detect_communities(&graph, 12);
        let metrics = compute_boundary_metrics(&graph, &communities);
        let report = classify_boundary_architecture(
            &boundary(),
            &graph,
            &metrics,
            &communities,
            &CodeCitySourceGraph {
                project_path: None,
                files: Vec::new(),
                artefacts: Vec::new(),
                edges: Vec::new(),
                external_dependency_hints: Vec::new(),
                diagnostics: Vec::new(),
            },
            &CodeCityConfig::default(),
        );

        assert_eq!(report.primary_pattern, CodeCityArchitecturePattern::Layered);
    }
}
