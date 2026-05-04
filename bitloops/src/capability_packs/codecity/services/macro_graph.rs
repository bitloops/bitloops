use std::collections::{BTreeMap, BTreeSet};

use super::community_detection::detect_communities;
use super::config::CodeCityConfig;
use super::graph_metrics::{
    FileGraph, collapse_strongly_connected_components, strongly_connected_components,
    topological_sort,
};
use super::source_graph::CodeCitySourceGraph;
use crate::capability_packs::codecity::types::{
    CodeCityBoundary, CodeCityMacroEdge, CodeCityMacroGraph, CodeCityMacroTopology,
};

pub fn build_macro_graph(
    source: &CodeCitySourceGraph,
    boundaries: &[CodeCityBoundary],
    file_to_boundary: &BTreeMap<String, String>,
) -> CodeCityMacroGraph {
    let mut counts = BTreeMap::<(String, String), usize>::new();
    let boundary_ids = boundaries
        .iter()
        .map(|boundary| boundary.id.clone())
        .collect::<Vec<_>>();

    for edge in &source.edges {
        let Some(from_boundary) = file_to_boundary.get(&edge.from_path) else {
            continue;
        };
        let Some(to_boundary) = file_to_boundary.get(&edge.to_path) else {
            continue;
        };
        if from_boundary == to_boundary {
            continue;
        }
        *counts
            .entry((from_boundary.clone(), to_boundary.clone()))
            .or_insert(0) += 1;
    }

    let edges = counts
        .into_iter()
        .map(
            |((from_boundary_id, to_boundary_id), weight)| CodeCityMacroEdge {
                file_edge_count: weight,
                weight,
                from_boundary_id,
                to_boundary_id,
            },
        )
        .collect::<Vec<_>>();

    let edge_count = edges.len();
    let boundary_count = boundaries.len();
    let density = if boundary_count <= 1 {
        0.0
    } else {
        edge_count as f64 / (boundary_count * (boundary_count - 1)) as f64
    };

    let modularity = boundary_graph(&boundary_ids, &edges)
        .as_ref()
        .map(|graph| detect_communities(graph, 12).modularity);
    let topology = classify_topology(&boundary_ids, &edges, density, modularity);

    CodeCityMacroGraph {
        topology,
        boundary_count,
        edge_count,
        density,
        modularity,
        edges,
    }
}

pub fn macro_fan_in_out(
    macro_graph: &CodeCityMacroGraph,
) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let mut fan_in = BTreeMap::<String, usize>::new();
    let mut fan_out = BTreeMap::<String, usize>::new();

    for edge in &macro_graph.edges {
        *fan_out.entry(edge.from_boundary_id.clone()).or_insert(0) += edge.weight;
        *fan_in.entry(edge.to_boundary_id.clone()).or_insert(0) += edge.weight;
        fan_in.entry(edge.from_boundary_id.clone()).or_insert(0);
        fan_out.entry(edge.to_boundary_id.clone()).or_insert(0);
    }

    (fan_in, fan_out)
}

pub fn apply_shared_library_flags(
    boundaries: &mut [CodeCityBoundary],
    macro_graph: &CodeCityMacroGraph,
    config: &CodeCityConfig,
) {
    let (fan_in, fan_out) = macro_fan_in_out(macro_graph);
    let fan_in_values = boundaries
        .iter()
        .map(|boundary| *fan_in.get(&boundary.id).unwrap_or(&0))
        .collect::<Vec<_>>();
    let fan_out_values = boundaries
        .iter()
        .map(|boundary| *fan_out.get(&boundary.id).unwrap_or(&0))
        .collect::<Vec<_>>();

    let fan_in_cutoff = percentile(
        &fan_in_values,
        config.boundaries.shared_library_fan_in_percentile,
    );
    let fan_out_cutoff = percentile(
        &fan_out_values,
        config.boundaries.shared_library_fan_out_percentile,
    );

    for boundary in boundaries {
        let in_degree = *fan_in.get(&boundary.id).unwrap_or(&0);
        let out_degree = *fan_out.get(&boundary.id).unwrap_or(&0);
        boundary.shared_library = macro_graph.boundary_count > 1
            && in_degree > 0
            && in_degree as f64 >= fan_in_cutoff
            && out_degree as f64 <= fan_out_cutoff
            && boundary.entry_points.is_empty();
    }
}

