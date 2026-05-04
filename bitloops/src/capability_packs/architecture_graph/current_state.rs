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

mod entry_points;

use entry_points::*;

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
        let mut deployment_by_root = BTreeMap::<String, DeploymentBinding>::new();
        for candidate in &config_candidates {
            if !is_deployable_config_candidate(candidate) {
                continue;
            }
            let binding = self.ensure_deployment_container_for_candidate(candidate);
            deployment_by_path
                .entry(candidate.path.clone())
                .or_insert_with(|| binding.clone());
            deployment_by_root
                .entry(deployment_root_from_candidate(candidate))
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
                let binding = deployment_binding_for_candidate(
                    &candidate,
                    &deployment_by_path,
                    &deployment_by_root,
                );
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
            let binding = deployment_binding_for_candidate(
                &candidate,
                &deployment_by_path,
                &deployment_by_root,
            );
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

#[cfg(test)]
mod tests;
