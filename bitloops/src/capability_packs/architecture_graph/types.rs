use serde::{Deserialize, Serialize};

pub const ARCHITECTURE_GRAPH_CAPABILITY_ID: &str = "architecture_graph";
pub const ARCHITECTURE_GRAPH_CONSUMER_ID: &str = "architecture_graph.snapshot";
pub const ARCHITECTURE_GRAPH_ASSERT_INGESTER_ID: &str = "architecture_graph.assert";
pub const ARCHITECTURE_GRAPH_REVOKE_INGESTER_ID: &str = "architecture_graph.revoke";
pub const ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_INGESTER_ID: &str =
    "architecture_graph.role_adjudication";
pub const ARCHITECTURE_GRAPH_FACT_SYNTHESIS_SLOT: &str = "fact_synthesis";
pub const ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT: &str = "role_adjudication";
pub const ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX: &str =
    "architecture_graph.roles.adjudication";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
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
    pub const fn as_str(self) -> &'static str {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
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
    pub const fn as_str(self) -> &'static str {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ArchitectureGraphAssertionAction {
    Assert,
    Suppress,
    Annotate,
}

impl ArchitectureGraphAssertionAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Assert => "ASSERT",
            Self::Suppress => "SUPPRESS",
            Self::Annotate => "ANNOTATE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ArchitectureGraphTargetKind {
    Node,
    Edge,
}

impl ArchitectureGraphTargetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Node => "NODE",
            Self::Edge => "EDGE",
        }
    }
}
