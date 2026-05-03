use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{Value, json};

use crate::host::capability_host::{
    CurrentStateConsumer, CurrentStateConsumerContext, CurrentStateConsumerFuture,
    CurrentStateConsumerRequest, CurrentStateConsumerResult,
};
use crate::host::language_adapter::{
    LanguageEntryPointArtefact, LanguageEntryPointCandidate, LanguageEntryPointFile,
};
use crate::models::{
    CurrentCanonicalArtefactRecord, CurrentCanonicalEdgeRecord, CurrentCanonicalFileRecord,
};

use super::storage::{
    ArchitectureGraphEdgeFact, ArchitectureGraphFacts, ArchitectureGraphNodeFact,
    component_node_id, container_node_id, deployment_unit_node_id, edge_id, node_id,
    replace_computed_graph, system_node_id,
};
use super::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_CONSUMER_ID, ArchitectureGraphEdgeKind,
    ArchitectureGraphNodeKind,
};

pub struct ArchitectureGraphCurrentStateConsumer;

impl CurrentStateConsumer for ArchitectureGraphCurrentStateConsumer {
    fn capability_id(&self) -> &str {
        ARCHITECTURE_GRAPH_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        ARCHITECTURE_GRAPH_CONSUMER_ID
    }

    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a> {
        Box::pin(async move {
            let files = context
                .relational
                .load_current_canonical_files(&request.repo_id)
                .context("loading current files for architecture graph")?;
            let artefacts = context
                .relational
                .load_current_canonical_artefacts(&request.repo_id)
                .context("loading current artefacts for architecture graph")?;
            let dependency_edges = context
                .relational
                .load_current_canonical_edges(&request.repo_id)
                .context("loading current dependency edges for architecture graph")?;

            let mut warnings = Vec::new();
            let mut builder = GraphBuilder::new(
                &request.repo_id,
                request.to_generation_seq_inclusive,
                request
                    .run_id
                    .as_deref()
                    .unwrap_or(ARCHITECTURE_GRAPH_CONSUMER_ID),
            );
            builder.seed_repo_structure();
            builder.add_code_nodes(&artefacts);
            builder.add_dependency_edges(&dependency_edges);
            builder.add_entry_points_and_flows(
                context,
                &request.repo_root,
                &files,
                &artefacts,
                &dependency_edges,
            );
            builder.add_components_for_containers(&artefacts);
            add_test_harness_facts(context, &mut builder, &mut warnings).await;
            builder.add_change_unit(request);

            let facts = builder.finish();
            let metrics = json!({
                "nodes": facts.nodes.len(),
                "edges": facts.edges.len(),
                "files": files.len(),
                "artefacts": artefacts.len(),
                "dependency_edges": dependency_edges.len(),
                "reconcile_mode": format!("{:?}", request.reconcile_mode),
            });
            replace_computed_graph(
                &context.storage,
                &request.repo_id,
                facts,
                request.to_generation_seq_inclusive,
                &warnings,
                metrics.clone(),
            )
            .await?;

            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings,
                metrics: Some(metrics),
            })
        })
    }
}

struct GraphBuilder {
    repo_id: String,
    generation: u64,
    run_id: String,
    nodes: BTreeMap<String, ArchitectureGraphNodeFact>,
    edges: BTreeMap<String, ArchitectureGraphEdgeFact>,
    artefact_nodes: BTreeMap<String, String>,
    symbol_nodes: BTreeMap<String, String>,
    path_nodes: BTreeMap<String, Vec<String>>,
    container_bindings: Vec<DeploymentBinding>,
}

#[derive(Debug, Clone)]
struct DeploymentBinding {
    container_id: String,
    container_root: String,
}

impl GraphBuilder {
    fn new(repo_id: &str, generation: u64, run_id: &str) -> Self {
        Self {
            repo_id: repo_id.to_string(),
            generation,
            run_id: run_id.to_string(),
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
            artefact_nodes: BTreeMap::new(),
            symbol_nodes: BTreeMap::new(),
            path_nodes: BTreeMap::new(),
            container_bindings: Vec::new(),
        }
    }

    fn finish(self) -> ArchitectureGraphFacts {
        ArchitectureGraphFacts {
            nodes: self.nodes.into_values().collect(),
            edges: self.edges.into_values().collect(),
        }
    }

    fn seed_repo_structure(&mut self) {
        let system_key = self.fallback_system_key();
        let system_id = self.fallback_system_id();

        self.upsert_node(ArchitectureGraphNodeFact {
            repo_id: self.repo_id.clone(),
            node_id: system_id.clone(),
            node_kind: ArchitectureGraphNodeKind::System.as_str().to_string(),
            label: "Repository system".to_string(),
            artefact_id: None,
            symbol_id: None,
            path: None,
            entry_kind: None,
            source_kind: "COMPUTED".to_string(),
            confidence: 1.0,
            provenance: self.provenance("repo_context"),
            evidence: json!([]),
            properties: json!({
                "repo_id": &self.repo_id,
                "system_key": system_key,
            }),
            last_observed_generation: Some(self.generation),
        });
    }

    fn add_code_nodes(&mut self, artefacts: &[CurrentCanonicalArtefactRecord]) {
        for artefact in artefacts {
            let code_node_id = node_id(
                &self.repo_id,
                ArchitectureGraphNodeKind::Node,
                &artefact.artefact_id,
            );
            self.artefact_nodes
                .insert(artefact.artefact_id.clone(), code_node_id.clone());
            self.symbol_nodes
                .insert(artefact.symbol_id.clone(), code_node_id.clone());
            self.path_nodes
                .entry(artefact.path.clone())
                .or_default()
                .push(code_node_id.clone());
            let label = artefact_display_name(artefact);
            self.upsert_node(ArchitectureGraphNodeFact {
                repo_id: self.repo_id.clone(),
                node_id: code_node_id.clone(),
                node_kind: ArchitectureGraphNodeKind::Node.as_str().to_string(),
                label,
                artefact_id: Some(artefact.artefact_id.clone()),
                symbol_id: Some(artefact.symbol_id.clone()),
                path: Some(artefact.path.clone()),
                entry_kind: None,
                source_kind: "COMPUTED".to_string(),
                confidence: 1.0,
                provenance: self.provenance("devql_current_state"),
                evidence: json!([{
                    "path": &artefact.path,
                    "startLine": artefact.start_line,
                    "endLine": artefact.end_line,
                    "canonicalKind": &artefact.canonical_kind,
                }]),
                properties: json!({
                    "language": &artefact.language,
                    "canonical_kind": &artefact.canonical_kind,
                    "language_kind": &artefact.language_kind,
                    "symbol_fqn": &artefact.symbol_fqn,
                    "parent_artefact_id": &artefact.parent_artefact_id,
                    "signature": &artefact.signature,
                }),
                last_observed_generation: Some(self.generation),
            });
        }
    }

