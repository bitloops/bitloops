use std::collections::BTreeMap;

use crate::capability_packs::codecity::types::{
    CodeCityBoundary, CodeCityBoundaryKind, CodeCityBoundarySource, CodeCityDiagnostic,
    CodeCityEntryPoint,
};

#[derive(Debug, Clone, PartialEq)]
pub struct CodeCityBoundaryDetectionResult {
    pub boundaries: Vec<CodeCityBoundary>,
    pub file_to_boundary: BTreeMap<String, String>,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedBoundary {
    pub(super) boundary: CodeCityBoundary,
    pub(super) files: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct BoundarySplitResult {
    pub(super) boundaries: Vec<ResolvedBoundary>,
    pub(super) diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone)]
pub(super) struct BoundaryBuildSpec {
    pub(super) root_path: String,
    pub(super) id: String,
    pub(super) name: String,
    pub(super) kind: CodeCityBoundaryKind,
    pub(super) ecosystem: Option<String>,
    pub(super) parent_boundary_id: Option<String>,
    pub(super) source_kind: CodeCityBoundarySource,
    pub(super) files: Vec<String>,
    pub(super) entry_points: Vec<CodeCityEntryPoint>,
    pub(super) diagnostics: Vec<CodeCityDiagnostic>,
}
