use async_graphql::{Enum, InputObject, SimpleObject};
use serde::Deserialize;

use super::connection::PageInfo;

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityWorldResult {
    pub capability: String,
    pub stage: String,
    pub status: String,
    pub repo_id: String,
    pub commit_sha: Option<String>,
    pub config_fingerprint: String,
    pub summary: CodeCitySummaryResult,
    pub health: CodeCityHealthOverviewResult,
    pub legends: CodeCityLegendsResult,
    pub layout: CodeCityLayoutResult,
    pub boundaries: Vec<CodeCityBoundaryResult>,
    pub macro_graph: Option<CodeCityMacroGraphResult>,
    pub architecture: Option<CodeCityArchitectureReportResult>,
    pub boundary_layouts: Vec<CodeCityBoundaryLayoutSummaryResult>,
    pub buildings: Vec<CodeCityBuildingResult>,
    pub arcs: Vec<CodeCityRenderArcResult>,
    pub dependency_arcs: Vec<CodeCityDependencyArcResult>,
    pub diagnostics: Vec<CodeCityDiagnosticResult>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityArchitectureResult {
    pub capability: String,
    pub stage: String,
    pub status: String,
    pub repo_id: String,
    pub commit_sha: Option<String>,
    pub config_fingerprint: String,
    pub summary: CodeCityArchitectureStageSummaryResult,
    pub macro_graph: Option<CodeCityMacroGraphResult>,
    pub architecture: CodeCityArchitectureReportResult,
    pub boundaries: Vec<CodeCityBoundaryResult>,
    pub boundary_reports: Vec<CodeCityBoundaryArchitectureReportResult>,
    pub diagnostics: Vec<CodeCityDiagnosticResult>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCitySummaryResult {
    pub file_count: i32,
    pub artefact_count: i32,
    pub dependency_count: i32,
    pub boundary_count: i32,
    pub macro_edge_count: i32,
    pub included_file_count: i32,
    pub excluded_file_count: i32,
    pub unhealthy_floor_count: i32,
    pub insufficient_health_data_count: i32,
    pub coverage_available: bool,
    pub git_history_available: bool,
    pub violation_count: i32,
    pub high_severity_violation_count: i32,
    pub visible_arc_count: i32,
    pub cross_boundary_arc_count: i32,
    pub max_importance: f64,
    pub max_height: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityHealthOverviewResult {
    pub status: String,
    pub analysis_window_months: i32,
    pub generated_at: Option<String>,
    pub confidence: f64,
    pub missing_signals: Vec<String>,
    pub coverage_available: bool,
    pub git_history_available: bool,
    pub weights: CodeCityHealthWeightsResult,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityHealthWeightsResult {
    pub churn: f64,
    pub complexity: f64,
    pub bugs: f64,
    pub coverage: f64,
    pub author_concentration: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityArchitectureStageSummaryResult {
    pub boundary_count: i32,
    pub macro_edge_count: i32,
    pub macro_topology: CodeCityMacroTopologyResult,
    pub primary_pattern: CodeCityArchitecturePatternResult,
    pub mud_warning_count: i32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityLayoutResult {
    pub layout_kind: String,
    pub width: f64,
    pub depth: f64,
    pub gap: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityBuildingResult {
    pub path: String,
    pub language: String,
    pub boundary_id: String,
    pub zone: String,
    pub inferred_zone: Option<String>,
    pub convention_zone: Option<String>,
    pub architecture_role: Option<String>,
    pub importance: CodeCityImportanceResult,
    pub size: CodeCitySizeResult,
    pub geometry: CodeCityGeometryResult,
    pub health_risk: Option<f64>,
    pub health_status: String,
    pub health_confidence: f64,
    pub colour: String,
    pub health_summary: CodeCityBuildingHealthSummaryResult,
    pub diagnostic_badges: Vec<CodeCityDiagnosticBadgeResult>,
    pub floors: Vec<CodeCityFloorResult>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityBuildingHealthSummaryResult {
    pub floor_count: i32,
    pub high_risk_floor_count: i32,
    pub insufficient_data_floor_count: i32,
    pub average_risk: Option<f64>,
    pub max_risk: Option<f64>,
    pub missing_signals: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityImportanceResult {
    pub score: f64,
    pub blast_radius: i32,
    pub weighted_fan_in: f64,
    pub articulation_score: f64,
    pub normalized_blast_radius: f64,
    pub normalized_weighted_fan_in: f64,
    pub normalized_articulation_score: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCitySizeResult {
    pub loc: i64,
    pub artefact_count: i32,
    pub total_height: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityGeometryResult {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub width: f64,
    pub depth: f64,
    pub side_length: f64,
    pub footprint_area: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityFloorResult {
    pub artefact_id: Option<String>,
    pub symbol_id: Option<String>,
    pub name: String,
    pub canonical_kind: Option<String>,
    pub language_kind: Option<String>,
    pub start_line: i64,
    pub end_line: i64,
    pub loc: i64,
    pub floor_index: i32,
    pub floor_height: f64,
    pub health_risk: Option<f64>,
    pub colour: String,
    pub health_status: String,
    pub health_confidence: f64,
    pub health_metrics: CodeCityHealthMetricsResult,
    pub health_evidence: CodeCityHealthEvidenceResult,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityHealthMetricsResult {
    pub churn: i32,
    pub complexity: f64,
    pub bug_count: i32,
    pub coverage: Option<f64>,
    pub author_concentration: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityHealthEvidenceResult {
    pub commits_touching: i32,
    pub bug_fix_commits: i32,
    pub distinct_authors: i32,
    pub covered_lines: Option<i32>,
    pub total_coverable_lines: Option<i32>,
    pub complexity_source: String,
    pub coverage_source: String,
    pub git_history_source: String,
    pub missing_signals: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityDependencyArcResult {
    pub from_path: String,
    pub to_path: String,
    pub edge_count: i32,
    pub arc_kind: String,
    pub severity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityLegendsResult {
    pub arc_kinds: Vec<CodeCityArcKindLegendResult>,
    pub violation_rules: Vec<CodeCityViolationRuleLegendResult>,
    pub severities: Vec<CodeCitySeverityLegendResult>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityArcKindLegendResult {
    pub kind: CodeCityArcKindResult,
    pub label: String,
    pub default_visible: bool,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityViolationRuleLegendResult {
    pub rule: CodeCityViolationRuleResult,
    pub pattern: CodeCityViolationPatternResult,
    pub severity: CodeCityViolationSeverityResult,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCitySeverityLegendResult {
    pub severity: CodeCityViolationSeverityResult,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityDiagnosticBadgeResult {
    pub kind: CodeCityDiagnosticBadgeKindResult,
    pub severity: CodeCityViolationSeverityResult,
    pub count: i32,
    pub tooltip: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityDiagnosticBadgeKindResult {
    ArchitectureViolation,
    CrossBoundaryCoupling,
    CycleParticipant,
    BridgeFile,
    HealthRisk,
    InsufficientData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityViolationSeverityResult {
    High,
    Medium,
    Low,
    Info,
}

impl CodeCityViolationSeverityResult {
    pub(crate) fn as_stage_value(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityViolationPatternResult {
    Layered,
    Hexagonal,
    Modular,
    EventDriven,
    CrossBoundary,
    Cycle,
    Mud,
}

impl CodeCityViolationPatternResult {
    pub(crate) fn as_stage_value(self) -> &'static str {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityViolationRuleResult {
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

impl CodeCityViolationRuleResult {
    pub(crate) fn as_stage_value(self) -> &'static str {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityArcKindResult {
    Dependency,
    Violation,
    CrossBoundary,
    Cycle,
    Bridge,
}

impl CodeCityArcKindResult {
    pub(crate) fn as_stage_value(self) -> &'static str {
        match self {
            Self::Dependency => "dependency",
            Self::Violation => "violation",
            Self::CrossBoundary => "cross_boundary",
            Self::Cycle => "cycle",
            Self::Bridge => "bridge",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityArcVisibilityResult {
    HiddenByDefault,
    VisibleOnSelection,
    VisibleAtMediumZoom,
    VisibleAtWorldZoom,
    AlwaysVisible,
}

impl CodeCityArcVisibilityResult {
    pub(crate) fn as_stage_value(self) -> &'static str {
        match self {
            Self::HiddenByDefault => "hidden_by_default",
            Self::VisibleOnSelection => "visible_on_selection",
            Self::VisibleAtMediumZoom => "visible_at_medium_zoom",
            Self::VisibleAtWorldZoom => "visible_at_world_zoom",
            Self::AlwaysVisible => "always_visible",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityDependencyDirectionResult {
    Incoming,
    Outgoing,
    Both,
}

impl CodeCityDependencyDirectionResult {
    pub(crate) fn as_stage_value(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
            Self::Both => "both",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityArcGeometryResult {
    pub from_x: f64,
    pub from_y: f64,
    pub from_z: f64,
    pub to_x: f64,
    pub to_y: f64,
    pub to_z: f64,
    pub control_y: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityRenderArcResult {
    pub id: String,
    pub kind: CodeCityArcKindResult,
    pub visibility: CodeCityArcVisibilityResult,
    pub severity: Option<CodeCityViolationSeverityResult>,
    pub from_path: Option<String>,
    pub to_path: Option<String>,
    pub from_boundary_id: Option<String>,
    pub to_boundary_id: Option<String>,
    pub source_arc_id: Option<String>,
    pub violation_id: Option<String>,
    pub weight: f64,
    pub label: Option<String>,
    pub tooltip: Option<String>,
    pub geometry: CodeCityArcGeometryResult,
    pub metadata_json: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityFileDependencyArcResult {
    pub arc_id: String,
    pub run_id: String,
    pub commit_sha: Option<String>,
    pub from_path: String,
    pub to_path: String,
    pub from_boundary_id: Option<String>,
    pub to_boundary_id: Option<String>,
    pub from_zone: Option<String>,
    pub to_zone: Option<String>,
    pub edge_count: i32,
    pub import_count: i32,
    pub call_count: i32,
    pub reference_count: i32,
    pub export_count: i32,
    pub inheritance_count: i32,
    pub weight: f64,
    pub cross_boundary: bool,
    pub has_violation: bool,
    pub highest_severity: Option<CodeCityViolationSeverityResult>,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityViolationEvidenceResult {
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

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityViolationResult {
    pub id: String,
    pub run_id: String,
    pub commit_sha: Option<String>,
    pub boundary_id: Option<String>,
    pub boundary_root: Option<String>,
    pub pattern: CodeCityViolationPatternResult,
    pub rule: CodeCityViolationRuleResult,
    pub severity: CodeCityViolationSeverityResult,
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
    pub evidence: Vec<CodeCityViolationEvidenceResult>,
    pub confidence: f64,
    pub suppressed: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityViolationRuleCountResult {
    pub rule: CodeCityViolationRuleResult,
    pub count: i32,
    pub severity: CodeCityViolationSeverityResult,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityViolationSummaryResult {
    pub total: i32,
    pub high: i32,
    pub medium: i32,
    pub low: i32,
    pub info: i32,
    pub by_rule: Vec<CodeCityViolationRuleCountResult>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityViolationConnectionEdgeResult {
    pub node: CodeCityViolationResult,
    pub cursor: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityViolationConnectionResult {
    pub total_count: i32,
    pub edges: Vec<CodeCityViolationConnectionEdgeResult>,
    pub page_info: PageInfo,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityArcConnectionEdgeResult {
    pub node: CodeCityRenderArcResult,
    pub cursor: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityArcConnectionResult {
    pub total_count: i32,
    pub edges: Vec<CodeCityArcConnectionEdgeResult>,
    pub page_info: PageInfo,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityDependencyConnectionEdgeResult {
    pub node: CodeCityFileDependencyArcResult,
    pub cursor: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityDependencyConnectionResult {
    pub total_count: i32,
    pub edges: Vec<CodeCityDependencyConnectionEdgeResult>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityFileArchitectureContextResult {
    pub boundary_id: Option<String>,
    pub boundary_name: Option<String>,
    pub primary_pattern: Option<CodeCityArchitecturePatternResult>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityFileDetailResult {
    pub status: String,
    pub path: String,
    pub building: CodeCityBuildingResult,
    pub architecture_context: CodeCityFileArchitectureContextResult,
    pub incoming_dependencies: CodeCityDependencyConnectionResult,
    pub outgoing_dependencies: CodeCityDependencyConnectionResult,
    pub violations: Vec<CodeCityViolationResult>,
    pub related_arcs: Vec<CodeCityRenderArcResult>,
}

#[derive(Debug, Clone, Default, InputObject)]
pub struct CodeCityViolationFilterInput {
    pub severity: Option<CodeCityViolationSeverityResult>,
    pub severities: Option<Vec<CodeCityViolationSeverityResult>>,
    pub pattern: Option<CodeCityViolationPatternResult>,
    pub rule: Option<CodeCityViolationRuleResult>,
    pub boundary_id: Option<String>,
    pub path: Option<String>,
    pub from_path: Option<String>,
    pub to_path: Option<String>,
    pub include_suppressed: Option<bool>,
}

#[derive(Debug, Clone, Default, InputObject)]
pub struct CodeCityArcFilterInput {
    pub kind: Option<CodeCityArcKindResult>,
    pub visibility: Option<CodeCityArcVisibilityResult>,
    pub severity: Option<CodeCityViolationSeverityResult>,
    pub boundary_id: Option<String>,
    pub path: Option<String>,
    pub direction: Option<CodeCityDependencyDirectionResult>,
    pub include_hidden: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityDiagnosticResult {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub path: Option<String>,
    pub boundary_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityBoundaryResult {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub kind: CodeCityBoundaryKindResult,
    pub ecosystem: Option<String>,
    pub parent_boundary_id: Option<String>,
    pub source: CodeCityBoundarySourceResult,
    pub file_count: i32,
    pub artefact_count: i32,
    pub dependency_count: i32,
    pub entry_points: Vec<CodeCityEntryPointResult>,
    pub shared_library: bool,
    pub atomic: bool,
    pub architecture: Option<CodeCityBoundaryArchitectureSummaryResult>,
    pub layout: Option<CodeCityBoundaryLayoutPreviewResult>,
    pub violation_summary: CodeCityViolationSummaryResult,
    pub diagnostics: Vec<CodeCityDiagnosticResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityBoundaryKindResult {
    Explicit,
    Runtime,
    Implicit,
    RootFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityBoundarySourceResult {
    Manifest,
    WorkspaceManifest,
    EntryPoint,
    CommunityDetection,
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityEntryPointResult {
    pub path: String,
    pub entry_kind: String,
    pub closure_file_count: i32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityBoundaryArchitectureSummaryResult {
    pub primary_pattern: CodeCityArchitecturePatternResult,
    pub primary_score: f64,
    pub secondary_pattern: Option<CodeCityArchitecturePatternResult>,
    pub mud_score: f64,
    pub modularity: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityBoundaryLayoutPreviewResult {
    pub strategy: CodeCityLayoutStrategyResult,
    pub zone_count: i32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityMacroGraphResult {
    pub topology: CodeCityMacroTopologyResult,
    pub boundary_count: i32,
    pub edge_count: i32,
    pub density: f64,
    pub modularity: Option<f64>,
    pub edges: Vec<CodeCityMacroEdgeResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityMacroTopologyResult {
    SingleBoundary,
    Star,
    Layered,
    Federated,
    Tangled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityMacroEdgeResult {
    pub from_boundary_id: String,
    pub to_boundary_id: String,
    pub weight: i32,
    pub file_edge_count: i32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityArchitectureReportResult {
    pub macro_topology: CodeCityMacroTopologyResult,
    pub primary_pattern: CodeCityArchitecturePatternResult,
    pub primary_score: f64,
    pub secondary_pattern: Option<CodeCityArchitecturePatternResult>,
    pub secondary_score: Option<f64>,
    pub mud_score: f64,
    pub mud_warning: bool,
    pub boundary_reports: Vec<CodeCityBoundaryArchitectureReportResult>,
    pub diagnostics: Vec<CodeCityDiagnosticResult>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityBoundaryArchitectureReportResult {
    pub boundary_id: String,
    pub primary_pattern: CodeCityArchitecturePatternResult,
    pub primary_score: f64,
    pub secondary_pattern: Option<CodeCityArchitecturePatternResult>,
    pub secondary_score: Option<f64>,
    pub scores: CodeCityArchitectureScoresResult,
    pub metrics: CodeCityBoundaryGraphMetricsResult,
    pub evidence: Vec<CodeCityArchitectureEvidenceResult>,
    pub diagnostics: Vec<CodeCityDiagnosticResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityArchitecturePatternResult {
    Layered,
    Hexagonal,
    Modular,
    EventDriven,
    PipeAndFilter,
    BallOfMud,
    Unclassified,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject, Default)]
pub struct CodeCityArchitectureScoresResult {
    pub layered: f64,
    pub hexagonal: f64,
    pub modular: f64,
    pub event_driven: f64,
    pub pipe_and_filter: f64,
    pub ball_of_mud: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject, Default)]
pub struct CodeCityBoundaryGraphMetricsResult {
    pub node_count: i32,
    pub edge_count: i32,
    pub density: f64,
    pub cycle_edge_count: i32,
    pub largest_scc_size: i32,
    pub scc_count: i32,
    pub back_edge_ratio: f64,
    pub modularity: f64,
    pub community_count: i32,
    pub max_fan_in: i32,
    pub median_fan_in: f64,
    pub max_fan_out: i32,
    pub median_fan_out: f64,
    pub average_path_length: Option<f64>,
    pub clustering_coefficient: f64,
    pub core_periphery_score: f64,
    pub direct_coupling_ratio: f64,
    pub branching_factor: f64,
    pub longest_path_len: i32,
    pub chain_dominance: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityArchitectureEvidenceResult {
    pub name: String,
    pub value: f64,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Enum)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityLayoutStrategyResult {
    Phase1GridTreemap,
    HexagonalRings,
    LayeredBands,
    ModularIslands,
    PipeAndFilterStrip,
    MudForceDirected,
    PlainTreemap,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityBoundaryLayoutSummaryResult {
    pub boundary_id: String,
    pub strategy: CodeCityLayoutStrategyResult,
    pub zone_count: i32,
    pub width: f64,
    pub depth: f64,
    pub x: f64,
    pub z: f64,
}