    fn add_dependency_edges(&mut self, dependency_edges: &[CurrentCanonicalEdgeRecord]) {
        for dependency in dependency_edges {
            let Some(from_node_id) = self
                .artefact_nodes
                .get(&dependency.from_artefact_id)
                .cloned()
            else {
                continue;
            };
            let to_node_id = dependency
                .to_artefact_id
                .as_ref()
                .and_then(|artefact_id| self.artefact_nodes.get(artefact_id))
                .or_else(|| {
                    dependency
                        .to_symbol_id
                        .as_ref()
                        .and_then(|symbol_id| self.symbol_nodes.get(symbol_id))
                })
                .cloned();
            let Some(to_node_id) = to_node_id else {
                continue;
            };
            self.upsert_edge_by_kind(
                ArchitectureGraphEdgeKind::DependsOn,
                from_node_id,
                to_node_id,
                "COMPUTED",
                0.90,
                self.provenance("devql_dependency_edge"),
                json!([{
                    "edgeId": &dependency.edge_id,
                    "path": &dependency.path,
                    "edgeKind": &dependency.edge_kind,
                    "startLine": dependency.start_line,
                    "endLine": dependency.end_line,
                }]),
                json!({
                    "language": &dependency.language,
                    "edge_kind": &dependency.edge_kind,
                    "to_symbol_ref": &dependency.to_symbol_ref,
                }),
            );
        }
    }

    fn add_entry_points_and_flows(
        &mut self,
        context: &CurrentStateConsumerContext,
        repo_root: &Path,
        files: &[CurrentCanonicalFileRecord],
        artefacts: &[CurrentCanonicalArtefactRecord],
        dependency_edges: &[CurrentCanonicalEdgeRecord],
    ) {
        let artefacts_by_path = group_entry_point_artefacts_by_path(artefacts);
        let adjacency = dependency_adjacency(dependency_edges);
        let config_candidates = detect_config_entry_points(repo_root, files, &artefacts_by_path);
        let mut deployment_by_path = BTreeMap::<String, DeploymentBinding>::new();
        for candidate in &config_candidates {
            let binding = self.ensure_deployment_container_for_candidate(candidate);
            deployment_by_path
                .entry(candidate.path.clone())
                .or_insert(binding);
        }

        for file in files {
            let file_artefacts = artefacts_by_path
                .get(&file.path)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let entry_file = LanguageEntryPointFile {
                path: file.path.clone(),
                language: file.resolved_language.clone(),
                content_id: file.effective_content_id.clone(),
            };
            let candidates = context
                .language_services
                .entry_point_candidates_for_file(&entry_file, file_artefacts);
            for candidate in candidates {
                let binding = deployment_by_path.get(&candidate.path);
                self.add_entry_point_candidate(
                    &candidate,
                    Some(file.resolved_language.as_str()),
                    "LANGUAGE_EVIDENCE",
                    "language_entry_point_support",
                    &adjacency,
                    binding,
                );
            }
        }

        for candidate in config_candidates {
            let language = files
                .iter()
                .find(|file| file.path == candidate.path)
                .map(|file| file.resolved_language.as_str());
            let binding = deployment_by_path.get(&candidate.path);
            self.add_entry_point_candidate(
                &candidate,
                language,
                "CONFIG_EVIDENCE",
                "config_entry_point_support",
                &adjacency,
                binding,
            );
        }
    }

    fn add_entry_point_candidate(
        &mut self,
        candidate: &LanguageEntryPointCandidate,
        language: Option<&str>,
        source_kind: &str,
        provenance_source: &str,
        adjacency: &BTreeMap<String, BTreeSet<String>>,
        deployment: Option<&DeploymentBinding>,
    ) {
        let entry_identity = candidate.artefact_id.as_deref().unwrap_or(&candidate.path);
        let entry_node_id = node_id(
            &self.repo_id,
            ArchitectureGraphNodeKind::EntryPoint,
            &format!("{}:{}", candidate.entry_kind, entry_identity),
        );
        let flow_node_id = node_id(
            &self.repo_id,
            ArchitectureGraphNodeKind::Flow,
            &format!("{}:{}", entry_node_id, candidate.entry_kind),
        );
        self.upsert_node(ArchitectureGraphNodeFact {
            repo_id: self.repo_id.clone(),
            node_id: entry_node_id.clone(),
            node_kind: ArchitectureGraphNodeKind::EntryPoint.as_str().to_string(),
            label: candidate.name.clone(),
            artefact_id: candidate.artefact_id.clone(),
            symbol_id: candidate.symbol_id.clone(),
            path: Some(candidate.path.clone()),
            entry_kind: Some(candidate.entry_kind.clone()),
            source_kind: source_kind.to_string(),
            confidence: candidate.confidence,
            provenance: self.provenance(provenance_source),
            evidence: json!(&candidate.evidence),
            properties: json!({
                "reason": &candidate.reason,
                "language": language,
            }),
            last_observed_generation: Some(self.generation),
        });
        self.upsert_node(ArchitectureGraphNodeFact {
            repo_id: self.repo_id.clone(),
            node_id: flow_node_id.clone(),
            node_kind: ArchitectureGraphNodeKind::Flow.as_str().to_string(),
            label: format!("{} flow", candidate.name),
            artefact_id: candidate.artefact_id.clone(),
            symbol_id: candidate.symbol_id.clone(),
            path: Some(candidate.path.clone()),
            entry_kind: Some(candidate.entry_kind.clone()),
            source_kind: "COMPUTED".to_string(),
            confidence: candidate.confidence.min(0.90),
            provenance: self.provenance("entry_point_flow_seed"),
            evidence: json!([{ "entryPointId": &entry_node_id }]),
            properties: json!({ "entry_kind": &candidate.entry_kind }),
            last_observed_generation: Some(self.generation),
        });
        if let Some(deployment) = deployment {
            self.upsert_edge_by_kind(
                ArchitectureGraphEdgeKind::Exposes,
                deployment.container_id.clone(),
                entry_node_id.clone(),
                "COMPUTED",
                candidate.confidence,
                self.provenance("entry_point_flow_seed"),
                json!([{ "path": &candidate.path }]),
                json!({}),
            );
        }
        self.upsert_edge_by_kind(
            ArchitectureGraphEdgeKind::Triggers,
            entry_node_id.clone(),
            flow_node_id.clone(),
            "COMPUTED",
            candidate.confidence,
            self.provenance("entry_point_flow_seed"),
            json!([{ "entryKind": &candidate.entry_kind }]),
            json!({}),
        );

        for traversed in self.traversed_code_nodes(candidate, adjacency) {
            self.upsert_edge_by_kind(
                ArchitectureGraphEdgeKind::Traverses,
                flow_node_id.clone(),
                traversed,
                "COMPUTED",
                candidate.confidence.min(0.80),
                self.provenance("dependency_closure"),
                json!([{ "entryPointId": &entry_node_id }]),
                json!({ "closure": "current_dependency_edges" }),
            );
        }
    }

