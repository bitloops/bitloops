use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::community_detection::CodeCityCommunityResult;
use super::config::ImportanceConfig;
use super::source_graph::{CodeCitySourceEdge, CodeCitySourceFile};
use crate::capability_packs::codecity::types::{CodeCityBoundaryGraphMetrics, CodeCityImportance};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileGraph {
    pub paths: Vec<String>,
    pub index_by_path: BTreeMap<String, usize>,
    pub edges: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollapsedGraph {
    pub component_by_node: Vec<usize>,
    pub components: Vec<Vec<usize>>,
    pub edges: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphLayering {
    pub component_order: Vec<usize>,
    pub layer_by_component: Vec<usize>,
    pub layer_count: usize,
    pub layer_clarity: f64,
}

pub fn build_file_graph(files: &[CodeCitySourceFile], edges: &[CodeCitySourceEdge]) -> FileGraph {
    let mut paths = files
        .iter()
        .filter(|file| file.included)
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();

    build_graph_from_paths(&paths, edges)
}

pub fn build_graph_from_paths(paths: &[String], edges: &[CodeCitySourceEdge]) -> FileGraph {
    let mut sorted_paths = paths.to_vec();
    sorted_paths.sort();
    sorted_paths.dedup();

    let index_by_path = sorted_paths
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
        paths: sorted_paths,
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

pub fn compute_boundary_metrics(
    graph: &FileGraph,
    communities: &CodeCityCommunityResult,
) -> CodeCityBoundaryGraphMetrics {
    if graph.paths.is_empty() {
        return CodeCityBoundaryGraphMetrics::default();
    }

    let node_count = graph.paths.len();
    let edge_count = graph.edges.len();
    let density = if node_count <= 1 {
        0.0
    } else {
        edge_count as f64 / (node_count * (node_count - 1)) as f64
    };

    let sccs = strongly_connected_components(graph);
    let collapsed = collapse_strongly_connected_components(graph, &sccs);
    let layer_info = infer_graph_layering(graph, &sccs, &collapsed);
    let back_edge_ratio = back_edge_ratio(graph, &sccs, &collapsed, &layer_info.component_order);
    let cycle_edge_count = graph
        .edges
        .iter()
        .filter(|&&(from, to)| {
            let component = collapsed.component_by_node[from];
            component == collapsed.component_by_node[to]
                && collapsed.components[component].len() > 1
        })
        .count();
    let largest_scc_size = sccs.iter().map(Vec::len).max().unwrap_or(0);
    let (fan_in, fan_out) = fan_in_out(graph);
    let max_fan_in = fan_in.iter().copied().max().unwrap_or(0);
    let max_fan_out = fan_out.iter().copied().max().unwrap_or(0);
    let median_fan_in = median_usize(&fan_in);
    let median_fan_out = median_usize(&fan_out);
    let average_path_length = average_path_length(graph);
    let clustering_coefficient = clustering_coefficient(graph);
    let core_periphery_score = core_periphery_score(graph, &fan_in, &fan_out);
    let direct_coupling_ratio = direct_coupling_ratio(graph, &fan_in);
    let branching_factor = branching_factor(graph);
    let longest_path_len = longest_path_length(&collapsed, &layer_info.component_order);
    let chain_dominance = chain_dominance(graph, &collapsed, &layer_info.component_order);

    CodeCityBoundaryGraphMetrics {
        node_count,
        edge_count,
        density,
        cycle_edge_count,
        largest_scc_size,
        scc_count: sccs.len(),
        back_edge_ratio,
        modularity: communities.modularity,
        community_count: communities.communities.len().max(1),
        max_fan_in,
        median_fan_in,
        max_fan_out,
        median_fan_out,
        average_path_length,
        clustering_coefficient,
        core_periphery_score,
        direct_coupling_ratio,
        branching_factor,
        longest_path_len,
        chain_dominance,
    }
}

pub fn compute_depth_scores(graph: &FileGraph) -> BTreeMap<String, f64> {
    let (fan_in, fan_out) = fan_in_out(graph);
    graph
        .paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let depth = fan_in[index] as f64 / (fan_in[index] + fan_out[index] + 1) as f64;
            (path.clone(), depth.clamp(0.0, 1.0))
        })
        .collect()
}

pub fn articulation_component_increase(graph: &FileGraph) -> Vec<usize> {
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
}

pub fn strongly_connected_components(graph: &FileGraph) -> Vec<Vec<usize>> {
    struct TarjanState {
        index: usize,
        indices: Vec<Option<usize>>,
        lowlink: Vec<usize>,
        stack: Vec<usize>,
        on_stack: Vec<bool>,
        components: Vec<Vec<usize>>,
    }

    fn strong_connect(node: usize, adjacency: &[Vec<usize>], state: &mut TarjanState) {
        state.indices[node] = Some(state.index);
        state.lowlink[node] = state.index;
        state.index += 1;
        state.stack.push(node);
        state.on_stack[node] = true;

        for &next in &adjacency[node] {
            if state.indices[next].is_none() {
                strong_connect(next, adjacency, state);
                state.lowlink[node] = state.lowlink[node].min(state.lowlink[next]);
            } else if state.on_stack[next] {
                state.lowlink[node] =
                    state.lowlink[node].min(state.indices[next].unwrap_or_default());
            }
        }

        if state.lowlink[node] == state.indices[node].unwrap_or_default() {
            let mut component = Vec::new();
            while let Some(next) = state.stack.pop() {
                state.on_stack[next] = false;
                component.push(next);
                if next == node {
                    break;
                }
            }
            component.sort_unstable();
            state.components.push(component);
        }
    }

    let adjacency = adjacency(graph);
    let mut state = TarjanState {
        index: 0,
        indices: vec![None; graph.paths.len()],
        lowlink: vec![0usize; graph.paths.len()],
        stack: Vec::new(),
        on_stack: vec![false; graph.paths.len()],
        components: Vec::new(),
    };

    for node in 0..graph.paths.len() {
        if state.indices[node].is_none() {
            strong_connect(node, &adjacency, &mut state);
        }
    }

    state.components.sort_by(|left, right| {
        graph.paths[left[0]]
            .cmp(&graph.paths[right[0]])
            .then_with(|| left.len().cmp(&right.len()))
    });
    state.components
}

pub fn collapse_strongly_connected_components(
    graph: &FileGraph,
    sccs: &[Vec<usize>],
) -> CollapsedGraph {
    let mut component_by_node = vec![0usize; graph.paths.len()];
    for (component_index, component) in sccs.iter().enumerate() {
        for &node in component {
            component_by_node[node] = component_index;
        }
    }

    let mut component_edges = BTreeSet::new();
    for &(from, to) in &graph.edges {
        let left = component_by_node[from];
        let right = component_by_node[to];
        if left != right {
            component_edges.insert((left, right));
        }
    }

    CollapsedGraph {
        component_by_node,
        components: sccs.to_vec(),
        edges: component_edges.into_iter().collect(),
    }
}

pub fn infer_graph_layering(
    graph: &FileGraph,
    sccs: &[Vec<usize>],
    collapsed: &CollapsedGraph,
) -> GraphLayering {
    let order = topological_sort(collapsed.components.len(), &collapsed.edges)
        .unwrap_or_else(|| (0..collapsed.components.len()).collect());
    let position = order
        .iter()
        .enumerate()
        .map(|(index, component)| (*component, index))
        .collect::<BTreeMap<_, _>>();

    let mut layer_by_component = vec![0usize; collapsed.components.len()];
    for &component in &order {
        let current = layer_by_component[component];
        for &(from, to) in &collapsed.edges {
            if from == component {
                layer_by_component[to] = layer_by_component[to].max(current + 1);
            }
        }
    }

    let layer_count = layer_by_component.iter().copied().max().unwrap_or(0) + 1;
    let mut inter_layer_edges = 0usize;
    let mut adjacent_edges = 0usize;
    for &(from, to) in &graph.edges {
        let from_component = collapsed.component_by_node[from];
        let to_component = collapsed.component_by_node[to];
        if from_component == to_component && sccs[from_component].len() > 1 {
            continue;
        }
        inter_layer_edges += 1;
        let left = layer_by_component[from_component];
        let right = layer_by_component[to_component];
        if left + 1 == right {
            adjacent_edges += 1;
        }
        let _ = position;
    }

    let layer_clarity = if inter_layer_edges == 0 {
        1.0
    } else {
        adjacent_edges as f64 / inter_layer_edges as f64
    };

    GraphLayering {
        component_order: order,
        layer_by_component,
        layer_count,
        layer_clarity: layer_clarity.clamp(0.0, 1.0),
    }
}

pub fn topological_sort(node_count: usize, edges: &[(usize, usize)]) -> Option<Vec<usize>> {
    let mut indegree = vec![0usize; node_count];
    let mut adjacency = vec![Vec::new(); node_count];
    for &(from, to) in edges {
        indegree[to] += 1;
        adjacency[from].push(to);
    }
    for targets in &mut adjacency {
        targets.sort_unstable();
        targets.dedup();
    }

    let mut ready = indegree
        .iter()
        .enumerate()
        .filter_map(|(node, &count)| (count == 0).then_some(node))
        .collect::<Vec<_>>();
    ready.sort_unstable_by(|left, right| right.cmp(left));

    let mut order = Vec::with_capacity(node_count);
    while let Some(node) = ready.pop() {
        order.push(node);
        for &next in &adjacency[node] {
            indegree[next] = indegree[next].saturating_sub(1);
            if indegree[next] == 0 {
                ready.push(next);
                ready.sort_unstable_by(|left, right| right.cmp(left));
            }
        }
    }

    (order.len() == node_count).then_some(order)
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

    articulation_component_increase(graph)
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

fn fan_in_out(graph: &FileGraph) -> (Vec<usize>, Vec<usize>) {
    let mut fan_in = vec![0usize; graph.paths.len()];
    let mut fan_out = vec![0usize; graph.paths.len()];
    for &(from, to) in &graph.edges {
        fan_out[from] += 1;
        fan_in[to] += 1;
    }
    (fan_in, fan_out)
}

fn back_edge_ratio(
    graph: &FileGraph,
    _sccs: &[Vec<usize>],
    collapsed: &CollapsedGraph,
    order: &[usize],
) -> f64 {
    if graph.edges.is_empty() {
        return 0.0;
    }

    let position = order
        .iter()
        .enumerate()
        .map(|(index, component)| (*component, index))
        .collect::<BTreeMap<_, _>>();

    let upward_or_cyclic = graph
        .edges
        .iter()
        .filter(|&&(from, to)| {
            let left = collapsed.component_by_node[from];
            let right = collapsed.component_by_node[to];
            if left == right {
                return collapsed.components[left].len() > 1;
            }
            position[&left] >= position[&right]
        })
        .count();

    upward_or_cyclic as f64 / graph.edges.len() as f64
}

fn average_path_length(graph: &FileGraph) -> Option<f64> {
    if graph.paths.is_empty() {
        return None;
    }

    let adjacency = adjacency(graph);
    let samples = if graph.paths.len() <= 1000 {
        (0..graph.paths.len()).collect::<Vec<_>>()
    } else {
        let mut sampled = BTreeSet::new();
        let degree_order = adjacency
            .iter()
            .enumerate()
            .map(|(index, targets)| (targets.len(), index))
            .collect::<Vec<_>>();
        for (_, index) in degree_order.into_iter().rev().take(50) {
            sampled.insert(index);
        }
        for index in 0..graph.paths.len().min(100) {
            sampled.insert(index);
        }
        let stride = (graph.paths.len() / 25).max(1);
        let mut current = 0usize;
        while current < graph.paths.len() {
            sampled.insert(current);
            current += stride;
        }
        sampled.into_iter().collect::<Vec<_>>()
    };

    let mut total = 0usize;
    let mut count = 0usize;
    for source in samples {
        let mut distance = vec![usize::MAX; graph.paths.len()];
        let mut queue = VecDeque::new();
        distance[source] = 0;
        queue.push_back(source);

        while let Some(node) = queue.pop_front() {
            for &next in &adjacency[node] {
                if distance[next] != usize::MAX {
                    continue;
                }
                distance[next] = distance[node] + 1;
                queue.push_back(next);
            }
        }

        for &value in &distance {
            if value != usize::MAX && value > 0 {
                total += value;
                count += 1;
            }
        }
    }

    (count > 0).then_some(total as f64 / count as f64)
}

fn clustering_coefficient(graph: &FileGraph) -> f64 {
    let adjacency = undirected_adjacency(graph);
    let mut total = 0.0_f64;

    for (node, neighbours) in adjacency.iter().enumerate() {
        let degree = neighbours.len();
        if degree < 2 {
            continue;
        }
        let mut links = 0usize;
        for left in 0..degree {
            for right in (left + 1)..degree {
                if adjacency[neighbours[left]]
                    .binary_search(&neighbours[right])
                    .is_ok()
                {
                    links += 1;
                }
            }
        }
        let possible = degree * (degree - 1) / 2;
        total += links as f64 / possible as f64;
        let _ = node;
    }

    total / graph.paths.len() as f64
}

fn core_periphery_score(graph: &FileGraph, fan_in: &[usize], fan_out: &[usize]) -> f64 {
    if graph.edges.is_empty() {
        return 0.0;
    }

    let depth = graph
        .paths
        .iter()
        .enumerate()
        .map(|(index, _)| fan_in[index] as f64 / (fan_in[index] + fan_out[index] + 1) as f64)
        .collect::<Vec<_>>();
    let core = depth
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (*value > 0.7).then_some(index))
        .collect::<BTreeSet<_>>();
    let periphery = depth
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (*value < 0.3).then_some(index))
        .collect::<BTreeSet<_>>();

    let inward_edge_ratio = graph
        .edges
        .iter()
        .filter(|&&(from, to)| depth[from] < depth[to])
        .count() as f64
        / graph.edges.len() as f64;

    let core_density = subset_density(graph, &core);
    let periphery_density = subset_density(graph, &periphery);
    let core_density_advantage = (core_density - periphery_density).clamp(0.0, 1.0);

    (inward_edge_ratio * 0.6 + core_density_advantage * 0.4).clamp(0.0, 1.0)
}

