use async_graphql::{Enum, InputObject, SimpleObject, types::Json};
use serde_json::Value;

use super::JsonScalar;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum NavigationPrimitiveKind {
    FileBlob,
    Package,
    Module,
    Symbol,
    DependencyEdge,
    CallEdge,
    DataFlowEdge,
    Entrypoint,
    PublicContract,
    DataObject,
    ConfigKey,
    RuntimeFlow,
    TestCase,
    VerificationCommand,
    PipelineJob,
    DecisionRecord,
    OwnershipRule,
    MetricObservation,
    IntegrationContract,
    SecurityBoundary,
    GlossaryTerm,
}

impl NavigationPrimitiveKind {
    pub(crate) fn as_db(self) -> &'static str {
        match self {
            Self::FileBlob => "FILE_BLOB",
            Self::Package => "PACKAGE",
            Self::Module => "MODULE",
            Self::Symbol => "SYMBOL",
            Self::DependencyEdge => "DEPENDENCY_EDGE",
            Self::CallEdge => "CALL_EDGE",
            Self::DataFlowEdge => "DATA_FLOW_EDGE",
            Self::Entrypoint => "ENTRYPOINT",
            Self::PublicContract => "PUBLIC_CONTRACT",
            Self::DataObject => "DATA_OBJECT",
            Self::ConfigKey => "CONFIG_KEY",
            Self::RuntimeFlow => "RUNTIME_FLOW",
            Self::TestCase => "TEST_CASE",
            Self::VerificationCommand => "VERIFICATION_COMMAND",
            Self::PipelineJob => "PIPELINE_JOB",
            Self::DecisionRecord => "DECISION_RECORD",
            Self::OwnershipRule => "OWNERSHIP_RULE",
            Self::MetricObservation => "METRIC_OBSERVATION",
            Self::IntegrationContract => "INTEGRATION_CONTRACT",
            Self::SecurityBoundary => "SECURITY_BOUNDARY",
            Self::GlossaryTerm => "GLOSSARY_TERM",
        }
    }

    pub(crate) fn from_db(value: &str) -> Option<Self> {
        match value {
            "FILE_BLOB" => Some(Self::FileBlob),
            "PACKAGE" => Some(Self::Package),
            "MODULE" => Some(Self::Module),
            "SYMBOL" => Some(Self::Symbol),
            "DEPENDENCY_EDGE" => Some(Self::DependencyEdge),
            "CALL_EDGE" => Some(Self::CallEdge),
            "DATA_FLOW_EDGE" => Some(Self::DataFlowEdge),
            "ENTRYPOINT" => Some(Self::Entrypoint),
            "PUBLIC_CONTRACT" => Some(Self::PublicContract),
            "DATA_OBJECT" => Some(Self::DataObject),
            "CONFIG_KEY" => Some(Self::ConfigKey),
            "RUNTIME_FLOW" => Some(Self::RuntimeFlow),
            "TEST_CASE" => Some(Self::TestCase),
            "VERIFICATION_COMMAND" => Some(Self::VerificationCommand),
            "PIPELINE_JOB" => Some(Self::PipelineJob),
            "DECISION_RECORD" => Some(Self::DecisionRecord),
            "OWNERSHIP_RULE" => Some(Self::OwnershipRule),
            "METRIC_OBSERVATION" => Some(Self::MetricObservation),
            "INTEGRATION_CONTRACT" => Some(Self::IntegrationContract),
            "SECURITY_BOUNDARY" => Some(Self::SecurityBoundary),
            "GLOSSARY_TERM" => Some(Self::GlossaryTerm),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum NavigationContextViewStatus {
    Fresh,
    Stale,
}

impl NavigationContextViewStatus {
    pub(crate) fn as_db(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
        }
    }

    pub(crate) fn from_db(value: &str) -> Option<Self> {
        match value {
            "fresh" => Some(Self::Fresh),
            "stale" => Some(Self::Stale),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, InputObject)]
pub struct NavigationContextFilterInput {
    #[graphql(default)]
    pub view_id: Option<String>,
    #[graphql(default)]
    pub view_status: Option<NavigationContextViewStatus>,
    #[graphql(default)]
    pub primitive_kind: Option<NavigationPrimitiveKind>,
    #[graphql(default)]
    pub edge_kind: Option<String>,
    #[graphql(default)]
    pub path: Option<String>,
    #[graphql(default)]
    pub source_kind: Option<String>,
}

#[derive(Debug, Clone, InputObject)]
pub struct AcceptNavigationContextViewInput {
    pub view_id: String,
    #[graphql(default)]
    pub expected_current_signature: Option<String>,
    #[graphql(default)]
    pub source: Option<String>,
    #[graphql(default)]
    pub reason: Option<String>,
    #[graphql(default)]
    pub materialised_ref: Option<String>,
}

#[derive(Debug, Clone, InputObject)]
pub struct MaterialiseNavigationContextViewInput {
    pub view_id: String,
    #[graphql(default)]
    pub expected_current_signature: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct NavigationContextSnapshot {
    pub views: Vec<NavigationContextView>,
    pub primitives: Vec<NavigationPrimitive>,
    pub edges: Vec<NavigationEdge>,
    pub total_views: i32,
    pub total_primitives: i32,
    pub total_edges: i32,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct NavigationContextView {
    pub view_id: String,
    pub view_kind: String,
    pub label: String,
    pub view_query_version: String,
    pub dependency_query: JsonScalar,
    pub accepted_signature: String,
    pub current_signature: String,
    pub status: NavigationContextViewStatus,
    pub stale_reason: JsonScalar,
    pub materialised_ref: Option<String>,
    pub last_observed_generation: Option<i32>,
    pub updated_at: String,
    pub dependencies: Vec<NavigationContextViewDependency>,
    pub acceptance_history: Vec<NavigationContextViewAcceptance>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct NavigationContextViewDependency {
    pub view_id: String,
    pub primitive_id: String,
    pub primitive_kind: NavigationPrimitiveKind,
    pub primitive_hash: String,
    pub dependency_role: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct NavigationContextViewAcceptance {
    pub acceptance_id: String,
    pub view_id: String,
    pub previous_accepted_signature: String,
    pub accepted_signature: String,
    pub current_signature: String,
    pub expected_current_signature: Option<String>,
    pub source: String,
    pub reason: Option<String>,
    pub materialised_ref: Option<String>,
    pub accepted_at: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct NavigationPrimitive {
    pub id: String,
    pub kind: NavigationPrimitiveKind,
    pub identity_key: String,
    pub label: String,
    pub path: Option<String>,
    pub artefact_id: Option<String>,
    pub symbol_id: Option<String>,
    pub source_kind: String,
    pub confidence: f64,
    pub primitive_hash: String,
    pub hash_version: String,
    pub properties: JsonScalar,
    pub provenance: JsonScalar,
    pub last_observed_generation: Option<i32>,
    pub updated_at: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct NavigationEdge {
    pub id: String,
    pub kind: String,
    pub from_primitive_id: String,
    pub to_primitive_id: String,
    pub source_kind: String,
    pub confidence: f64,
    pub edge_hash: String,
    pub hash_version: String,
    pub properties: JsonScalar,
    pub provenance: JsonScalar,
    pub last_observed_generation: Option<i32>,
    pub updated_at: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct AcceptNavigationContextViewResult {
    pub success: bool,
    pub acceptance_id: String,
    pub view_id: String,
    pub previous_accepted_signature: String,
    pub accepted_signature: String,
    pub current_signature: String,
    pub status: NavigationContextViewStatus,
    pub source: String,
    pub reason: Option<String>,
    pub materialised_ref: Option<String>,
    pub accepted_at: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct MaterialiseNavigationContextViewResult {
    pub success: bool,
    pub materialisation_id: String,
    pub materialised_ref: String,
    pub view_id: String,
    pub view_kind: String,
    pub label: String,
    pub accepted_signature: String,
    pub current_signature: String,
    pub status: NavigationContextViewStatus,
    pub materialisation_format: String,
    pub materialisation_version: String,
    pub payload: JsonScalar,
    pub rendered_text: String,
    pub primitive_count: i32,
    pub edge_count: i32,
    pub materialised_at: String,
}

impl NavigationContextSnapshot {
    pub(crate) fn new(
        views: Vec<NavigationContextView>,
        primitives: Vec<NavigationPrimitive>,
        edges: Vec<NavigationEdge>,
        total_views: usize,
        total_primitives: usize,
        total_edges: usize,
    ) -> Self {
        Self {
            views,
            primitives,
            edges,
            total_views: count(total_views),
            total_primitives: count(total_primitives),
            total_edges: count(total_edges),
        }
    }
}

impl AcceptNavigationContextViewResult {
    pub(crate) fn from_acceptance(
        acceptance: crate::capability_packs::navigation_context::storage::NavigationViewAcceptance,
    ) -> Self {
        Self {
            success: true,
            acceptance_id: acceptance.acceptance_id,
            view_id: acceptance.view_id,
            previous_accepted_signature: acceptance.previous_accepted_signature,
            accepted_signature: acceptance.accepted_signature,
            current_signature: acceptance.current_signature,
            status: NavigationContextViewStatus::from_db(&acceptance.status)
                .unwrap_or(NavigationContextViewStatus::Fresh),
            source: acceptance.source,
            reason: acceptance.reason,
            materialised_ref: acceptance.materialised_ref,
            accepted_at: acceptance.accepted_at,
        }
    }
}

impl MaterialiseNavigationContextViewResult {
    pub(crate) fn from_materialisation(
        materialisation: crate::capability_packs::navigation_context::storage::NavigationViewMaterialisation,
    ) -> Self {
        Self {
            success: true,
            materialisation_id: materialisation.materialisation_id,
            materialised_ref: materialisation.materialised_ref,
            view_id: materialisation.view_id,
            view_kind: materialisation.view_kind,
            label: materialisation.label,
            accepted_signature: materialisation.accepted_signature,
            current_signature: materialisation.current_signature,
            status: NavigationContextViewStatus::from_db(&materialisation.status)
                .unwrap_or(NavigationContextViewStatus::Fresh),
            materialisation_format: materialisation.materialisation_format,
            materialisation_version: materialisation.materialisation_version,
            payload: json_scalar(materialisation.payload),
            rendered_text: materialisation.rendered_text,
            primitive_count: materialisation.primitive_count,
            edge_count: materialisation.edge_count,
            materialised_at: materialisation.materialised_at,
        }
    }
}

pub(crate) fn json_scalar(value: Value) -> JsonScalar {
    Json(value)
}

fn count(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
