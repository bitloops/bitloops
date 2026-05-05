use std::collections::{BTreeMap, BTreeSet};

use super::graph_metrics::FileGraph;

const MAX_COMMUNITY_DETECTION_NODES: usize = 512;
const MAX_COMMUNITY_DETECTION_EDGES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeCityCommunity {
    pub id: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodeCityCommunityResult {
    pub modularity: f64,
    pub communities: Vec<CodeCityCommunity>,
    pub community_by_path: BTreeMap<String, String>,
}

impl Default for CodeCityCommunityResult {
    fn default() -> Self {
        Self {
            modularity: 0.0,
            communities: Vec::new(),
            community_by_path: BTreeMap::new(),
        }
    }
}

pub fn detect_communities(graph: &FileGraph, max_iterations: usize) -> CodeCityCommunityResult {
    if graph.paths.is_empty() {
        return CodeCityCommunityResult::default();
    }

    if graph.paths.len() == 1 {
        let path = graph.paths[0].clone();
        return CodeCityCommunityResult {
            modularity: 0.0,
            communities: vec![CodeCityCommunity {
                id: "community_1".to_string(),
                paths: vec![path.clone()],
            }],
            community_by_path: BTreeMap::from([(path, "community_1".to_string())]),
        };
    }

    if graph.edges.is_empty() {
        return single_community_result(graph);
    }

    if graph.paths.len() > MAX_COMMUNITY_DETECTION_NODES
        || graph.edges.len() > MAX_COMMUNITY_DETECTION_EDGES
    {
        return single_community_result(graph);
    }

    let undirected = undirected_weights(graph);
    let mut assignment = (0..graph.paths.len()).collect::<Vec<_>>();
    let mut current_modularity = modularity_for_assignment(graph, &undirected, &assignment);

    for _ in 0..max_iterations {
        let mut changed = false;

        for node in 0..graph.paths.len() {
            let current = assignment[node];
            let mut candidates = neighbour_communities(node, &undirected, &assignment);
            candidates.insert(current);

            let mut best_assignment = assignment.clone();
            let mut best_modularity = current_modularity;
            let mut best_key = community_key(graph, &assignment, current);

            for candidate in candidates {
                if candidate == current {
                    continue;
                }

                let mut candidate_assignment = assignment.clone();
                candidate_assignment[node] = candidate;
                normalise_assignment(graph, &mut candidate_assignment);

                let modularity =
                    modularity_for_assignment(graph, &undirected, &candidate_assignment);
                let key = community_key(graph, &candidate_assignment, candidate_assignment[node]);

                if modularity > best_modularity + 1e-9
                    || ((modularity - best_modularity).abs() <= 1e-9 && key < best_key)
                {
                    best_assignment = candidate_assignment;
                    best_modularity = modularity;
                    best_key = key;
                }
            }

            if best_assignment != assignment {
                assignment = best_assignment;
                current_modularity = best_modularity;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    normalise_assignment(graph, &mut assignment);
    build_result(graph, &assignment, current_modularity)
}

fn single_community_result(graph: &FileGraph) -> CodeCityCommunityResult {
    let paths = graph.paths.clone();
    CodeCityCommunityResult {
        modularity: 0.0,
        communities: vec![CodeCityCommunity {
            id: "community_1".to_string(),
            paths: paths.clone(),
        }],
        community_by_path: paths
            .into_iter()
            .map(|path| (path, "community_1".to_string()))
            .collect(),
    }
}

fn build_result(
    graph: &FileGraph,
    assignment: &[usize],
    modularity: f64,
) -> CodeCityCommunityResult {
    let mut grouped = BTreeMap::<usize, Vec<String>>::new();
    for (index, path) in graph.paths.iter().enumerate() {
        grouped
            .entry(assignment[index])
            .or_default()
            .push(path.clone());
    }

    let mut communities = grouped
        .into_iter()
        .enumerate()
        .map(|(ordinal, (_, mut paths))| {
            paths.sort();
            CodeCityCommunity {
                id: format!("community_{}", ordinal + 1),
                paths,
            }
        })
        .collect::<Vec<_>>();

    communities.sort_by(|left, right| left.paths[0].cmp(&right.paths[0]));
    for (ordinal, community) in communities.iter_mut().enumerate() {
        community.id = format!("community_{}", ordinal + 1);
    }

    let mut community_by_path = BTreeMap::new();
    for community in &communities {
        for path in &community.paths {
            community_by_path.insert(path.clone(), community.id.clone());
        }
    }

    CodeCityCommunityResult {
        modularity: modularity.clamp(0.0, 1.0),
        communities,
        community_by_path,
    }
}

fn neighbour_communities(
    node: usize,
    undirected: &BTreeMap<(usize, usize), f64>,
    assignment: &[usize],
) -> BTreeSet<usize> {
    let mut communities = BTreeSet::new();
    for &(left, right) in undirected.keys() {
        if left == node {
            communities.insert(assignment[right]);
        } else if right == node {
            communities.insert(assignment[left]);
        }
    }
    communities
}

fn normalise_assignment(graph: &FileGraph, assignment: &mut [usize]) {
    let mut min_path_by_community = BTreeMap::<usize, &str>::new();
    for (index, community) in assignment.iter().copied().enumerate() {
        let path = graph.paths[index].as_str();
        min_path_by_community
            .entry(community)
            .and_modify(|current| {
                if path < *current {
                    *current = path;
                }
            })
            .or_insert(path);
    }

    let mut ordered = min_path_by_community
        .into_iter()
        .map(|(community, path)| (path.to_string(), community))
        .collect::<Vec<_>>();
    ordered.sort();

    let remap = ordered
        .into_iter()
        .enumerate()
        .map(|(index, (_, community))| (community, index))
        .collect::<BTreeMap<_, _>>();

    for label in assignment.iter_mut() {
        *label = remap[label];
    }
}

fn community_key(graph: &FileGraph, assignment: &[usize], community: usize) -> String {
    graph
        .paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| (assignment[index] == community).then_some(path.clone()))
        .min()
        .unwrap_or_else(|| format!("community_{community}"))
}

fn undirected_weights(graph: &FileGraph) -> BTreeMap<(usize, usize), f64> {
    let mut weights = BTreeMap::<(usize, usize), f64>::new();
    for &(from, to) in &graph.edges {
        let key = if from < to { (from, to) } else { (to, from) };
        *weights.entry(key).or_insert(0.0) += 1.0;
    }
    weights
}

fn modularity_for_assignment(
    graph: &FileGraph,
    undirected: &BTreeMap<(usize, usize), f64>,
    assignment: &[usize],
) -> f64 {
    let mut degree = vec![0.0_f64; graph.paths.len()];
    let mut total_weight = 0.0_f64;

    for (&(left, right), &weight) in undirected {
        degree[left] += weight;
        degree[right] += weight;
        total_weight += weight;
    }

    if total_weight <= f64::EPSILON {
        return 0.0;
    }

    let twice_m = total_weight * 2.0;
    let mut internal_adjacency_by_community = BTreeMap::<usize, f64>::new();
    for (&(left, right), &weight) in undirected {
        if assignment[left] == assignment[right] {
            *internal_adjacency_by_community
                .entry(assignment[left])
                .or_insert(0.0) += weight * 2.0;
        }
    }

    let mut degree_by_community = BTreeMap::<usize, f64>::new();
    for (index, value) in degree.iter().copied().enumerate() {
        *degree_by_community.entry(assignment[index]).or_insert(0.0) += value;
    }

    let modularity = degree_by_community
        .into_iter()
        .map(|(community, degree_sum)| {
            internal_adjacency_by_community
                .get(&community)
                .copied()
                .unwrap_or(0.0)
                - (degree_sum * degree_sum / twice_m)
        })
        .sum::<f64>();

    (modularity / twice_m).max(0.0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::detect_communities;
    use crate::capability_packs::codecity::services::graph_metrics::FileGraph;

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

    #[test]
    fn detects_two_dense_clusters() {
        let graph = graph(
            &["a", "b", "c", "d", "e", "f"],
            &[
                (0, 1),
                (1, 0),
                (1, 2),
                (2, 1),
                (3, 4),
                (4, 3),
                (4, 5),
                (5, 4),
                (2, 3),
            ],
        );

        let result = detect_communities(&graph, 12);

        assert_eq!(result.communities.len(), 2);
        assert!(result.modularity > 0.2);
    }

    #[test]
    fn keeps_single_cluster_for_chain_graph() {
        let graph = graph(&["a", "b", "c"], &[(0, 1), (1, 2)]);

        let result = detect_communities(&graph, 12);

        assert_eq!(result.communities.len(), 1);
    }

    #[test]
    fn keeps_single_cluster_for_graph_without_edges() {
        let graph = graph(&["a", "b", "c"], &[]);

        let result = detect_communities(&graph, 12);

        assert_eq!(result.communities.len(), 1);
        assert_eq!(result.communities[0].paths, vec!["a", "b", "c"]);
        assert_eq!(result.community_by_path["a"], "community_1");
        assert_eq!(result.modularity, 0.0);
    }

    #[test]
    fn skips_expensive_detection_for_large_interactive_graphs() {
        let paths = (0..=super::MAX_COMMUNITY_DETECTION_NODES)
            .map(|index| format!("file_{index}.rs"))
            .collect::<Vec<_>>();
        let path_refs = paths.iter().map(String::as_str).collect::<Vec<_>>();
        let edges = (0..super::MAX_COMMUNITY_DETECTION_NODES)
            .map(|index| (index, index + 1))
            .collect::<Vec<_>>();
        let graph = graph(&path_refs, &edges);

        let result = detect_communities(&graph, 12);

        assert_eq!(result.communities.len(), 1);
        assert_eq!(result.communities[0].paths.len(), path_refs.len());
        assert_eq!(result.community_by_path["file_0.rs"], "community_1");
        assert_eq!(result.modularity, 0.0);
    }
}
