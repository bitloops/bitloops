use std::collections::HashMap;

use crate::host::devql::{AnalysisMode, FileRole, TextIndexMode};

#[derive(Debug, Clone, PartialEq)]
pub enum SyncMode {
    Auto,
    Full,
    Paths(Vec<String>),
    Repair,
    Validate,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EffectiveSource {
    Head,
    Index,
    Worktree,
}

impl EffectiveSource {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Index => "index",
            Self::Worktree => "worktree",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DesiredFileState {
    pub(crate) path: String,
    pub(crate) analysis_mode: AnalysisMode,
    pub(crate) file_role: FileRole,
    pub(crate) text_index_mode: TextIndexMode,
    pub(crate) language: String,
    pub(crate) resolved_language: String,
    pub(crate) dialect: Option<String>,
    pub(crate) primary_context_id: Option<String>,
    pub(crate) secondary_context_ids: Vec<String>,
    pub(crate) frameworks: Vec<String>,
    pub(crate) runtime_profile: Option<String>,
    pub(crate) classification_reason: String,
    pub(crate) context_fingerprint: Option<String>,
    pub(crate) extraction_fingerprint: String,
    pub(crate) head_content_id: Option<String>,
    pub(crate) index_content_id: Option<String>,
    pub(crate) worktree_content_id: Option<String>,
    pub(crate) effective_content_id: String,
    pub(crate) effective_source: EffectiveSource,
    pub(crate) exists_in_head: bool,
    pub(crate) exists_in_index: bool,
    pub(crate) exists_in_worktree: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredFileState {
    pub(crate) path: String,
    pub(crate) analysis_mode: AnalysisMode,
    pub(crate) file_role: FileRole,
    pub(crate) text_index_mode: TextIndexMode,
    pub(crate) language: String,
    pub(crate) resolved_language: String,
    pub(crate) dialect: Option<String>,
    pub(crate) primary_context_id: Option<String>,
    pub(crate) secondary_context_ids: Vec<String>,
    pub(crate) frameworks: Vec<String>,
    pub(crate) runtime_profile: Option<String>,
    pub(crate) classification_reason: String,
    pub(crate) context_fingerprint: Option<String>,
    pub(crate) extraction_fingerprint: String,
    pub(crate) effective_content_id: String,
    pub(crate) effective_source: EffectiveSource,
    pub(crate) parser_version: String,
    pub(crate) extractor_version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PathAction {
    Unchanged,
    Added,
    Changed,
    Removed,
}

#[derive(Debug, Clone)]
pub(crate) struct ClassifiedPath {
    pub(crate) path: String,
    pub(crate) action: PathAction,
    pub(crate) desired: Option<DesiredFileState>,
}

#[derive(Debug, Default)]
pub(crate) struct SyncCounters {
    pub(crate) paths_unchanged: usize,
    pub(crate) paths_added: usize,
    pub(crate) paths_changed: usize,
    pub(crate) paths_removed: usize,
    pub(crate) cache_hits: usize,
    pub(crate) cache_misses: usize,
    pub(crate) parse_errors: usize,
}

pub(crate) type DesiredManifest = HashMap<String, DesiredFileState>;
pub(crate) type StoredManifest = HashMap<String, StoredFileState>;
