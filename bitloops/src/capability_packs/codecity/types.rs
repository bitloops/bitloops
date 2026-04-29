use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::host::capability_host::StageResponse;

pub const CODECITY_CAPABILITY_ID: &str = "codecity";
pub const CODECITY_WORLD_STAGE_ID: &str = "codecity_world";
pub const CODECITY_BOUNDARIES_STAGE_ID: &str = "codecity_boundaries";
pub const CODECITY_ARCHITECTURE_STAGE_ID: &str = "codecity_architecture";
pub const CODECITY_ROOT_BOUNDARY_ID: &str = "boundary:root";
pub const CODECITY_UNCLASSIFIED_ZONE: &str = "unclassified";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityWorldPayload {
    pub capability: String,
    pub stage: String,
    pub status: String,
    pub repo_id: String,
    pub commit_sha: Option<String>,
    pub config_fingerprint: String,
    pub summary: CodeCitySummary,
    pub layout: CodeCityLayoutSummary,
    pub boundaries: Vec<CodeCityBoundary>,
    pub macro_graph: Option<CodeCityMacroGraph>,
    pub architecture: Option<CodeCityArchitectureReport>,
    pub boundary_layouts: Vec<CodeCityBoundaryLayoutSummary>,
    pub buildings: Vec<CodeCityBuilding>,
    pub dependency_arcs: Vec<CodeCityDependencyArc>,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityArchitecturePayload {
    pub capability: String,
    pub stage: String,
    pub status: String,
    pub repo_id: String,
    pub commit_sha: Option<String>,
    pub config_fingerprint: String,
    pub summary: CodeCityArchitectureStageSummary,
    pub macro_graph: Option<CodeCityMacroGraph>,
    pub architecture: CodeCityArchitectureReport,
    pub boundaries: Vec<CodeCityBoundary>,
    pub boundary_reports: Vec<CodeCityBoundaryArchitectureReport>,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityBoundariesPayload {
    pub capability: String,
    pub stage: String,
    pub status: String,
    pub repo_id: String,
    pub commit_sha: Option<String>,
    pub config_fingerprint: String,
    pub boundaries: Vec<CodeCityBoundary>,
    pub file_to_boundary: Vec<CodeCityFileBoundaryAssignment>,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeCitySummary {
    pub file_count: usize,
    pub artefact_count: usize,
    pub dependency_count: usize,
    pub boundary_count: usize,
    pub macro_edge_count: usize,
    pub included_file_count: usize,
    pub excluded_file_count: usize,
    pub max_importance: f64,
    pub max_height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityArchitectureStageSummary {
    pub boundary_count: usize,
    pub macro_edge_count: usize,
    pub macro_topology: CodeCityMacroTopology,
    pub primary_pattern: CodeCityArchitecturePattern,
    pub mud_warning_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityLayoutSummary {
    pub layout_kind: String,
    pub width: f64,
    pub depth: f64,
    pub gap: f64,
}

impl Default for CodeCityLayoutSummary {
    fn default() -> Self {
        Self {
            layout_kind: "phase1_grid_treemap".to_string(),
            width: 0.0,
            depth: 0.0,
            gap: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityBuilding {
    pub path: String,
    pub language: String,
    pub boundary_id: String,
    pub zone: String,
    pub inferred_zone: Option<String>,
    pub convention_zone: Option<String>,
    pub architecture_role: Option<String>,
    pub importance: CodeCityImportance,
    pub size: CodeCitySize,
    pub geometry: CodeCityGeometry,
    pub floors: Vec<CodeCityFloor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeCityImportance {
    pub score: f64,
    pub blast_radius: usize,
    pub weighted_fan_in: f64,
    pub articulation_score: f64,
    pub normalized_blast_radius: f64,
    pub normalized_weighted_fan_in: f64,
    pub normalized_articulation_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeCitySize {
    pub loc: i64,
    pub artefact_count: usize,
    pub total_height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeCityGeometry {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub width: f64,
    pub depth: f64,
    pub side_length: f64,
    pub footprint_area: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityFloor {
    pub artefact_id: Option<String>,
    pub symbol_id: Option<String>,
    pub name: String,
    pub canonical_kind: Option<String>,
    pub language_kind: Option<String>,
    pub start_line: i64,
    pub end_line: i64,
    pub loc: i64,
    pub floor_index: usize,
    pub floor_height: f64,
    pub health_risk: Option<f64>,
    pub colour: String,
    pub health_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityDependencyArc {
    pub from_path: String,
    pub to_path: String,
    pub edge_count: usize,
    pub arc_kind: String,
    pub severity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeCityDiagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub path: Option<String>,
    pub boundary_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityBoundary {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub kind: CodeCityBoundaryKind,
    pub ecosystem: Option<String>,
    pub parent_boundary_id: Option<String>,
    pub source: CodeCityBoundarySource,
    pub file_count: usize,
    pub artefact_count: usize,
    pub dependency_count: usize,
    pub entry_points: Vec<CodeCityEntryPoint>,
    pub shared_library: bool,
    pub atomic: bool,
    pub architecture: Option<CodeCityBoundaryArchitectureSummary>,
    pub layout: Option<CodeCityBoundaryLayoutPreview>,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityBoundaryKind {
    Explicit,
    Runtime,
    Implicit,
    RootFallback,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityBoundarySource {
    Manifest,
    WorkspaceManifest,
    EntryPoint,
    CommunityDetection,
    Fallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeCityEntryPoint {
    pub path: String,
    pub entry_kind: String,
    pub closure_file_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityMacroGraph {
    pub topology: CodeCityMacroTopology,
    pub boundary_count: usize,
    pub edge_count: usize,
    pub density: f64,
    pub modularity: Option<f64>,
    pub edges: Vec<CodeCityMacroEdge>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityMacroTopology {
    SingleBoundary,
    Star,
    Layered,
    Federated,
    Tangled,
    Unknown,
}

impl CodeCityMacroTopology {
    pub fn as_layout_kind(self) -> &'static str {
        match self {
            Self::SingleBoundary => "phase1_grid_treemap",
            Self::Star => "phase2_star_boundaries",
            Self::Layered => "phase2_layered_boundaries",
            Self::Federated => "phase2_federated_boundaries",
            Self::Tangled => "phase2_tangled_boundaries",
            Self::Unknown => "phase2_boundary_grid",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeCityMacroEdge {
    pub from_boundary_id: String,
    pub to_boundary_id: String,
    pub weight: usize,
    pub file_edge_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityArchitectureReport {
    pub macro_topology: CodeCityMacroTopology,
    pub primary_pattern: CodeCityArchitecturePattern,
    pub primary_score: f64,
    pub secondary_pattern: Option<CodeCityArchitecturePattern>,
    pub secondary_score: Option<f64>,
    pub mud_score: f64,
    pub mud_warning: bool,
    pub boundary_reports: Vec<CodeCityBoundaryArchitectureReport>,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityBoundaryArchitectureReport {
    pub boundary_id: String,
    pub primary_pattern: CodeCityArchitecturePattern,
    pub primary_score: f64,
    pub secondary_pattern: Option<CodeCityArchitecturePattern>,
    pub secondary_score: Option<f64>,
    pub scores: CodeCityArchitectureScores,
    pub metrics: CodeCityBoundaryGraphMetrics,
    pub evidence: Vec<CodeCityArchitectureEvidence>,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityArchitecturePattern {
    Layered,
    Hexagonal,
    Modular,
    EventDriven,
    PipeAndFilter,
    BallOfMud,
    Unclassified,
}

impl CodeCityArchitecturePattern {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Layered => "layered",
            Self::Hexagonal => "hexagonal",
            Self::Modular => "modular",
            Self::EventDriven => "event_driven",
            Self::PipeAndFilter => "pipe_and_filter",
            Self::BallOfMud => "ball_of_mud",
            Self::Unclassified => "unclassified",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeCityArchitectureScores {
    pub layered: f64,
    pub hexagonal: f64,
    pub modular: f64,
    pub event_driven: f64,
    pub pipe_and_filter: f64,
    pub ball_of_mud: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeCityBoundaryGraphMetrics {
    pub node_count: usize,
    pub edge_count: usize,
    pub density: f64,
    pub cycle_edge_count: usize,
    pub largest_scc_size: usize,
    pub scc_count: usize,
    pub back_edge_ratio: f64,
    pub modularity: f64,
    pub community_count: usize,
    pub max_fan_in: usize,
    pub median_fan_in: f64,
    pub max_fan_out: usize,
    pub median_fan_out: f64,
    pub average_path_length: Option<f64>,
    pub clustering_coefficient: f64,
    pub core_periphery_score: f64,
    pub direct_coupling_ratio: f64,
    pub branching_factor: f64,
    pub longest_path_len: usize,
    pub chain_dominance: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityArchitectureEvidence {
    pub name: String,
    pub value: f64,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityBoundaryArchitectureSummary {
    pub primary_pattern: CodeCityArchitecturePattern,
    pub primary_score: f64,
    pub secondary_pattern: Option<CodeCityArchitecturePattern>,
    pub mud_score: f64,
    pub modularity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityBoundaryLayoutPreview {
    pub strategy: CodeCityLayoutStrategy,
    pub zone_count: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityZone {
    Core,
    Application,
    Periphery,
    Edge,
    Ports,
    Shared,
    Unclassified,
}

impl CodeCityZone {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Application => "application",
            Self::Periphery => "periphery",
            Self::Edge => "edge",
            Self::Ports => "ports",
            Self::Shared => "shared",
            Self::Unclassified => CODECITY_UNCLASSIFIED_ZONE,
        }
    }

    pub fn ordered() -> &'static [Self] {
        &[
            Self::Core,
            Self::Application,
            Self::Periphery,
            Self::Ports,
            Self::Edge,
            Self::Shared,
            Self::Unclassified,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityZoneAssignment {
    pub path: String,
    pub boundary_id: String,
    pub zone: CodeCityZone,
    pub convention_zone: Option<CodeCityZone>,
    pub inferred_zone: Option<CodeCityZone>,
    pub depth_score: Option<f64>,
    pub confidence: f64,
    pub disagreement: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeCityLayoutStrategy {
    Phase1GridTreemap,
    HexagonalRings,
    LayeredBands,
    ModularIslands,
    PipeAndFilterStrip,
    MudForceDirected,
    PlainTreemap,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityBoundaryLayoutSummary {
    pub boundary_id: String,
    pub strategy: CodeCityLayoutStrategy,
    pub zone_count: usize,
    pub width: f64,
    pub depth: f64,
    pub x: f64,
    pub z: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeCityFileBoundaryAssignment {
    pub path: String,
    pub boundary_id: String,
}

pub fn codecity_current_scope_required_stage_response(stage_id: &str) -> StageResponse {
    StageResponse::new(
        json!({
            "capability": CODECITY_CAPABILITY_ID,
            "stage": stage_id,
            "status": "failed",
            "reason": "codecity_current_scope_required",
        }),
        format!(
            "{stage_id} requires the current repository scope; historical and temporary asOf(...) scopes are not supported in CodeCity phase 2."
        ),
    )
}

pub fn codecity_source_data_unavailable_stage_response(
    stage_id: &str,
    message: impl Into<String>,
) -> StageResponse {
    let message = message.into();
    StageResponse::new(
        json!({
            "capability": CODECITY_CAPABILITY_ID,
            "stage": stage_id,
            "status": "failed",
            "reason": "codecity_source_data_unavailable",
        }),
        message,
    )
}