    fn ensure_deployment_container_for_candidate(
        &mut self,
        candidate: &LanguageEntryPointCandidate,
    ) -> DeploymentBinding {
        let deployment_kind = candidate.entry_kind.clone();
        let deployment_identity =
            format!("{}:{}:{}", deployment_kind, candidate.name, candidate.path);
        let deployment_id =
            deployment_unit_node_id(&self.repo_id, &deployment_kind, &deployment_identity);
        let container_key = format!("{}:{}:{}", deployment_kind, candidate.name, candidate.path);
        let container_id = container_node_id(&self.repo_id, &container_key);
        let container_kind = infer_container_kind(candidate);
        let container_root = deployment_root_from_candidate(candidate);
        let system_id = self.fallback_system_id();
        let system_key = self.fallback_system_key();

        self.upsert_node(ArchitectureGraphNodeFact {
            repo_id: self.repo_id.clone(),
            node_id: deployment_id.clone(),
            node_kind: ArchitectureGraphNodeKind::DeploymentUnit
                .as_str()
                .to_string(),
            label: format!("{} deployment", candidate.name),
            artefact_id: candidate.artefact_id.clone(),
            symbol_id: candidate.symbol_id.clone(),
            path: Some(container_root.clone()),
            entry_kind: Some(candidate.entry_kind.clone()),
            source_kind: "CONFIG_EVIDENCE".to_string(),
            confidence: candidate.confidence,
            provenance: self.provenance("config_deployment_unit"),
            evidence: json!(&candidate.evidence),
            properties: json!({
                "deployment_kind": deployment_kind,
                "deployable_path": &candidate.path,
                "deployment_root": &container_root,
            }),
            last_observed_generation: Some(self.generation),
        });
        self.upsert_node(ArchitectureGraphNodeFact {
            repo_id: self.repo_id.clone(),
            node_id: container_id.clone(),
            node_kind: ArchitectureGraphNodeKind::Container.as_str().to_string(),
            label: candidate.name.clone(),
            artefact_id: candidate.artefact_id.clone(),
            symbol_id: candidate.symbol_id.clone(),
            path: Some(container_root.clone()),
            entry_kind: Some(candidate.entry_kind.clone()),
            source_kind: "CONFIG_EVIDENCE".to_string(),
            confidence: candidate.confidence.min(0.90),
            provenance: self.provenance("config_container"),
            evidence: json!(&candidate.evidence),
            properties: json!({
                "system_key": system_key,
                "container_key": container_key,
                "container_kind": container_kind,
                "deployment_kind": &candidate.entry_kind,
            }),
            last_observed_generation: Some(self.generation),
        });
        self.upsert_edge_by_kind(
            ArchitectureGraphEdgeKind::Contains,
            system_id.clone(),
            container_id.clone(),
            "COMPUTED",
            candidate.confidence.min(0.90),
            self.provenance("config_container"),
            json!([{ "path": &candidate.path }]),
            json!({ "system_key": system_key }),
        );
        self.upsert_edge_by_kind(
            ArchitectureGraphEdgeKind::Produces,
            system_id,
            deployment_id.clone(),
            "COMPUTED",
            candidate.confidence.min(0.85),
            self.provenance("config_deployment_unit"),
            json!([{ "path": &candidate.path }]),
            json!({ "deployment_kind": &candidate.entry_kind }),
        );
        self.upsert_edge_by_kind(
            ArchitectureGraphEdgeKind::Realises,
            deployment_id.clone(),
            container_id.clone(),
            "COMPUTED",
            candidate.confidence,
            self.provenance("config_container"),
            json!([{ "path": &candidate.path }]),
            json!({}),
        );

        let binding = DeploymentBinding {
            container_id,
            container_root,
        };
        if !self
            .container_bindings
            .iter()
            .any(|existing| existing.container_id == binding.container_id)
        {
            self.container_bindings.push(binding.clone());
        }
        binding
    }

    fn add_components_for_containers(&mut self, artefacts: &[CurrentCanonicalArtefactRecord]) {
        let bindings = self.container_bindings.clone();
        for artefact in artefacts {
            let Some(code_node_id) = self.artefact_nodes.get(&artefact.artefact_id).cloned() else {
                continue;
            };
            let Some(binding) = bindings
                .iter()
                .filter(|binding| path_in_root(&artefact.path, &binding.container_root))
                .max_by_key(|binding| binding.container_root.len())
            else {
                continue;
            };
            let Some(component_key) =
                component_key_for_path(&binding.container_root, &artefact.path)
            else {
                continue;
            };
            let component_id =
                component_node_id(&self.repo_id, &binding.container_id, &component_key);
            self.upsert_node(ArchitectureGraphNodeFact {
                repo_id: self.repo_id.clone(),
                node_id: component_id.clone(),
                node_kind: ArchitectureGraphNodeKind::Component.as_str().to_string(),
                label: component_label(&component_key),
                artefact_id: None,
                symbol_id: None,
                path: Some(component_path(&binding.container_root, &component_key)),
                entry_kind: None,
                source_kind: "HEURISTIC".to_string(),
                confidence: 0.55,
                provenance: self.provenance("component_path_boundary"),
                evidence: json!([{ "path": &artefact.path }]),
                properties: json!({
                    "component_key": component_key,
                    "container_id": &binding.container_id,
                }),
                last_observed_generation: Some(self.generation),
            });
            self.upsert_edge_by_kind(
                ArchitectureGraphEdgeKind::Contains,
                binding.container_id.clone(),
                component_id.clone(),
                "HEURISTIC",
                0.55,
                self.provenance("component_path_boundary"),
                json!([{ "path": &artefact.path }]),
                json!({}),
            );
            self.upsert_edge_by_kind(
                ArchitectureGraphEdgeKind::Implements,
                code_node_id,
                component_id,
                "HEURISTIC",
                0.55,
                self.provenance("component_path_boundary"),
                json!([{ "path": &artefact.path }]),
                json!({}),
            );
        }
    }