fn classify_topology(
    boundary_ids: &[String],
    edges: &[CodeCityMacroEdge],
    density: f64,
    modularity: Option<f64>,
) -> CodeCityMacroTopology {
    if boundary_ids.len() <= 1 {
        return CodeCityMacroTopology::SingleBoundary;
    }

    let Some(graph) = boundary_graph(boundary_ids, edges) else {
        return CodeCityMacroTopology::Unknown;
    };
    let sccs = strongly_connected_components(&graph);
    if sccs.iter().any(|component| component.len() > 2) || density > 0.3 {
        return CodeCityMacroTopology::Tangled;
    }

    let (fan_in, _) = macro_fan_in_out(&CodeCityMacroGraph {
        topology: CodeCityMacroTopology::Unknown,
        boundary_count: boundary_ids.len(),
        edge_count: edges.len(),
        density,
        modularity,
        edges: edges.to_vec(),
    });
    let mut fan_in_values = fan_in.values().copied().collect::<Vec<_>>();
    fan_in_values.sort_unstable();
    let median_fan_in = if fan_in_values.is_empty() {
        0.0
    } else if fan_in_values.len() % 2 == 0 {
        (fan_in_values[fan_in_values.len() / 2 - 1] + fan_in_values[fan_in_values.len() / 2]) as f64
            / 2.0
    } else {
        fan_in_values[fan_in_values.len() / 2] as f64
    };
    if let Some((central_boundary, max_fan_in)) = fan_in
        .iter()
        .max_by_key(|(_, degree)| *degree)
        .map(|(id, degree)| (id, *degree))
    {
        let dependents = edges
            .iter()
            .filter(|edge| edge.to_boundary_id == *central_boundary)
            .map(|edge| edge.from_boundary_id.clone())
            .collect::<BTreeSet<_>>();
        if max_fan_in as f64 > median_fan_in * 3.0
            && dependents.len() as f64 / boundary_ids.len() as f64 > 0.6
        {
            return CodeCityMacroTopology::Star;
        }
    }

    let collapsed = collapse_strongly_connected_components(&graph, &sccs);
    if topological_sort(collapsed.components.len(), &collapsed.edges).is_some() {
        return CodeCityMacroTopology::Layered;
    }
    if density < 0.15 && modularity.unwrap_or(0.0) > 0.5 {
        return CodeCityMacroTopology::Federated;
    }

    CodeCityMacroTopology::Unknown
}

fn boundary_graph(boundary_ids: &[String], edges: &[CodeCityMacroEdge]) -> Option<FileGraph> {
    if boundary_ids.is_empty() {
        return None;
    }

    let paths = boundary_ids.to_vec();
    let index_by_path = paths
        .iter()
        .enumerate()
        .map(|(index, path)| (path.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let graph_edges = edges
        .iter()
        .filter_map(|edge| {
            Some((
                *index_by_path.get(&edge.from_boundary_id)?,
                *index_by_path.get(&edge.to_boundary_id)?,
            ))
        })
        .collect::<Vec<_>>();

    Some(FileGraph {
        paths,
        index_by_path,
        edges: graph_edges,
    })
}

fn percentile(values: &[usize], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let percentile = percentile.clamp(0.0, 100.0);
    let index = ((percentile / 100.0) * (sorted.len().saturating_sub(1)) as f64).round() as usize;
    sorted[index] as f64
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::build_macro_graph;
    use crate::capability_packs::codecity::services::source_graph::{
        CodeCitySourceEdge, CodeCitySourceGraph,
    };
    use crate::capability_packs::codecity::types::{
        CodeCityBoundary, CodeCityBoundaryKind, CodeCityBoundarySource, CodeCityMacroTopology,
    };

    fn boundary(id: &str) -> CodeCityBoundary {
        CodeCityBoundary {
            id: id.to_string(),
            name: id.to_string(),
            root_path: id.trim_start_matches("boundary:").to_string(),
            kind: CodeCityBoundaryKind::Explicit,
            ecosystem: Some("node".to_string()),
            parent_boundary_id: None,
            source: CodeCityBoundarySource::Manifest,
            file_count: 1,
            artefact_count: 1,
            dependency_count: 0,
            entry_points: Vec::new(),
            shared_library: false,
            atomic: true,
            architecture: None,
            layout: None,
            violation_summary: Default::default(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn classifies_single_boundary_topology() {
        let graph = CodeCitySourceGraph {
            project_path: None,
            files: Vec::new(),
            artefacts: Vec::new(),
            edges: Vec::new(),
            external_dependency_hints: Vec::new(),
            diagnostics: Vec::new(),
        };
        let macro_graph = build_macro_graph(&graph, &[boundary("boundary:root")], &BTreeMap::new());

        assert_eq!(macro_graph.topology, CodeCityMacroTopology::SingleBoundary);
    }

    #[test]
    fn builds_macro_edges_from_cross_boundary_dependencies() {
        let graph = CodeCitySourceGraph {
            project_path: None,
            files: Vec::new(),
            artefacts: Vec::new(),
            edges: vec![CodeCitySourceEdge {
                edge_id: "edge-1".to_string(),
                from_path: "apps/api/src/a.ts".to_string(),
                to_path: "libs/core/src/b.ts".to_string(),
                from_symbol_id: "a".to_string(),
                from_artefact_id: "artefact-a".to_string(),
                to_symbol_id: Some("b".to_string()),
                to_artefact_id: Some("artefact-b".to_string()),
                to_symbol_ref: Some("libs/core/src/b.ts::b".to_string()),
                edge_kind: "imports".to_string(),
                language: "typescript".to_string(),
                start_line: Some(1),
                end_line: Some(1),
                metadata: "{}".to_string(),
            }],
            external_dependency_hints: Vec::new(),
            diagnostics: Vec::new(),
        };
        let file_to_boundary = BTreeMap::from([
            (
                "apps/api/src/a.ts".to_string(),
                "boundary:apps/api".to_string(),
            ),
            (
                "libs/core/src/b.ts".to_string(),
                "boundary:libs/core".to_string(),
            ),
        ]);

        let macro_graph = build_macro_graph(
            &graph,
            &[
                boundary("boundary:apps/api"),
                boundary("boundary:libs/core"),
            ],
            &file_to_boundary,
        );

        assert_eq!(macro_graph.edges.len(), 1);
        assert_eq!(macro_graph.edges[0].weight, 1);
    }
}
