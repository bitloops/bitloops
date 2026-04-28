use async_graphql::SimpleObject;
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
    pub buildings: Vec<CodeCityBuildingResult>,
    pub dependency_arcs: Vec<CodeCityDependencyArcResult>,
    pub diagnostics: Vec<CodeCityDiagnosticResult>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct CodeCitySummaryResult {
    pub file_count: i32,
    pub artefact_count: i32,
    pub dependency_count: i32,
    pub included_file_count: i32,
    pub excluded_file_count: i32,
    pub max_importance: f64,
    pub max_height: f64,
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
}
