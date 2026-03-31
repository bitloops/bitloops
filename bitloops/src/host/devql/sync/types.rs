use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SyncMode {
    Auto,
    Full,
    Paths(Vec<String>),
    Repair,
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
    pub(crate) language: String,
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
    pub(crate) language: String,
    pub(crate) effective_content_id: String,
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