    fn traversed_code_nodes(
        &self,
        candidate: &LanguageEntryPointCandidate,
        adjacency: &BTreeMap<String, BTreeSet<String>>,
    ) -> BTreeSet<String> {
        let mut start_artefacts = BTreeSet::new();
        if let Some(artefact_id) = candidate.artefact_id.as_ref() {
            start_artefacts.insert(artefact_id.clone());
        } else if let Some(nodes) = self.path_nodes.get(&candidate.path) {
            return nodes.iter().cloned().collect();
        }

        let mut visited_artefacts = BTreeSet::new();
        let mut queue = VecDeque::new();
        for artefact_id in start_artefacts {
            visited_artefacts.insert(artefact_id.clone());
            queue.push_back(artefact_id);
        }
        while let Some(artefact_id) = queue.pop_front() {
            if let Some(next) = adjacency.get(&artefact_id) {
                for target in next {
                    if visited_artefacts.insert(target.clone()) {
                        queue.push_back(target.clone());
                    }
                }
            }
        }
        visited_artefacts
            .iter()
            .filter_map(|artefact_id| self.artefact_nodes.get(artefact_id).cloned())
            .collect()
    }

    fn add_change_unit(&mut self, request: &CurrentStateConsumerRequest) {
        let mut affected_paths = BTreeSet::new();
        affected_paths.extend(request.affected_paths.iter().cloned());
        affected_paths.extend(request.file_upserts.iter().map(|file| file.path.clone()));
        affected_paths.extend(request.file_removals.iter().map(|file| file.path.clone()));
        affected_paths.extend(
            request
                .artefact_upserts
                .iter()
                .map(|artefact| artefact.path.clone()),
        );
        affected_paths.extend(
            request
                .artefact_removals
                .iter()
                .map(|artefact| artefact.path.clone()),
        );
        if affected_paths.is_empty() && request.run_id.is_none() {
            return;
        }

        let change_node_id = node_id(
            &self.repo_id,
            ArchitectureGraphNodeKind::ChangeUnit,
            &format!("generation:{}", request.to_generation_seq_inclusive),
        );
        self.upsert_node(ArchitectureGraphNodeFact {
            repo_id: self.repo_id.clone(),
            node_id: change_node_id.clone(),
            node_kind: ArchitectureGraphNodeKind::ChangeUnit.as_str().to_string(),
            label: format!("DevQL generation {}", request.to_generation_seq_inclusive),
            artefact_id: None,
            symbol_id: None,
            path: None,
            entry_kind: None,
            source_kind: "CHANGE_DATA".to_string(),
            confidence: 0.80,
            provenance: self.provenance("current_state_reconcile"),
            evidence: json!([{
                "fromGenerationExclusive": request.from_generation_seq_exclusive,
                "toGenerationInclusive": request.to_generation_seq_inclusive,
                "runId": &request.run_id,
            }]),
            properties: json!({
                "active_branch": &request.active_branch,
                "head_commit_sha": &request.head_commit_sha,
                "affected_paths": &affected_paths,
            }),
            last_observed_generation: Some(self.generation),
        });
        let affected_node_ids: Vec<String> = affected_paths
            .iter()
            .filter_map(|path| self.path_nodes.get(path))
            .flat_map(|nodes| nodes.iter().cloned())
            .collect();
        for node_id in affected_node_ids {
            self.upsert_edge_by_kind(
                ArchitectureGraphEdgeKind::Impacts,
                change_node_id.clone(),
                node_id,
                "CHANGE_DATA",
                0.75,
                self.provenance("current_state_reconcile"),
                json!([{ "affectedPaths": &affected_paths }]),
                json!({}),
            );
        }
    }

