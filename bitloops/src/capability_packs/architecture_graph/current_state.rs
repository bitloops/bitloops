use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::host::capability_host::{
    CurrentStateConsumer, CurrentStateConsumerContext, CurrentStateConsumerFuture,
    CurrentStateConsumerRequest, CurrentStateConsumerResult,
};
use crate::host::inference::StructuredGenerationRequest;
use crate::host::language_adapter::{
    LanguageEntryPointArtefact, LanguageEntryPointCandidate, LanguageEntryPointFile,
};
use crate::models::{
    CurrentCanonicalArtefactRecord, CurrentCanonicalEdgeRecord, CurrentCanonicalFileRecord,
};

use super::storage::{
    ArchitectureGraphEdgeFact, ArchitectureGraphFacts, ArchitectureGraphNodeFact,
    component_node_id, container_node_id, deployment_unit_node_id, edge_id, edge_id_for_kind,
    node_id, replace_computed_graph, system_node_id,
};
use super::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_CONSUMER_ID,
    ARCHITECTURE_GRAPH_FACT_SYNTHESIS_SLOT, ArchitectureGraphEdgeKind, ArchitectureGraphNodeKind,
};

mod builder;
mod consumer;
mod entry_points;
mod synthesis;
mod test_harness;

use builder::{DeploymentBinding, GraphBuilder};
pub use consumer::ArchitectureGraphCurrentStateConsumer;
use entry_points::*;
use synthesis::add_agent_synthesised_facts;
#[cfg(test)]
use synthesis::architecture_fact_synthesis_user_prompt;
use test_harness::add_test_harness_facts;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ComponentArtefactInput {
    pub artefact_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ChangeUnitMetrics {
    pub affected_paths: usize,
    pub impacted_nodes: usize,
}

#[cfg(test)]
mod tests;