fn direct_coupling_ratio(graph: &FileGraph, fan_in: &[usize]) -> f64 {
    if graph.edges.is_empty() {
        return 0.0;
    }

    let median = median_usize(fan_in);
    let hub_threshold = (median * 5.0).max(1.0);
    let hubs = fan_in
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (*value as f64 > hub_threshold).then_some(index))
        .collect::<BTreeSet<_>>();
    if hubs.is_empty() {
        return 1.0;
    }

    graph
        .edges
        .iter()
        .filter(|&&(from, to)| !hubs.contains(&from) && !hubs.contains(&to))
        .count() as f64
        / graph.edges.len() as f64
}

fn branching_factor(graph: &FileGraph) -> f64 {
    let adjacency = adjacency(graph);
    let non_leaf = adjacency
        .iter()
        .filter_map(|targets| (!targets.is_empty()).then_some(targets.len()))
        .collect::<Vec<_>>();
    if non_leaf.is_empty() {
        return 0.0;
    }
    non_leaf.iter().sum::<usize>() as f64 / non_leaf.len() as f64
}

fn longest_path_length(collapsed: &CollapsedGraph, order: &[usize]) -> usize {
    let mut distance = vec![0usize; collapsed.components.len()];
    for &node in order {
        for &(from, to) in &collapsed.edges {
            if from == node {
                distance[to] = distance[to].max(distance[from] + 1);
            }
        }
    }
    distance.into_iter().max().unwrap_or(0)
}

