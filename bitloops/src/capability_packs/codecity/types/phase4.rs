use serde::{Deserialize, Serialize};

use super::{CodeCityArchitecturePattern, CodeCityBuilding, CodeCityDiagnostic};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeCitySnapshotState {
    Missing,
    Queued,
    Running,
    Ready,
    Failed,
}

impl CodeCitySnapshotState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

impl Default for CodeCitySnapshotState {
    fn default() -> Self {
        Self::Missing
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeCitySnapshotStatus {
    pub state: CodeCitySnapshotState,
    pub stale: bool,
    pub repo_id: String,
    pub project_path: Option<String>,
    pub snapshot_key: String,
    pub config_fingerprint: String,
    pub source_generation_seq: Option<u64>,
    pub last_success_generation_seq: Option<u64>,
    pub run_id: Option<String>,
    pub commit_sha: Option<String>,
    pub generated_at: Option<String>,
    pub updated_at: Option<String>,
    pub last_error: Option<String>,
}

impl Default for CodeCitySnapshotStatus {
    fn default() -> Self {
        Self {
            state: CodeCitySnapshotState::Missing,
            stale: false,
            repo_id: String::new(),
            project_path: None,
            snapshot_key: String::new(),
            config_fingerprint: String::new(),
            source_generation_seq: None,
            last_success_generation_seq: None,
            run_id: None,
            commit_sha: None,
            generated_at: None,
            updated_at: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityViolationSeverity {
    High,
    Medium,
    Low,
    Info,
}

impl CodeCityViolationSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        }
    }

    pub fn rank(self) -> usize {
        match self {
            Self::High => 0,
            Self::Medium => 1,
            Self::Low => 2,
            Self::Info => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityViolationPattern {
    Layered,
    Hexagonal,
    Modular,
    EventDriven,
    CrossBoundary,
    Cycle,
    Mud,
}

impl CodeCityViolationPattern {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Layered => "layered",
            Self::Hexagonal => "hexagonal",
            Self::Modular => "modular",
            Self::EventDriven => "event_driven",
            Self::CrossBoundary => "cross_boundary",
            Self::Cycle => "cycle",
            Self::Mud => "mud",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityViolationRule {
    LayeredUpwardDependency,
    LayeredSkippedLayer,
    HexagonalCoreImportsPeriphery,
    HexagonalCoreImportsExternal,
    HexagonalApplicationImportsEdge,
    ModularInternalCrossModuleDependency,
    ModularBroadBridgeFile,
    EventDrivenDirectPeerDependency,
    CrossBoundaryCycle,
    CrossBoundaryHighCoupling,
}

impl CodeCityViolationRule {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LayeredUpwardDependency => "layered_upward_dependency",
            Self::LayeredSkippedLayer => "layered_skipped_layer",
            Self::HexagonalCoreImportsPeriphery => "hexagonal_core_imports_periphery",
            Self::HexagonalCoreImportsExternal => "hexagonal_core_imports_external",
            Self::HexagonalApplicationImportsEdge => "hexagonal_application_imports_edge",
            Self::ModularInternalCrossModuleDependency => {
                "modular_internal_cross_module_dependency"
            }
            Self::ModularBroadBridgeFile => "modular_broad_bridge_file",
            Self::EventDrivenDirectPeerDependency => "event_driven_direct_peer_dependency",
            Self::CrossBoundaryCycle => "cross_boundary_cycle",
            Self::CrossBoundaryHighCoupling => "cross_boundary_high_coupling",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityArcKind {
    Dependency,
    Violation,
    CrossBoundary,
    Cycle,
    Bridge,
}

impl CodeCityArcKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dependency => "dependency",
            Self::Violation => "violation",
            Self::CrossBoundary => "cross_boundary",
            Self::Cycle => "cycle",
            Self::Bridge => "bridge",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityArcVisibility {
    HiddenByDefault,
    VisibleOnSelection,
    VisibleAtMediumZoom,
    VisibleAtWorldZoom,
    AlwaysVisible,
}

impl CodeCityArcVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HiddenByDefault => "hidden_by_default",
            Self::VisibleOnSelection => "visible_on_selection",
            Self::VisibleAtMediumZoom => "visible_at_medium_zoom",
            Self::VisibleAtWorldZoom => "visible_at_world_zoom",
            Self::AlwaysVisible => "always_visible",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityDependencyDirection {
    Incoming,
    Outgoing,
    Both,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityDiagnosticBadgeKind {
    ArchitectureViolation,
    CrossBoundaryCoupling,
    CycleParticipant,
    BridgeFile,
    HealthRisk,
    InsufficientData,
}

impl CodeCityDiagnosticBadgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ArchitectureViolation => "architecture_violation",
            Self::CrossBoundaryCoupling => "cross_boundary_coupling",
            Self::CycleParticipant => "cycle_participant",
            Self::BridgeFile => "bridge_file",
            Self::HealthRisk => "health_risk",
            Self::InsufficientData => "insufficient_data",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityDiagnosticBadge {
    pub kind: CodeCityDiagnosticBadgeKind,
    pub severity: CodeCityViolationSeverity,
    pub count: usize,
    pub tooltip: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeCityLegends {
    pub arc_kinds: Vec<CodeCityArcKindLegend>,
    pub violation_rules: Vec<CodeCityViolationRuleLegend>,
    pub severities: Vec<CodeCitySeverityLegend>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityArcKindLegend {
    pub kind: CodeCityArcKind,
    pub label: String,
    pub default_visible: bool,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityViolationRuleLegend {
    pub rule: CodeCityViolationRule,
    pub pattern: CodeCityViolationPattern,
    pub severity: CodeCityViolationSeverity,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCitySeverityLegend {
    pub severity: CodeCityViolationSeverity,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeCityDependencyEvidence {
    pub evidence_id: String,
    pub run_id: String,
    pub commit_sha: Option<String>,
    pub from_path: String,
    pub to_path: Option<String>,
    pub to_symbol_ref: Option<String>,
    pub from_boundary_id: Option<String>,
    pub to_boundary_id: Option<String>,
    pub from_zone: Option<String>,
    pub to_zone: Option<String>,
    pub from_symbol_id: Option<String>,
    pub from_artefact_id: Option<String>,
    pub to_symbol_id: Option<String>,
    pub to_artefact_id: Option<String>,
    pub edge_id: Option<String>,
    pub edge_kind: String,
    pub language: Option<String>,
    pub start_line: Option<i64>,
    pub end_line: Option<i64>,
    pub metadata_json: String,
    pub resolved: bool,
    pub cross_boundary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityFileDependencyArc {
    pub arc_id: String,
    pub run_id: String,
    pub commit_sha: Option<String>,
    pub from_path: String,
    pub to_path: String,
    pub from_boundary_id: Option<String>,
    pub to_boundary_id: Option<String>,
    pub from_zone: Option<String>,
    pub to_zone: Option<String>,
    pub edge_count: usize,
    pub import_count: usize,
    pub call_count: usize,
    pub reference_count: usize,
    pub export_count: usize,
    pub inheritance_count: usize,
    pub weight: f64,
    pub cross_boundary: bool,
    pub has_violation: bool,
    pub highest_severity: Option<CodeCityViolationSeverity>,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeCityViolationEvidence {
    pub evidence_id: String,
    pub edge_id: Option<String>,
    pub edge_kind: String,
    pub from_symbol_id: Option<String>,
    pub to_symbol_id: Option<String>,
    pub from_artefact_id: Option<String>,
    pub to_artefact_id: Option<String>,
    pub start_line: Option<i64>,
    pub end_line: Option<i64>,
    pub to_symbol_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityArchitectureViolation {
    pub id: String,
    pub run_id: String,
    pub commit_sha: Option<String>,
    pub boundary_id: Option<String>,
    pub boundary_root: Option<String>,
    pub pattern: CodeCityViolationPattern,
    pub rule: CodeCityViolationRule,
    pub severity: CodeCityViolationSeverity,
    pub from_path: String,
    pub to_path: Option<String>,
    pub from_zone: Option<String>,
    pub to_zone: Option<String>,
    pub from_boundary_id: Option<String>,
    pub to_boundary_id: Option<String>,
    pub arc_id: Option<String>,
    pub message: String,
    pub explanation: String,
    pub recommendation: Option<String>,
    pub evidence_ids: Vec<String>,
    pub evidence: Vec<CodeCityViolationEvidence>,
    pub confidence: f64,
    pub suppressed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeCityViolationSummary {
    pub total: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub info: usize,
    pub by_rule: Vec<CodeCityViolationRuleCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityViolationRuleCount {
    pub rule: CodeCityViolationRule,
    pub count: usize,
    pub severity: CodeCityViolationSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityArcGeometry {
    pub from_x: f64,
    pub from_y: f64,
    pub from_z: f64,
    pub to_x: f64,
    pub to_y: f64,
    pub to_z: f64,
    pub control_y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityRenderArc {
    pub id: String,
    pub kind: CodeCityArcKind,
    pub visibility: CodeCityArcVisibility,
    pub severity: Option<CodeCityViolationSeverity>,
    pub from_path: Option<String>,
    pub to_path: Option<String>,
    pub from_boundary_id: Option<String>,
    pub to_boundary_id: Option<String>,
    pub source_arc_id: Option<String>,
    pub violation_id: Option<String>,
    pub weight: f64,
    pub label: Option<String>,
    pub tooltip: Option<String>,
    pub geometry: CodeCityArcGeometry,
    pub metadata_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeCityPhase4Snapshot {
    pub repo_id: String,
    pub run_id: String,
    pub commit_sha: Option<String>,
    pub evidence: Vec<CodeCityDependencyEvidence>,
    pub file_arcs: Vec<CodeCityFileDependencyArc>,
    pub violations: Vec<CodeCityArchitectureViolation>,
    pub render_arcs: Vec<CodeCityRenderArc>,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeCityViolationFilter {
    pub severity: Option<CodeCityViolationSeverity>,
    pub severities: Vec<CodeCityViolationSeverity>,
    pub pattern: Option<CodeCityViolationPattern>,
    pub rule: Option<CodeCityViolationRule>,
    pub boundary_id: Option<String>,
    pub path: Option<String>,
    pub from_path: Option<String>,
    pub to_path: Option<String>,
    pub include_suppressed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeCityArcFilter {
    pub kind: Option<CodeCityArcKind>,
    pub visibility: Option<CodeCityArcVisibility>,
    pub severity: Option<CodeCityViolationSeverity>,
    pub boundary_id: Option<String>,
    pub path: Option<String>,
    pub direction: Option<CodeCityDependencyDirection>,
    pub include_hidden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityDependencyConnectionPayload {
    pub total_count: usize,
    pub edges: Vec<CodeCityDependencyConnectionEdgePayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityDependencyConnectionEdgePayload {
    pub node: CodeCityFileDependencyArc,
    pub cursor: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityFileArchitectureContext {
    pub boundary_id: Option<String>,
    pub boundary_name: Option<String>,
    pub primary_pattern: Option<CodeCityArchitecturePattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityFileDetailPayload {
    pub status: String,
    pub path: String,
    pub snapshot_status: CodeCitySnapshotStatus,
    pub building: Option<CodeCityBuilding>,
    pub architecture_context: CodeCityFileArchitectureContext,
    pub incoming_dependencies: CodeCityDependencyConnectionPayload,
    pub outgoing_dependencies: CodeCityDependencyConnectionPayload,
    pub violations: Vec<CodeCityArchitectureViolation>,
    pub related_arcs: Vec<CodeCityRenderArc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityPageInfo {
    pub has_next_page: bool,
    pub has_previous_page: bool,
    pub start_cursor: Option<String>,
    pub end_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityViolationConnectionPayload {
    pub snapshot_status: CodeCitySnapshotStatus,
    pub total_count: usize,
    pub edges: Vec<CodeCityViolationConnectionEdgePayload>,
    pub page_info: CodeCityPageInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityViolationConnectionEdgePayload {
    pub node: CodeCityArchitectureViolation,
    pub cursor: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityArcConnectionPayload {
    pub snapshot_status: CodeCitySnapshotStatus,
    pub total_count: usize,
    pub edges: Vec<CodeCityArcConnectionEdgePayload>,
    pub page_info: CodeCityPageInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityArcConnectionEdgePayload {
    pub node: CodeCityRenderArc,
    pub cursor: String,
}
