use super::*;

pub(super) fn systems_from_repo_graph(
    repository: ArchitectureGraphRepositoryRef,
    graph: ArchitectureGraph,
    system_key_filter: Option<&str>,
) -> BTreeMap<String, ArchitectureSystem> {
    let nodes_by_id = graph
        .nodes
        .into_iter()
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let mut system_ids_by_container = BTreeMap::<String, Vec<String>>::new();
    let mut components_by_container = BTreeMap::<String, Vec<String>>::new();
    let mut deployments_by_container = BTreeMap::<String, Vec<String>>::new();
    let mut entry_points_by_container = BTreeMap::<String, Vec<String>>::new();
    for edge in graph.edges {
        match edge.kind {
            ArchitectureGraphEdgeKind::Contains => {
                let Some(from) = nodes_by_id.get(&edge.from_node_id) else {
                    continue;
                };
                let Some(to) = nodes_by_id.get(&edge.to_node_id) else {
                    continue;
                };
                match (from.kind, to.kind) {
                    (ArchitectureGraphNodeKind::System, ArchitectureGraphNodeKind::Container) => {
                        system_ids_by_container
                            .entry(to.id.clone())
                            .or_default()
                            .push(from.id.clone());
                    }
                    (
                        ArchitectureGraphNodeKind::Container,
                        ArchitectureGraphNodeKind::Component,
                    ) => {
                        components_by_container
                            .entry(from.id.clone())
                            .or_default()
                            .push(to.id.clone());
                    }
                    _ => {}
                }
            }
            ArchitectureGraphEdgeKind::Realises => {
                let Some(from) = nodes_by_id.get(&edge.from_node_id) else {
                    continue;
                };
                let Some(to) = nodes_by_id.get(&edge.to_node_id) else {
                    continue;
                };
                if from.kind == ArchitectureGraphNodeKind::DeploymentUnit
                    && to.kind == ArchitectureGraphNodeKind::Container
                {
                    deployments_by_container
                        .entry(to.id.clone())
                        .or_default()
                        .push(from.id.clone());
                }
            }
            ArchitectureGraphEdgeKind::Exposes => {
                let Some(from) = nodes_by_id.get(&edge.from_node_id) else {
                    continue;
                };
                let Some(to) = nodes_by_id.get(&edge.to_node_id) else {
                    continue;
                };
                if from.kind == ArchitectureGraphNodeKind::Container
                    && to.kind == ArchitectureGraphNodeKind::EntryPoint
                {
                    entry_points_by_container
                        .entry(from.id.clone())
                        .or_default()
                        .push(to.id.clone());
                }
            }
            _ => {}
        }
    }

    let mut systems = BTreeMap::<String, ArchitectureSystem>::new();
    for container in nodes_by_id
        .values()
        .filter(|node| node.kind == ArchitectureGraphNodeKind::Container)
    {
        let Some(system_ids) = system_ids_by_container.get(&container.id) else {
            continue;
        };
        for system_id in system_ids {
            let Some(system) = nodes_by_id.get(system_id) else {
                continue;
            };
            let system_key =
                property_string(system, "system_key").unwrap_or_else(|| system.id.clone());
            if system_key_filter.is_some_and(|filter| filter != system_key) {
                continue;
            }
            let architecture_container = ArchitectureContainer {
                id: container.id.clone(),
                key: property_string(container, "container_key"),
                kind: property_string(container, "container_kind"),
                label: container.label.clone(),
                repository: repository.clone(),
                node: container.clone(),
                components: collect_nodes(&nodes_by_id, components_by_container.get(&container.id)),
                deployment_units: collect_nodes(
                    &nodes_by_id,
                    deployments_by_container.get(&container.id),
                ),
                entry_points: collect_nodes(
                    &nodes_by_id,
                    entry_points_by_container.get(&container.id),
                ),
            };
            systems
                .entry(system_key.clone())
                .and_modify(|existing| {
                    merge_repository_refs(
                        &mut existing.repositories,
                        std::slice::from_ref(&repository),
                    );
                    if !existing
                        .containers
                        .iter()
                        .any(|existing| existing.id == architecture_container.id)
                    {
                        existing.containers.push(architecture_container.clone());
                    }
                    sort_containers(&mut existing.containers);
                })
                .or_insert_with(|| ArchitectureSystem {
                    id: system.id.clone(),
                    key: system_key,
                    label: system.label.clone(),
                    repositories: vec![repository.clone()],
                    containers: vec![architecture_container],
                    node: system.clone(),
                });
        }
    }
    systems
}

pub(super) fn collect_nodes(
    nodes_by_id: &BTreeMap<String, ArchitectureGraphNode>,
    ids: Option<&Vec<String>>,
) -> Vec<ArchitectureGraphNode> {
    let mut nodes = ids
        .into_iter()
        .flat_map(|ids| ids.iter())
        .filter_map(|id| nodes_by_id.get(id).cloned())
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.id.cmp(&right.id))
    });
    nodes
}

pub(super) fn property_string(node: &ArchitectureGraphNode, key: &str) -> Option<String> {
    node.properties
        .0
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn repository_ref(repository: &SelectedRepository) -> ArchitectureGraphRepositoryRef {
    ArchitectureGraphRepositoryRef {
        repo_id: repository.repo_id().to_string(),
        name: repository.name().to_string(),
        provider: repository.provider().to_string(),
        organization: repository.organization().to_string(),
    }
}

pub(super) fn merge_repository_refs(
    target: &mut Vec<ArchitectureGraphRepositoryRef>,
    source: &[ArchitectureGraphRepositoryRef],
) {
    for repository in source {
        if !target
            .iter()
            .any(|existing| existing.repo_id == repository.repo_id)
        {
            target.push(repository.clone());
        }
    }
    target.sort_by(|left, right| {
        left.repo_id
            .cmp(&right.repo_id)
            .then_with(|| left.name.cmp(&right.name))
    });
}

pub(super) fn sort_containers(containers: &mut [ArchitectureContainer]) {
    containers.sort_by(|left, right| {
        left.repository
            .repo_id
            .cmp(&right.repository.repo_id)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.id.cmp(&right.id))
    });
}

pub(super) fn is_missing_architecture_graph_table_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("architecture_graph_nodes_current")
        || message.contains("architecture_graph_edges_current")
        || message.contains("architecture_graph_assertions")
}
