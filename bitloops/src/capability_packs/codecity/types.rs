use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::host::capability_host::StageResponse;

pub const CODECITY_CAPABILITY_ID: &str = "codecity";
pub const CODECITY_WORLD_STAGE_ID: &str = "codecity_world";

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
    pub buildings: Vec<CodeCityBuilding>,
    pub dependency_arcs: Vec<CodeCityDependencyArc>,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodeCitySummary {
    pub file_count: usize,
    pub artefact_count: usize,
    pub dependency_count: usize,
    pub included_file_count: usize,
    pub excluded_file_count: usize,
    pub max_importance: f64,
    pub max_height: f64,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityDiagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub path: Option<String>,
}

pub fn codecity_current_scope_required_stage_response() -> StageResponse {
    StageResponse::new(
        json!({
            "capability": CODECITY_CAPABILITY_ID,
            "stage": CODECITY_WORLD_STAGE_ID,
            "status": "failed",
            "reason": "codecity_current_scope_required",
        }),
        "codecity_world requires the current repository scope; historical and temporary asOf(...) scopes are not supported in phase 1.",
    )
}

pub fn codecity_source_data_unavailable_stage_response(
    message: impl Into<String>,
) -> StageResponse {
    let message = message.into();
    StageResponse::new(
        json!({
            "capability": CODECITY_CAPABILITY_ID,
            "stage": CODECITY_WORLD_STAGE_ID,
            "status": "failed",
            "reason": "codecity_source_data_unavailable",
        }),
        message,
    )
}
