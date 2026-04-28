use std::collections::{BTreeMap, BTreeSet};

use super::config::ImportanceConfig;
use super::source_graph::{CodeCitySourceEdge, CodeCitySourceFile};
use crate::capability_packs::codecity::types::CodeCityImportance;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileGraph {
    pub paths: Vec<String>,
    pub index_by_path: BTreeMap<String, usize>,
    pub edges: Vec<(usize, usize)>,
}

pub fn build_file_graph(files: &[CodeCitySourceFile], edges: &[CodeCitySourceEdge]) -> FileGraph {
    let mut paths = files
        .iter()
        .filter(|file| file.included)
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();

    let index_by_path = paths
        .iter()
        .enumerate()
        .map(|(index, path)| (path.clone(), index))
        .collect::<BTreeMap<_, _>>();

    let mut unique_edges = BTreeSet::new();
    for edge in edges {
        let Some(&from_index) = index_by_path.get(&edge.from_path) else {
            continue;
        };
        let Some(&to_index) = index_by_path.get(&edge.to_path) else {
            continue;
        };
        unique_edges.insert((from_index, to_index));
    }

    FileGraph {
        paths,
        index_by_path,
        edges: unique_edges.into_iter().collect(),
    }
}

pub fn compute_importance(
    graph: &FileGraph,
    config: &ImportanceConfig,
) -> BTreeMap<String, CodeCityImportance> {
    if graph.paths.is_empty() {
        return BTreeMap::new();
    }

    let blast_radius = blast_radius(graph);
    let weighted_fan_in = pagerank(graph, config.pagerank_damping, config.pagerank_threshold);
    let articulation = articulation_scores(graph);

    let blast_norm = minmax(
        &blast_radius
            .iter()
            .map(|value| *value as f64)
            .collect::<Vec<_>>(),
    );
    let fan_in_norm = minmax(&weighted_fan_in);
    let articulation_norm = minmax(&articulation);

    graph
        .paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let score = config.blast_radius_weight * blast_norm[index]
                + config.weighted_fan_in_weight * fan_in_norm[index]
                + config.articulation_score_weight * articulation_norm[index];
            (
                path.clone(),
                CodeCityImportance {
                    score: score.clamp(0.0, 1.0),
                    blast_radius: blast_radius[index],
                    weighted_fan_in: weighted_fan_in[index],
                    articulation_score: articulation[index],
                    normalized_blast_radius: blast_norm[index],
                    normalized_weighted_fan_in: fan_in_norm[index],
                    normalized_articulation_score: articulation_norm[index],
                },
            )
        })
        .collect()
}

pub(crate) fn minmax(values: &[f64]) -> Vec<f64> {
    if values.is_empty() {
        return Vec::new();
    }

    let sanitised = values
        .iter()
        .map(|value| if value.is_finite() { *value } else { 0.0 })
        .collect::<Vec<_>>();
    let min = sanitised.iter().copied().reduce(f64::min).unwrap_or(0.0);
    let max = sanitised.iter().copied().reduce(f64::max).unwrap_or(0.0);

    if (max - min).abs() < f64::EPSILON {
        return vec![0.0; values.len()];
    }

    sanitised
        .into_iter()
        .map(|value| ((value - min) / (max - min)).clamp(0.0, 1.0))
        .collect()
}

fn blast_radius(graph: &FileGraph) -> Vec<usize> {
    let reverse = reverse_adjacency(graph);
    let mut blast = vec![0usize; graph.paths.len()];

    for node in 0..graph.paths.len() {
        let mut visited = vec![false; graph.paths.len()];
        let mut stack = reverse[node].clone();
        let mut count = 0usize;

        while let Some(next) = stack.pop() {
            if visited[next] {
                continue;
            }
            visited[next] = true;
            count += 1;
            stack.extend(reverse[next].iter().copied());
        }

        blast[node] = count;
    }

    blast
}

fn pagerank(graph: &FileGraph, damping: f64, threshold: f64) -> Vec<f64> {
    let node_count = graph.paths.len();
    let adjacency = adjacency(graph);
    let out_degree = adjacency.iter().map(Vec::len).collect::<Vec<_>>();
    let mut rank = vec![1.0 / node_count as f64; node_count];

    for _ in 0..100 {
        let mut next = vec![(1.0 - damping) / node_count as f64; node_count];
        let dangling_sum = rank
            .iter()
            .enumerate()
            .filter_map(|(index, value)| (out_degree[index] == 0).then_some(*value))
            .sum::<f64>();

        for (index, targets) in adjacency.iter().enumerate() {
            if targets.is_empty() {
                continue;
            }
            let share = damping * rank[index] / targets.len() as f64;
            for &target in targets {
                next[target] += share;
            }
        }

        let dangling_share = damping * dangling_sum / node_count as f64;
        for value in &mut next {
            *value += dangling_share;
        }

        let delta = next
            .iter()
            .zip(rank.iter())
            .map(|(left, right)| (left - right).abs())
            .sum::<f64>();
        rank = next;
        if delta < threshold {
            break;
        }
    }

    rank
}