    fn upsert_node(&mut self, node: ArchitectureGraphNodeFact) {
        match self.nodes.get(&node.node_id) {
            Some(existing) if existing.confidence >= node.confidence => {}
            _ => {
                self.nodes.insert(node.node_id.clone(), node);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn upsert_edge_by_kind(
        &mut self,
        kind: ArchitectureGraphEdgeKind,
        from_node_id: String,
        to_node_id: String,
        source_kind: &str,
        confidence: f64,
        provenance: Value,
        evidence: Value,
        properties: Value,
    ) {
        let edge_id = edge_id(&self.repo_id, kind, &from_node_id, &to_node_id);
        let edge = ArchitectureGraphEdgeFact {
            repo_id: self.repo_id.clone(),
            edge_id: edge_id.clone(),
            edge_kind: kind.as_str().to_string(),
            from_node_id,
            to_node_id,
            source_kind: source_kind.to_string(),
            confidence,
            provenance,
            evidence,
            properties,
            last_observed_generation: Some(self.generation),
        };
        match self.edges.get(&edge_id) {
            Some(existing) if existing.confidence >= edge.confidence => {}
            _ => {
                self.edges.insert(edge_id, edge);
            }
        }
    }

    fn provenance(&self, source: &str) -> Value {
        json!({
            "capability": ARCHITECTURE_GRAPH_CAPABILITY_ID,
            "consumer": ARCHITECTURE_GRAPH_CONSUMER_ID,
            "run_id": &self.run_id,
            "source": source,
        })
    }

    fn fallback_system_key(&self) -> String {
        format!("repo:{}", self.repo_id)
    }

    fn fallback_system_id(&self) -> String {
        system_node_id(&self.fallback_system_key())
    }
}

async fn add_test_harness_facts(
    context: &CurrentStateConsumerContext,
    builder: &mut GraphBuilder,
    warnings: &mut Vec<String>,
) {
    let test_rows = match optional_query(
        context,
        &format!(
            "SELECT artefact_id, symbol_id, path, name, canonical_kind, language_kind, symbol_fqn, start_line, end_line \
             FROM test_artefacts_current WHERE repo_id = '{}' ORDER BY path, start_line, symbol_id",
            crate::host::devql::esc_pg(&builder.repo_id)
        ),
        warnings,
    )
    .await
    {
        Some(rows) => rows,
        None => return,
    };

    let system_id = builder.fallback_system_id();
    let mut test_artefact_nodes = BTreeMap::new();
    let mut test_symbol_nodes = BTreeMap::new();
    for row in test_rows {
        let Some(artefact_id) = string_field(&row, "artefact_id") else {
            continue;
        };
        let Some(symbol_id) = string_field(&row, "symbol_id") else {
            continue;
        };
        let path = string_field(&row, "path");
        let test_node_id = node_id(
            &builder.repo_id,
            ArchitectureGraphNodeKind::Test,
            &artefact_id,
        );
        test_artefact_nodes.insert(artefact_id.clone(), test_node_id.clone());
        test_symbol_nodes.insert(symbol_id.clone(), test_node_id.clone());
        builder.upsert_node(ArchitectureGraphNodeFact {
            repo_id: builder.repo_id.clone(),
            node_id: test_node_id.clone(),
            node_kind: ArchitectureGraphNodeKind::Test.as_str().to_string(),
            label: string_field(&row, "name").unwrap_or_else(|| artefact_id.clone()),
            artefact_id: Some(artefact_id),
            symbol_id: Some(symbol_id),
            path,
            entry_kind: None,
            source_kind: "TEST_HARNESS".to_string(),
            confidence: 0.90,
            provenance: builder.provenance("test_harness_current_state"),
            evidence: json!([row]),
            properties: json!({}),
            last_observed_generation: Some(builder.generation),
        });
        builder.upsert_edge_by_kind(
            ArchitectureGraphEdgeKind::Contains,
            system_id.clone(),
            test_node_id,
            "TEST_HARNESS",
            0.80,
            builder.provenance("test_harness_current_state"),
            json!([]),
            json!({}),
        );
    }

    let edge_rows = match optional_query(
        context,
        &format!(
            "SELECT edge_id, from_artefact_id, from_symbol_id, to_artefact_id, to_symbol_id, edge_kind, path, start_line, end_line \
             FROM test_artefact_edges_current WHERE repo_id = '{}' ORDER BY edge_id",
            crate::host::devql::esc_pg(&builder.repo_id)
        ),
        warnings,
    )
    .await
    {
        Some(rows) => rows,
        None => return,
    };

    for row in edge_rows {
        let test_node = string_field(&row, "from_artefact_id")
            .and_then(|id| test_artefact_nodes.get(&id).cloned())
            .or_else(|| {
                string_field(&row, "from_symbol_id")
                    .and_then(|id| test_symbol_nodes.get(&id).cloned())
            });
        let production_node = string_field(&row, "to_artefact_id")
            .and_then(|id| builder.artefact_nodes.get(&id).cloned())
            .or_else(|| {
                string_field(&row, "to_symbol_id")
                    .and_then(|id| builder.symbol_nodes.get(&id).cloned())
            });
        let (Some(production_node), Some(test_node)) = (production_node, test_node) else {
            continue;
        };
        builder.upsert_edge_by_kind(
            ArchitectureGraphEdgeKind::VerifiedBy,
            production_node,
            test_node,
            "TEST_HARNESS",
            0.75,
            builder.provenance("test_harness_current_state"),
            json!([row]),
            json!({}),
        );
    }
}

async fn optional_query(
    context: &CurrentStateConsumerContext,
    sql: &str,
    warnings: &mut Vec<String>,
) -> Option<Vec<Value>> {
    match context.storage.query_rows(sql).await {
        Ok(rows) => Some(rows),
        Err(err) if err.to_string().contains("no such table") => {
            warnings.push(format!(
                "Optional architecture graph source unavailable: {err}"
            ));
            None
        }
        Err(err) => {
            warnings.push(format!(
                "Optional architecture graph source query failed: {err:#}"
            ));
            None
        }
    }
}

fn group_entry_point_artefacts_by_path(
    artefacts: &[CurrentCanonicalArtefactRecord],
) -> BTreeMap<String, Vec<LanguageEntryPointArtefact>> {
    let mut grouped: BTreeMap<String, Vec<LanguageEntryPointArtefact>> = BTreeMap::new();
    for artefact in artefacts {
        grouped
            .entry(artefact.path.clone())
            .or_default()
            .push(LanguageEntryPointArtefact {
                artefact_id: artefact.artefact_id.clone(),
                symbol_id: artefact.symbol_id.clone(),
                path: artefact.path.clone(),
                name: artefact_name(artefact),
                canonical_kind: artefact.canonical_kind.clone(),
                language_kind: artefact.language_kind.clone(),
                symbol_fqn: artefact.symbol_fqn.clone(),
                signature: artefact.signature.clone(),
                modifiers: parse_modifiers(&artefact.modifiers),
                start_line: artefact.start_line,
                end_line: artefact.end_line,
            });
    }
    grouped
}

fn dependency_adjacency(
    edges: &[CurrentCanonicalEdgeRecord],
) -> BTreeMap<String, BTreeSet<String>> {
    let mut adjacency = BTreeMap::new();
    for edge in edges {
        let Some(to) = edge.to_artefact_id.as_ref() else {
            continue;
        };
        adjacency
            .entry(edge.from_artefact_id.clone())
            .or_insert_with(BTreeSet::new)
            .insert(to.clone());
    }
    adjacency
}

fn infer_container_kind(candidate: &LanguageEntryPointCandidate) -> &'static str {
    match candidate.entry_kind.as_str() {
        "cargo_bin" | "npm_bin" | "python_console_script" | "rust_cli_dispatch" => "cli",
        "npm_script" if candidate.name == "worker" => "worker",
        "npm_script" | "python_web_app" | "go_http_handler" | "next_route_handler" => "service",
        _ => "runtime",
    }
}

fn deployment_root_from_candidate(candidate: &LanguageEntryPointCandidate) -> String {
    candidate
        .evidence
        .iter()
        .find(|path| {
            path.ends_with("Cargo.toml")
                || path.ends_with("package.json")
                || path.ends_with("pyproject.toml")
        })
        .and_then(|path| Path::new(path).parent())
        .map(normalise_repo_path)
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| {
            Path::new(&candidate.path)
                .parent()
                .map(normalise_repo_path)
                .filter(|path| !path.is_empty())
                .unwrap_or_else(|| ".".to_string())
        })
}

fn path_in_root(path: &str, root: &str) -> bool {
    root == "."
        || path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn component_key_for_path(root: &str, path: &str) -> Option<String> {
    let relative = if root == "." {
        path
    } else {
        path.strip_prefix(root)?.trim_start_matches('/')
    };
    let parts = relative
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return None;
    }
    if parts[0] == "src" && parts.len() > 1 {
        return Some(format!("src/{}", component_segment(parts[1])));
    }
    Some(component_segment(parts[0]))
}

fn component_segment(path_segment: &str) -> String {
    Path::new(path_segment)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or(path_segment)
        .to_string()
}

fn component_label(component_key: &str) -> String {
    component_key
        .rsplit('/')
        .next()
        .unwrap_or(component_key)
        .replace(['_', '-'], " ")
}

fn component_path(root: &str, component_key: &str) -> String {
    if root == "." {
        component_key.to_string()
    } else {
        format!("{root}/{component_key}")
    }
}

fn detect_config_entry_points(
    repo_root: &Path,
    files: &[CurrentCanonicalFileRecord],
    artefacts_by_path: &BTreeMap<String, Vec<LanguageEntryPointArtefact>>,
) -> Vec<LanguageEntryPointCandidate> {
    let file_paths = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    let mut candidates = Vec::new();
    for file in files {
        let Some(basename) = Path::new(&file.path)
            .file_name()
            .and_then(|name| name.to_str())
        else {
            continue;
        };
        match basename {
            "Cargo.toml" => detect_cargo_entry_points(
                repo_root,
                &file.path,
                &file_paths,
                artefacts_by_path,
                &mut candidates,
            ),
            "package.json" => detect_package_json_entry_points(
                repo_root,
                &file.path,
                &file_paths,
                artefacts_by_path,
                &mut candidates,
            ),
            "pyproject.toml" => detect_pyproject_entry_points(
                repo_root,
                &file.path,
                &file_paths,
                artefacts_by_path,
                &mut candidates,
            ),
            _ => {}
        }
    }
    candidates
}

fn detect_cargo_entry_points(
    repo_root: &Path,
    config_path: &str,
    file_paths: &BTreeSet<String>,
    artefacts_by_path: &BTreeMap<String, Vec<LanguageEntryPointArtefact>>,
    candidates: &mut Vec<LanguageEntryPointCandidate>,
) {
    let Some(content) = read_repo_file(repo_root, config_path) else {
        return;
    };
    let Ok(document) = content.parse::<toml_edit::DocumentMut>() else {
        return;
    };
    let package_name = document
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(|name| name.as_str())
        .map(ToString::to_string);
    let Some(package_name) = package_name else {
        return;
    };

    let default_main = repo_relative_join(config_path, "src/main.rs");
    if file_paths.contains(&default_main) {
        candidates.push(config_candidate_for_path(
            &default_main,
            artefacts_by_path,
            "cargo_bin",
            &package_name,
            0.92,
            "Cargo package default binary target",
            vec![config_path.to_string(), default_main.clone()],
        ));
    }

    if let Some(bins) = document
        .get("bin")
        .and_then(|item| item.as_array_of_tables())
    {
        for bin in bins {
            let name = bin
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or(package_name.as_str());
            let explicit_path = bin
                .get("path")
                .and_then(|value| value.as_str())
                .map(|path| repo_relative_join(config_path, path));
            let inferred_path = repo_relative_join(config_path, &format!("src/bin/{name}.rs"));
            let path = explicit_path
                .filter(|path| file_paths.contains(path))
                .or_else(|| file_paths.contains(&inferred_path).then_some(inferred_path));
            if let Some(path) = path {
                candidates.push(config_candidate_for_path(
                    &path,
                    artefacts_by_path,
                    "cargo_bin",
                    name,
                    0.94,
                    "Cargo explicit binary target",
                    vec![config_path.to_string(), path.clone()],
                ));
            }
        }
    }
}

fn detect_package_json_entry_points(
    repo_root: &Path,
    config_path: &str,
    file_paths: &BTreeSet<String>,
    artefacts_by_path: &BTreeMap<String, Vec<LanguageEntryPointArtefact>>,
    candidates: &mut Vec<LanguageEntryPointCandidate>,
) {
    let Some(content) = read_repo_file(repo_root, config_path) else {
        return;
    };
    let Ok(document) = serde_json::from_str::<Value>(&content) else {
        return;
    };
    let package_name = document
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("package");

    match document.get("bin") {
        Some(Value::String(path)) => {
            let path = repo_relative_join(config_path, path);
            if file_paths.contains(&path) {
                candidates.push(config_candidate_for_path(
                    &path,
                    artefacts_by_path,
                    "npm_bin",
                    package_name,
                    0.90,
                    "package.json binary target",
                    vec![config_path.to_string(), path.clone()],
                ));
            }
        }
        Some(Value::Object(bins)) => {
            for (name, path) in bins {
                let Some(path) = path.as_str() else {
                    continue;
                };
                let path = repo_relative_join(config_path, path);
                if file_paths.contains(&path) {
                    candidates.push(config_candidate_for_path(
                        &path,
                        artefacts_by_path,
                        "npm_bin",
                        name,
                        0.90,
                        "package.json binary target",
                        vec![config_path.to_string(), path.clone()],
                    ));
                }
            }
        }
        _ => {}
    }

    if let Some(Value::Object(scripts)) = document.get("scripts") {
        for script_name in ["start", "dev", "serve", "worker", "cli"] {
            let Some(script) = scripts.get(script_name).and_then(Value::as_str) else {
                continue;
            };
            if let Some(path) = script_entry_path(config_path, script, file_paths) {
                candidates.push(config_candidate_for_path(
                    &path,
                    artefacts_by_path,
                    "npm_script",
                    script_name,
                    0.76,
                    "package.json runtime script",
                    vec![config_path.to_string(), script.to_string(), path.clone()],
                ));
            }
        }
    }
}

fn detect_pyproject_entry_points(
    repo_root: &Path,
    config_path: &str,
    file_paths: &BTreeSet<String>,
    artefacts_by_path: &BTreeMap<String, Vec<LanguageEntryPointArtefact>>,
    candidates: &mut Vec<LanguageEntryPointCandidate>,
) {
    let Some(content) = read_repo_file(repo_root, config_path) else {
        return;
    };
    let Ok(document) = content.parse::<toml_edit::DocumentMut>() else {
        return;
    };
    let Some(scripts) = document
        .get("project")
        .and_then(|project| project.get("scripts"))
        .and_then(|scripts| scripts.as_table())
    else {
        return;
    };
    for (name, value) in scripts {
        let Some(target) = value.as_str() else {
            continue;
        };
        let Some((module, _function)) = target.split_once(':') else {
            continue;
        };
        let module_path = module.replace('.', "/");
        let candidates_for_script = [
            repo_relative_join(config_path, &format!("{module_path}.py")),
            repo_relative_join(config_path, &format!("src/{module_path}.py")),
            repo_relative_join(config_path, &format!("{module_path}/__init__.py")),
            repo_relative_join(config_path, &format!("src/{module_path}/__init__.py")),
        ];
        if let Some(path) = candidates_for_script
            .into_iter()
            .find(|path| file_paths.contains(path))
        {
            candidates.push(config_candidate_for_path(
                &path,
                artefacts_by_path,
                "python_console_script",
                name,
                0.86,
                "pyproject.toml console script",
                vec![config_path.to_string(), target.to_string(), path.clone()],
            ));
        }
    }
}

fn config_candidate_for_path(
    path: &str,
    artefacts_by_path: &BTreeMap<String, Vec<LanguageEntryPointArtefact>>,
    entry_kind: &str,
    name: &str,
    confidence: f64,
    reason: &str,
    evidence: Vec<String>,
) -> LanguageEntryPointCandidate {
    let artefact = artefacts_by_path.get(path).and_then(|artefacts| {
        artefacts
            .iter()
            .find(|artefact| artefact.name == "main")
            .or_else(|| {
                artefacts
                    .iter()
                    .find(|artefact| is_entry_candidate_artefact(artefact))
            })
            .or_else(|| artefacts.first())
    });
    LanguageEntryPointCandidate {
        path: path.to_string(),
        artefact_id: artefact.map(|artefact| artefact.artefact_id.clone()),
        symbol_id: artefact.map(|artefact| artefact.symbol_id.clone()),
        name: if name.is_empty() {
            artefact
                .map(|artefact| artefact.name.clone())
                .unwrap_or_else(|| path.to_string())
        } else {
            name.to_string()
        },
        entry_kind: entry_kind.to_string(),
        confidence,
        reason: reason.to_string(),
        evidence,
    }
}

fn is_entry_candidate_artefact(artefact: &LanguageEntryPointArtefact) -> bool {
    matches!(
        artefact.canonical_kind.as_deref(),
        Some("function" | "method" | "callable" | "value" | "variable")
    )
}

fn script_entry_path(
    config_path: &str,
    script: &str,
    file_paths: &BTreeSet<String>,
) -> Option<String> {
    for token in script.split_whitespace() {
        let token = token
            .trim_matches(|ch| matches!(ch, '"' | '\'' | '(' | ')' | ','))
            .trim_start_matches("./");
        if !matches!(
            Path::new(token).extension().and_then(|ext| ext.to_str()),
            Some("js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs")
        ) {
            continue;
        }
        let path = repo_relative_join(config_path, token);
        if file_paths.contains(&path) {
            return Some(path);
        }
    }

    [
        "src/index.ts",
        "src/index.js",
        "src/server.ts",
        "src/server.js",
        "server.ts",
        "server.js",
        "app.ts",
        "app.js",
    ]
    .into_iter()
    .map(|path| repo_relative_join(config_path, path))
    .find(|path| file_paths.contains(path))
}

fn read_repo_file(repo_root: &Path, relative_path: &str) -> Option<String> {
    std::fs::read_to_string(repo_root.join(relative_path)).ok()
}

fn repo_relative_join(config_path: &str, child_path: &str) -> String {
    let mut path = PathBuf::new();
    if let Some(parent) = Path::new(config_path).parent()
        && parent != Path::new("")
    {
        path.push(parent);
    }
    path.push(child_path.trim_start_matches("./"));
    normalise_repo_path(&path)
}

fn normalise_repo_path(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(value) => {
                if let Some(value) = value.to_str() {
                    parts.push(value);
                }
            }
            _ => {}
        }
    }
    parts.join("/")
}

