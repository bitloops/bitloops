use async_graphql::{Enum, InputObject, SimpleObject, types::Json};
use serde_json::Value;

use super::JsonScalar;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ArchitectureGraphNodeKind {
    System,
    DeploymentUnit,
    Container,
    Component,
    Domain,
    Entity,
    Capability,
    EntryPoint,
    Flow,
    Node,
    PersistenceObject,
    Event,
    ExternalSystem,
    Contract,
    Test,
    ChangeUnit,
    RiskSignal,
}

impl ArchitectureGraphNodeKind {
    pub(crate) fn as_db(self) -> &'static str {
        match self {
            Self::System => "SYSTEM",
            Self::DeploymentUnit => "DEPLOYMENT_UNIT",
            Self::Container => "CONTAINER",
            Self::Component => "COMPONENT",
            Self::Domain => "DOMAIN",
            Self::Entity => "ENTITY",
            Self::Capability => "CAPABILITY",
            Self::EntryPoint => "ENTRY_POINT",
            Self::Flow => "FLOW",
            Self::Node => "NODE",
            Self::PersistenceObject => "PERSISTENCE_OBJECT",
            Self::Event => "EVENT",
            Self::ExternalSystem => "EXTERNAL_SYSTEM",
            Self::Contract => "CONTRACT",
            Self::Test => "TEST",
            Self::ChangeUnit => "CHANGE_UNIT",
            Self::RiskSignal => "RISK_SIGNAL",
        }
    }

    pub(crate) fn from_db(value: &str) -> Option<Self> {
        match value {
            "SYSTEM" => Some(Self::System),
            "DEPLOYMENT_UNIT" => Some(Self::DeploymentUnit),
            "CONTAINER" => Some(Self::Container),
            "COMPONENT" => Some(Self::Component),
            "DOMAIN" => Some(Self::Domain),
            "ENTITY" => Some(Self::Entity),
            "CAPABILITY" => Some(Self::Capability),
            "ENTRY_POINT" => Some(Self::EntryPoint),
            "FLOW" => Some(Self::Flow),
            "NODE" => Some(Self::Node),
            "PERSISTENCE_OBJECT" => Some(Self::PersistenceObject),
            "EVENT" => Some(Self::Event),
            "EXTERNAL_SYSTEM" => Some(Self::ExternalSystem),
            "CONTRACT" => Some(Self::Contract),
            "TEST" => Some(Self::Test),
            "CHANGE_UNIT" => Some(Self::ChangeUnit),
            "RISK_SIGNAL" => Some(Self::RiskSignal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ArchitectureGraphEdgeKind {
    Contains,
    Owns,
    Exposes,
    Produces,
    Realises,
    Triggers,
    Traverses,
    Reads,
    Writes,
    Emits,
    Calls,
    Implements,
    DependsOn,
    VerifiedBy,
    Stores,
    Modifies,
    Impacts,
    Scores,
}

impl ArchitectureGraphEdgeKind {
    pub(crate) fn as_db(self) -> &'static str {
        match self {
            Self::Contains => "CONTAINS",
            Self::Owns => "OWNS",
            Self::Exposes => "EXPOSES",
            Self::Produces => "PRODUCES",
            Self::Realises => "REALISES",
            Self::Triggers => "TRIGGERS",
            Self::Traverses => "TRAVERSES",
            Self::Reads => "READS",
            Self::Writes => "WRITES",
            Self::Emits => "EMITS",
            Self::Calls => "CALLS",
            Self::Implements => "IMPLEMENTS",
            Self::DependsOn => "DEPENDS_ON",
            Self::VerifiedBy => "VERIFIED_BY",
            Self::Stores => "STORES",
            Self::Modifies => "MODIFIES",
            Self::Impacts => "IMPACTS",
            Self::Scores => "SCORES",
        }
    }

    pub(crate) fn from_db(value: &str) -> Option<Self> {
        match value {
            "CONTAINS" => Some(Self::Contains),
            "OWNS" => Some(Self::Owns),
            "EXPOSES" => Some(Self::Exposes),
            "PRODUCES" => Some(Self::Produces),
            "REALISES" => Some(Self::Realises),
            "TRIGGERS" => Some(Self::Triggers),
            "TRAVERSES" => Some(Self::Traverses),
            "READS" => Some(Self::Reads),
            "WRITES" => Some(Self::Writes),
            "EMITS" => Some(Self::Emits),
            "CALLS" => Some(Self::Calls),
            "IMPLEMENTS" => Some(Self::Implements),
            "DEPENDS_ON" => Some(Self::DependsOn),
            "VERIFIED_BY" => Some(Self::VerifiedBy),
            "STORES" => Some(Self::Stores),
            "MODIFIES" => Some(Self::Modifies),
            "IMPACTS" => Some(Self::Impacts),
            "SCORES" => Some(Self::Scores),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ArchitectureGraphAssertionAction {
    Assert,
    Suppress,
    Annotate,
}

impl ArchitectureGraphAssertionAction {
    pub(crate) fn as_db(self) -> &'static str {
        match self {
            Self::Assert => "ASSERT",
            Self::Suppress => "SUPPRESS",
            Self::Annotate => "ANNOTATE",
        }
    }

    pub(crate) fn from_db(value: &str) -> Option<Self> {
        match value {
            "ASSERT" => Some(Self::Assert),
            "SUPPRESS" => Some(Self::Suppress),
            "ANNOTATE" => Some(Self::Annotate),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ArchitectureGraphTargetKind {
    Node,
    Edge,
}

impl ArchitectureGraphTargetKind {
    pub(crate) fn as_db(self) -> &'static str {
        match self {
            Self::Node => "NODE",
            Self::Edge => "EDGE",
        }
    }

    pub(crate) fn from_db(value: &str) -> Option<Self> {
        match value {
            "NODE" => Some(Self::Node),
            "EDGE" => Some(Self::Edge),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, InputObject, Default)]
pub struct ArchitectureGraphFilterInput {
    #[graphql(default)]
    pub node_kind: Option<ArchitectureGraphNodeKind>,
    #[graphql(default)]
    pub edge_kind: Option<ArchitectureGraphEdgeKind>,
    #[graphql(default)]
    pub path: Option<String>,
    #[graphql(default)]
    pub source_kind: Option<String>,
    #[graphql(default = true)]
    pub effective_only: bool,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct ArchitectureGraph {
    pub nodes: Vec<ArchitectureGraphNode>,
    pub edges: Vec<ArchitectureGraphEdge>,
    pub total_nodes: i32,
    pub total_edges: i32,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct ArchitectureGraphNode {
    pub id: String,
    pub kind: ArchitectureGraphNodeKind,
    pub label: String,
    pub artefact_id: Option<String>,
    pub symbol_id: Option<String>,
    pub path: Option<String>,
    pub entry_kind: Option<String>,
    pub source_kind: String,
    pub confidence: f64,
    pub computed: bool,
    pub asserted: bool,
    pub suppressed: bool,
    pub effective: bool,
    pub provenance: JsonScalar,
    pub computed_provenance: JsonScalar,
    pub asserted_provenance: JsonScalar,
    pub evidence: JsonScalar,
    pub properties: JsonScalar,
    pub annotations: Vec<ArchitectureGraphAssertionSummary>,
}

impl ArchitectureGraphNode {
    pub(crate) fn assertion(
        id: String,
        kind: ArchitectureGraphNodeKind,
        label: String,
        artefact_id: Option<String>,
        symbol_id: Option<String>,
        path: Option<String>,
        entry_kind: Option<String>,
        source_kind: String,
        confidence: f64,
        provenance: Value,
        evidence: Value,
        properties: Value,
    ) -> Self {
        Self {
            id,
            kind,
            label,
            artefact_id,
            symbol_id,
            path,
            entry_kind,
            source_kind,
            confidence,
            computed: false,
            asserted: true,
            suppressed: false,
            effective: true,
            provenance: Json(provenance.clone()),
            computed_provenance: Json(Value::Null),
            asserted_provenance: Json(provenance),
            evidence: Json(evidence),
            properties: Json(properties),
            annotations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct ArchitectureGraphEdge {
    pub id: String,
    pub kind: ArchitectureGraphEdgeKind,
    pub from_node_id: String,
    pub to_node_id: String,
    pub source_kind: String,
    pub confidence: f64,
    pub computed: bool,
    pub asserted: bool,
    pub suppressed: bool,
    pub effective: bool,
    pub provenance: JsonScalar,
    pub computed_provenance: JsonScalar,
    pub asserted_provenance: JsonScalar,
    pub evidence: JsonScalar,
    pub properties: JsonScalar,
    pub annotations: Vec<ArchitectureGraphAssertionSummary>,
}

impl ArchitectureGraphEdge {
    pub(crate) fn assertion(
        id: String,
        kind: ArchitectureGraphEdgeKind,
        from_node_id: String,
        to_node_id: String,
        source_kind: String,
        confidence: f64,
        provenance: Value,
        evidence: Value,
        properties: Value,
    ) -> Self {
        Self {
            id,
            kind,
            from_node_id,
            to_node_id,
            source_kind,
            confidence,
            computed: false,
            asserted: true,
            suppressed: false,
            effective: true,
            provenance: Json(provenance.clone()),
            computed_provenance: Json(Value::Null),
            asserted_provenance: Json(provenance),
            evidence: Json(evidence),
            properties: Json(properties),
            annotations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct ArchitectureGraphFlow {
    pub entry_point: ArchitectureGraphNode,
    pub flow: ArchitectureGraphNode,
    pub traversed_nodes: Vec<ArchitectureGraphNode>,
    pub steps: Vec<ArchitectureGraphFlowStep>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct ArchitectureGraphFlowStep {
    pub ordinal: i32,
    pub module_key: String,
    pub depth: i32,
    pub nodes: Vec<ArchitectureGraphNode>,
    pub predecessor_module_keys: Vec<String>,
    pub edge_kinds: Vec<ArchitectureGraphEdgeKind>,
    pub cyclic: bool,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct ArchitectureGraphRepositoryRef {
    pub repo_id: String,
    pub name: String,
    pub provider: String,
    pub organization: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct ArchitectureContainer {
    pub id: String,
    pub key: Option<String>,
    pub kind: Option<String>,
    pub label: String,
    pub repository: ArchitectureGraphRepositoryRef,
    pub node: ArchitectureGraphNode,
    pub components: Vec<ArchitectureGraphNode>,
    pub deployment_units: Vec<ArchitectureGraphNode>,
    pub entry_points: Vec<ArchitectureGraphNode>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct ArchitectureSystem {
    pub id: String,
    pub key: String,
    pub label: String,
    pub repositories: Vec<ArchitectureGraphRepositoryRef>,
    pub containers: Vec<ArchitectureContainer>,
    pub node: ArchitectureGraphNode,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct ArchitectureGraphAssertionSummary {
    pub id: String,
    pub action: ArchitectureGraphAssertionAction,
    pub target_kind: ArchitectureGraphTargetKind,
    pub reason: String,
    pub source: String,
    pub provenance: JsonScalar,
    pub evidence: JsonScalar,
    pub properties: JsonScalar,
}

#[derive(Debug, Clone, InputObject)]
pub struct AssertArchitectureGraphFactInput {
    pub action: ArchitectureGraphAssertionAction,
    pub target_kind: ArchitectureGraphTargetKind,
    #[graphql(default)]
    pub node: Option<ArchitectureGraphNodeAssertionInput>,
    #[graphql(default)]
    pub edge: Option<ArchitectureGraphEdgeAssertionInput>,
    pub reason: String,
    #[graphql(default)]
    pub source: Option<String>,
    #[graphql(default)]
    pub confidence: Option<f64>,
    #[graphql(default)]
    pub provenance: Option<JsonScalar>,
    #[graphql(default)]
    pub evidence: Option<JsonScalar>,
    #[graphql(default)]
    pub properties: Option<JsonScalar>,
}

#[derive(Debug, Clone, InputObject)]
pub struct ArchitectureGraphNodeAssertionInput {
    #[graphql(default)]
    pub id: Option<String>,
    pub kind: ArchitectureGraphNodeKind,
    #[graphql(default)]
    pub label: Option<String>,
    #[graphql(default)]
    pub artefact_id: Option<String>,
    #[graphql(default)]
    pub symbol_id: Option<String>,
    #[graphql(default)]
    pub path: Option<String>,
    #[graphql(default)]
    pub entry_kind: Option<String>,
}

#[derive(Debug, Clone, InputObject)]
pub struct ArchitectureGraphEdgeAssertionInput {
    #[graphql(default)]
    pub id: Option<String>,
    pub kind: ArchitectureGraphEdgeKind,
    pub from_node_id: String,
    pub to_node_id: String,
}

#[derive(Debug, Clone, InputObject)]
pub struct AssertArchitectureSystemMembershipInput {
    pub system_key: String,
    #[graphql(default)]
    pub system_label: Option<String>,
    #[graphql(default)]
    pub repository: Option<String>,
    #[graphql(default)]
    pub container_id: Option<String>,
    #[graphql(default)]
    pub container_key: Option<String>,
    #[graphql(default)]
    pub container_label: Option<String>,
    #[graphql(default)]
    pub container_kind: Option<String>,
    #[graphql(default)]
    pub deployment_unit_id: Option<String>,
    pub reason: String,
    #[graphql(default)]
    pub source: Option<String>,
    #[graphql(default)]
    pub confidence: Option<f64>,
    #[graphql(default)]
    pub properties: Option<JsonScalar>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct ArchitectureGraphAssertionResult {
    pub success: bool,
    pub assertion_id: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct ArchitectureSystemMembershipAssertionResult {
    pub success: bool,
    pub system_id: String,
    pub container_id: String,
    pub assertion_ids: Vec<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct RevokeArchitectureGraphAssertionResult {
    pub success: bool,
    pub id: String,
    pub revoked: bool,
}
