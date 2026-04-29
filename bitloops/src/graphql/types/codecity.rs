use async_graphql::{Enum, SimpleObject};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCityWorldResult {
    pub capability: String,
    pub stage: String,
    pub status: String,
    pub repo_id: String,
    pub commit_sha: Option<String>,
    pub config_fingerprint: String,
    pub summary: CodeCitySummaryResult,
    pub layout: CodeCityLayoutResult,
    pub boundaries: Vec<CodeCityBoundaryResult>,
    pub macro_graph: Option<CodeCityMacroGraphResult>,
    pub architecture: Option<CodeCityArchitectureReportResult>,
    pub boundary_layouts: Vec<CodeCityBoundaryLayoutSummaryResult>,
    pub buildings: Vec<CodeCityBuildingResult>,
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
    pub max_importance: f64,
    pub max_height: f64,
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
    pub floors: Vec<CodeCityFloorResult>,
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