fn artefact_display_name(artefact: &CurrentCanonicalArtefactRecord) -> String {
    artefact
        .symbol_fqn
        .as_deref()
        .map(last_symbol_segment)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| artefact.artefact_id.clone())
}

fn artefact_name(artefact: &CurrentCanonicalArtefactRecord) -> String {
    artefact_display_name(artefact)
}

fn last_symbol_segment(value: &str) -> String {
    value
        .rsplit([':', '.', '#'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(value)
        .to_string()
}

fn parse_modifiers(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

fn string_field(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, language: &str) -> CurrentCanonicalFileRecord {
        CurrentCanonicalFileRecord {
            repo_id: "repo".to_string(),
            path: path.to_string(),
            analysis_mode: "parsed".to_string(),
            file_role: "source".to_string(),
            language: language.to_string(),
            resolved_language: language.to_string(),
            effective_content_id: format!("content:{path}"),
            parser_version: "test".to_string(),
            extractor_version: "test".to_string(),
            exists_in_head: true,
            exists_in_index: true,
            exists_in_worktree: true,
        }
    }

    fn entry_artefact(path: &str, name: &str, kind: &str) -> LanguageEntryPointArtefact {
        LanguageEntryPointArtefact {
            artefact_id: format!("{path}:{name}:artefact"),
            symbol_id: format!("{path}:{name}:symbol"),
            path: path.to_string(),
            name: name.to_string(),
            canonical_kind: Some(kind.to_string()),
            language_kind: Some("function_item".to_string()),
            symbol_fqn: Some(format!("{path}::{name}")),
            signature: None,
            modifiers: Vec::new(),
            start_line: 1,
            end_line: 3,
        }
    }

    fn current_artefact(path: &str, name: &str) -> CurrentCanonicalArtefactRecord {
        CurrentCanonicalArtefactRecord {
            repo_id: "repo".to_string(),
            path: path.to_string(),
            content_id: format!("content:{path}"),
            symbol_id: format!("{path}:{name}:symbol"),
            artefact_id: format!("{path}:{name}:artefact"),
            language: "rust".to_string(),
            extraction_fingerprint: "fingerprint".to_string(),
            canonical_kind: Some("function".to_string()),
            language_kind: Some("function_item".to_string()),
            symbol_fqn: Some(format!("{path}::{name}")),
            parent_symbol_id: None,
            parent_artefact_id: None,
            start_line: 1,
            end_line: 3,
            start_byte: 0,
            end_byte: 10,
            signature: None,
            modifiers: "[]".to_string(),
            docstring: None,
        }
    }

    #[test]
    fn dependency_adjacency_keeps_resolved_edges_only() {
        let edges = vec![
            CurrentCanonicalEdgeRecord {
                repo_id: "repo".to_string(),
                edge_id: "edge-1".to_string(),
                path: "src/lib.rs".to_string(),
                content_id: "content".to_string(),
                from_symbol_id: "a".to_string(),
                from_artefact_id: "a-art".to_string(),
                to_symbol_id: Some("b".to_string()),
                to_artefact_id: Some("b-art".to_string()),
                to_symbol_ref: None,
                edge_kind: "call".to_string(),
                language: "rust".to_string(),
                start_line: None,
                end_line: None,
                metadata: "{}".to_string(),
            },
            CurrentCanonicalEdgeRecord {
                repo_id: "repo".to_string(),
                edge_id: "edge-2".to_string(),
                path: "src/lib.rs".to_string(),
                content_id: "content".to_string(),
                from_symbol_id: "b".to_string(),
                from_artefact_id: "b-art".to_string(),
                to_symbol_id: None,
                to_artefact_id: None,
                to_symbol_ref: Some("external".to_string()),
                edge_kind: "call".to_string(),
                language: "rust".to_string(),
                start_line: None,
                end_line: None,
                metadata: "{}".to_string(),
            },
        ];

        let adjacency = dependency_adjacency(&edges);

        assert_eq!(
            adjacency
                .get("a-art")
                .and_then(|targets| targets.iter().next())
                .map(String::as_str),
            Some("b-art")
        );
        assert!(!adjacency.contains_key("b-art"));
    }

    #[test]
    fn config_entry_points_include_cargo_package_binary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cargo_path = temp.path().join("crates/bitloops-inference/Cargo.toml");
        std::fs::create_dir_all(cargo_path.parent().expect("parent")).expect("create dirs");
        std::fs::write(
            &cargo_path,
            "[package]\nname = \"bitloops-inference\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write Cargo.toml");
        std::fs::create_dir_all(temp.path().join("crates/bitloops-inference/src"))
            .expect("create src");
        std::fs::write(
            temp.path().join("crates/bitloops-inference/src/main.rs"),
            "fn main() {}\n",
        )
        .expect("write main");

        let files = vec![
            file("crates/bitloops-inference/Cargo.toml", "toml"),
            file("crates/bitloops-inference/src/main.rs", "rust"),
        ];
        let artefacts = vec![entry_artefact(
            "crates/bitloops-inference/src/main.rs",
            "main",
            "function",
        )];
        let grouped = group_artefacts_for_test(artefacts);

        let candidates = detect_config_entry_points(temp.path(), &files, &grouped);

        let cargo_bin = candidates
            .iter()
            .find(|candidate| candidate.entry_kind == "cargo_bin")
            .expect("cargo bin entry point");
        assert_eq!(cargo_bin.path, "crates/bitloops-inference/src/main.rs");
        assert_eq!(cargo_bin.name, "bitloops-inference");
        assert!(cargo_bin.artefact_id.is_some());
    }

    #[test]
    fn config_entry_points_include_package_json_bin_and_runtime_script() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("package.json"),
            r#"{
  "name": "sample-cli",
  "bin": { "sample": "./src/cli.ts" },
  "scripts": { "start": "tsx ./src/server.ts" }
}
"#,
        )
        .expect("write package.json");
        std::fs::create_dir_all(temp.path().join("src")).expect("create src");
        std::fs::write(
            temp.path().join("src/cli.ts"),
            "export function main() {}\n",
        )
        .expect("write cli");
        std::fs::write(
            temp.path().join("src/server.ts"),
            "export function startServer() {}\n",
        )
        .expect("write server");

        let files = vec![
            file("package.json", "json"),
            file("src/cli.ts", "typescript"),
            file("src/server.ts", "typescript"),
        ];
        let grouped = group_artefacts_for_test(vec![
            entry_artefact("src/cli.ts", "main", "function"),
            entry_artefact("src/server.ts", "startServer", "function"),
        ]);

        let candidates = detect_config_entry_points(temp.path(), &files, &grouped);

        assert!(
            candidates.iter().any(
                |candidate| candidate.entry_kind == "npm_bin" && candidate.path == "src/cli.ts"
            )
        );
        assert!(candidates.iter().any(|candidate| {
            candidate.entry_kind == "npm_script" && candidate.path == "src/server.ts"
        }));
    }

    #[test]
    fn repo_structure_creates_fallback_system_without_deployment_unit() {
        let mut builder = GraphBuilder::new("repo", 7, "run");
        builder.seed_repo_structure();

        let facts = builder.finish();

        assert!(facts.nodes.iter().any(|node| {
            node.node_kind == ArchitectureGraphNodeKind::System.as_str()
                && node.properties["system_key"] == "repo:repo"
        }));
        assert!(
            !facts
                .nodes
                .iter()
                .any(|node| node.node_kind == ArchitectureGraphNodeKind::DeploymentUnit.as_str()),
            "repo root alone must not be a deployment unit"
        );
    }

    #[test]
    fn config_candidate_creates_deployment_container_and_realises_edge() {
        let mut builder = GraphBuilder::new("repo", 7, "run");
        builder.seed_repo_structure();
        let candidate = LanguageEntryPointCandidate {
            path: "crates/cli/src/main.rs".to_string(),
            artefact_id: Some("main-art".to_string()),
            symbol_id: Some("main-symbol".to_string()),
            name: "cli".to_string(),
            entry_kind: "cargo_bin".to_string(),
            confidence: 0.94,
            reason: "Cargo binary target".to_string(),
            evidence: vec![
                "crates/cli/Cargo.toml".to_string(),
                "crates/cli/src/main.rs".to_string(),
            ],
        };

        builder.ensure_deployment_container_for_candidate(&candidate);
        let facts = builder.finish();

        assert!(facts.nodes.iter().any(|node| {
            node.node_kind == ArchitectureGraphNodeKind::DeploymentUnit.as_str()
                && node.properties["deployment_kind"] == "cargo_bin"
        }));
        assert!(facts.nodes.iter().any(|node| {
            node.node_kind == ArchitectureGraphNodeKind::Container.as_str()
                && node.properties["container_kind"] == "cli"
                && node.properties["system_key"] == "repo:repo"
        }));
        assert!(
            facts
                .edges
                .iter()
                .any(|edge| edge.edge_kind == ArchitectureGraphEdgeKind::Realises.as_str())
        );
        assert!(
            facts
                .edges
                .iter()
                .any(|edge| edge.edge_kind == ArchitectureGraphEdgeKind::Produces.as_str())
        );
    }

    #[test]
    fn components_are_inferred_inside_detected_container() {
        let mut builder = GraphBuilder::new("repo", 7, "run");
        builder.seed_repo_structure();
        let artefacts = vec![
            current_artefact("crates/cli/src/main.rs", "main"),
            current_artefact("crates/cli/src/runtime.rs", "run"),
        ];
        builder.add_code_nodes(&artefacts);
        let candidate = LanguageEntryPointCandidate {
            path: "crates/cli/src/main.rs".to_string(),
            artefact_id: Some("crates/cli/src/main.rs:main:artefact".to_string()),
            symbol_id: Some("crates/cli/src/main.rs:main:symbol".to_string()),
            name: "cli".to_string(),
            entry_kind: "cargo_bin".to_string(),
            confidence: 0.94,
            reason: "Cargo binary target".to_string(),
            evidence: vec![
                "crates/cli/Cargo.toml".to_string(),
                "crates/cli/src/main.rs".to_string(),
            ],
        };
        builder.ensure_deployment_container_for_candidate(&candidate);

        builder.add_components_for_containers(&artefacts);
        let facts = builder.finish();

        assert!(facts.nodes.iter().any(|node| {
            node.node_kind == ArchitectureGraphNodeKind::Component.as_str()
                && node.properties["component_key"] == "src/runtime"
        }));
        assert!(
            facts
                .edges
                .iter()
                .any(|edge| edge.edge_kind == ArchitectureGraphEdgeKind::Implements.as_str())
        );
    }

    fn group_artefacts_for_test(
        artefacts: Vec<LanguageEntryPointArtefact>,
    ) -> BTreeMap<String, Vec<LanguageEntryPointArtefact>> {
        let mut grouped = BTreeMap::new();
        for artefact in artefacts {
            grouped
                .entry(artefact.path.clone())
                .or_insert_with(Vec::new)
                .push(artefact);
        }
        grouped
    }
}