fn chain_dominance(graph: &FileGraph, collapsed: &CollapsedGraph, order: &[usize]) -> f64 {
    if graph.paths.is_empty() {
        return 0.0;
    }

    let mut distance = vec![0usize; collapsed.components.len()];
    let mut previous = vec![None; collapsed.components.len()];
    for &node in order {
        for &(from, to) in &collapsed.edges {
            if from == node && distance[from] + 1 > distance[to] {
                distance[to] = distance[from] + 1;
                previous[to] = Some(from);
            }
        }
    }

    let Some((mut current, _)) = distance.iter().enumerate().max_by_key(|(_, value)| *value) else {
        return 0.0;
    };

    let mut path_components = BTreeSet::new();
    path_components.insert(current);
    while let Some(prev) = previous[current] {
        current = prev;
        path_components.insert(current);
    }

    let path_nodes = path_components
        .into_iter()
        .map(|component| collapsed.components[component].len())
        .sum::<usize>();
    path_nodes as f64 / graph.paths.len() as f64
}

fn subset_density(graph: &FileGraph, nodes: &BTreeSet<usize>) -> f64 {
    if nodes.len() <= 1 {
        return 0.0;
    }

    let edges = graph
        .edges
        .iter()
        .filter(|&&(from, to)| nodes.contains(&from) && nodes.contains(&to))
        .count();
    edges as f64 / (nodes.len() * (nodes.len() - 1)) as f64
}