fn articulation_scores(graph: &FileGraph) -> Vec<f64> {
    let node_count = graph.paths.len();
    if node_count == 0 {
        return Vec::new();
    }

    let adjacency = undirected_adjacency(graph);
    let mut discovery = vec![None; node_count];
    let mut low = vec![0usize; node_count];
    let mut additional_components = vec![0usize; node_count];
    let mut time = 0usize;

    fn dfs(
        node: usize,
        parent: Option<usize>,
        adjacency: &[Vec<usize>],
        discovery: &mut [Option<usize>],
        low: &mut [usize],
        additional_components: &mut [usize],
        time: &mut usize,
    ) {
        discovery[node] = Some(*time);
        low[node] = *time;
        *time += 1;

        let mut child_count = 0usize;
        for &next in &adjacency[node] {
            if discovery[next].is_none() {
                child_count += 1;
                dfs(
                    next,
                    Some(node),
                    adjacency,
                    discovery,
                    low,
                    additional_components,
                    time,
                );
                low[node] = low[node].min(low[next]);
                if parent.is_some() && low[next] >= discovery[node].unwrap_or_default() {
                    additional_components[node] += 1;
                }
            } else if Some(next) != parent {
                low[node] = low[node].min(discovery[next].unwrap_or_default());
            }
        }

        if parent.is_none() && child_count > 1 {
            additional_components[node] = child_count - 1;
        }
    }

    for node in 0..node_count {
        if discovery[node].is_none() {
            dfs(
                node,
                None,
                &adjacency,
                &mut discovery,
                &mut low,
                &mut additional_components,
                &mut time,
            );
        }
    }

    additional_components
        .into_iter()
        .map(|count| count as f64 / node_count as f64)
        .collect()
}

fn adjacency(graph: &FileGraph) -> Vec<Vec<usize>> {
    let mut adjacency = vec![Vec::new(); graph.paths.len()];
    for &(from, to) in &graph.edges {
        adjacency[from].push(to);
    }
    for targets in &mut adjacency {
        targets.sort_unstable();
        targets.dedup();
    }
    adjacency
}

fn reverse_adjacency(graph: &FileGraph) -> Vec<Vec<usize>> {
    let mut reverse = vec![Vec::new(); graph.paths.len()];
    for &(from, to) in &graph.edges {
        reverse[to].push(from);
    }
    for targets in &mut reverse {
        targets.sort_unstable();
        targets.dedup();
    }
    reverse
}

fn undirected_adjacency(graph: &FileGraph) -> Vec<Vec<usize>> {
    let mut adjacency = vec![Vec::new(); graph.paths.len()];
    for &(from, to) in &graph.edges {
        adjacency[from].push(to);
        adjacency[to].push(from);
    }
    for targets in &mut adjacency {
        targets.sort_unstable();
        targets.dedup();
    }
    adjacency
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;
    use std::collections::BTreeMap;

    use super::{FileGraph, compute_importance, minmax};
    use crate::capability_packs::codecity::services::config::CodeCityConfig;

    fn file_graph(paths: &[&str], edges: &[(usize, usize)]) -> FileGraph {
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

    #[test]
    fn chain_graph_gives_higher_blast_radius_to_shared_targets() {
        let graph = file_graph(&["a", "b", "c"], &[(0, 1), (1, 2)]);
        let importance = compute_importance(&graph, &CodeCityConfig::default().importance);

        assert!(importance["c"].blast_radius > importance["b"].blast_radius);
        assert!(importance["b"].blast_radius > importance["a"].blast_radius);
    }

    #[test]
    fn star_graph_gives_central_target_the_highest_importance() {
        let graph = file_graph(&["a", "b", "c", "core"], &[(0, 3), (1, 3), (2, 3)]);
        let importance = compute_importance(&graph, &CodeCityConfig::default().importance);

        let mut ordered = importance.into_iter().collect::<Vec<_>>();
        ordered.sort_by(|left, right| {
            right
                .1
                .score
                .partial_cmp(&left.1.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });

        assert_eq!(ordered[0].0, "core");
    }

    #[test]
    fn disconnected_graph_does_not_panic_and_keeps_isolated_nodes_zeroed() {
        let graph = file_graph(&["a", "b", "c"], &[(0, 1)]);
        let importance = compute_importance(&graph, &CodeCityConfig::default().importance);

        assert_eq!(importance["c"].blast_radius, 0);
        assert!(importance["c"].score >= 0.0);
    }

    #[test]
    fn articulation_graph_only_marks_the_bridge_node() {
        let graph = file_graph(&["a", "b", "c"], &[(0, 1), (1, 2)]);
        let importance = compute_importance(&graph, &CodeCityConfig::default().importance);

        assert!(importance["b"].articulation_score > 0.0);
        assert_eq!(importance["a"].articulation_score, 0.0);
        assert_eq!(importance["c"].articulation_score, 0.0);
    }

    #[test]
    fn minmax_returns_zeroes_for_equal_values() {
        assert_eq!(minmax(&[2.0, 2.0, 2.0]), vec![0.0, 0.0, 0.0]);
    }
}
