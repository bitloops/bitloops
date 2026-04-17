use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCurrentStateBatchUpdate {
    pub repo_id: String,
    pub repo_root: std::path::PathBuf,
    pub active_branch: Option<String>,
    pub head_commit_sha: Option<String>,
    pub file_upserts: Vec<crate::host::capability_host::ChangedFile>,
    pub file_removals: Vec<crate::host::capability_host::RemovedFile>,
    pub artefact_upserts: Vec<crate::host::capability_host::ChangedArtefact>,
    pub artefact_removals: Vec<crate::host::capability_host::RemovedArtefact>,
}

impl SyncCurrentStateBatchUpdate {
    pub fn is_empty(&self) -> bool {
        self.file_upserts.is_empty()
            && self.file_removals.is_empty()
            && self.artefact_upserts.is_empty()
            && self.artefact_removals.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncProgressPhase {
    Queued,
    EnsuringSchema,
    InspectingWorkspace,
    BuildingManifest,
    LoadingStoredState,
    ClassifyingPaths,
    RemovingPaths,
    ExtractingPaths,
    MaterialisingPaths,
    RunningGc,
    Complete,
    Failed,
}

impl SyncProgressPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::EnsuringSchema => "ensuring_schema",
            Self::InspectingWorkspace => "inspecting_workspace",
            Self::BuildingManifest => "building_manifest",
            Self::LoadingStoredState => "loading_stored_state",
            Self::ClassifyingPaths => "classifying_paths",
            Self::RemovingPaths => "removing_paths",
            Self::ExtractingPaths => "extracting_paths",
            Self::MaterialisingPaths => "materialising_paths",
            Self::RunningGc => "running_gc",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgressUpdate {
    pub phase: SyncProgressPhase,
    pub current_path: Option<String>,
    pub paths_total: usize,
    pub paths_completed: usize,
    pub paths_remaining: usize,
    pub paths_unchanged: usize,
    pub paths_added: usize,
    pub paths_changed: usize,
    pub paths_removed: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub parse_errors: usize,
}

impl Default for SyncProgressUpdate {
    fn default() -> Self {
        Self {
            phase: SyncProgressPhase::Queued,
            current_path: None,
            paths_total: 0,
            paths_completed: 0,
            paths_remaining: 0,
            paths_unchanged: 0,
            paths_added: 0,
            paths_changed: 0,
            paths_removed: 0,
            cache_hits: 0,
            cache_misses: 0,
            parse_errors: 0,
        }
    }
}

pub trait SyncObserver: Send + Sync {
    fn on_progress(&self, update: SyncProgressUpdate);

    fn on_current_state_batch(&self, _update: SyncCurrentStateBatchUpdate) {}
}

pub(super) fn emit_progress(
    observer: Option<&dyn SyncObserver>,
    phase: SyncProgressPhase,
    current_path: Option<String>,
    counters: &sync::types::SyncCounters,
    paths_total: usize,
    paths_completed: usize,
) {
    if let Some(observer) = observer {
        observer.on_progress(SyncProgressUpdate {
            phase,
            current_path,
            paths_total,
            paths_completed,
            paths_remaining: paths_total.saturating_sub(paths_completed),
            paths_unchanged: counters.paths_unchanged,
            paths_added: counters.paths_added,
            paths_changed: counters.paths_changed,
            paths_removed: counters.paths_removed,
            cache_hits: counters.cache_hits,
            cache_misses: counters.cache_misses,
            parse_errors: counters.parse_errors,
        });
    }
}

pub(super) fn emit_current_state_batch(
    observer: Option<&dyn SyncObserver>,
    update: SyncCurrentStateBatchUpdate,
) {
    if let Some(observer) = observer {
        observer.on_current_state_batch(update);
    }
}