fn median_usize(values: &[usize]) -> f64 {
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
    use std::cmp::Ordering;
    use std::collections::BTreeMap;

    use super::{
        FileGraph, articulation_component_increase, compute_boundary_metrics, compute_importance,
        minmax, strongly_connected_components,
    };
    use crate::capability_packs::codecity::services::community_detection::detect_communities;
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
        let articulation = articulation_component_increase(&graph);

        assert!(articulation[1] > 0);
        assert_eq!(articulation[0], 0);
        assert_eq!(articulation[2], 0);
    }

    #[test]
    fn minmax_returns_zeroes_for_equal_values() {
        assert_eq!(minmax(&[2.0, 2.0, 2.0]), vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn tarjan_detects_sccs() {
        let graph = file_graph(&["a", "b", "c", "d"], &[(0, 1), (1, 0), (1, 2), (2, 3)]);
        let sccs = strongly_connected_components(&graph);

        assert_eq!(sccs.len(), 3);
        assert_eq!(sccs[0], vec![0, 1]);
    }

    #[test]
    fn boundary_metrics_report_non_zero_back_edge_ratio_for_cycles() {
        let graph = file_graph(&["a", "b", "c"], &[(0, 1), (1, 2), (2, 0)]);
        let communities = detect_communities(&graph, 8);
        let metrics = compute_boundary_metrics(&graph, &communities);

        assert!(metrics.back_edge_ratio > 0.0);
        assert_eq!(metrics.largest_scc_size, 3);
    }
}
