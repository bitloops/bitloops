#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub success: bool,
    pub mode: String,
    pub parser_version: String,
    pub extractor_version: String,
    pub active_branch: Option<String>,
    pub head_commit_sha: Option<String>,
    pub head_tree_sha: Option<String>,
    pub paths_unchanged: usize,
    pub paths_added: usize,
    pub paths_changed: usize,
    pub paths_removed: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub parse_errors: usize,
    pub validation: Option<SyncValidationSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncValidationSummary {
    pub valid: bool,
    pub expected_artefacts: usize,
    pub actual_artefacts: usize,
    pub expected_edges: usize,
    pub actual_edges: usize,
    pub missing_artefacts: usize,
    pub stale_artefacts: usize,
    pub mismatched_artefacts: usize,
    pub missing_edges: usize,
    pub stale_edges: usize,
    pub mismatched_edges: usize,
    pub files_with_drift: Vec<SyncValidationFileDrift>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncValidationFileDrift {
    pub path: String,
    pub missing_artefacts: usize,
    pub stale_artefacts: usize,
    pub mismatched_artefacts: usize,
    pub missing_edges: usize,
    pub stale_edges: usize,
    pub mismatched_edges: usize,
}
