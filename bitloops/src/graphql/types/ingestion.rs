use async_graphql::{Enum, SimpleObject};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum IngestionPhase {
    Initializing,
    Extracting,
    Persisting,
    Complete,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct IngestionProgressEvent {
    pub phase: IngestionPhase,
    pub checkpoints_total: i32,
    pub checkpoints_processed: i32,
    pub current_checkpoint_id: Option<String>,
    pub current_checkpoint_sha: Option<String>,
    pub events_inserted: i32,
    pub artefacts_upserted: i32,
    pub checkpoints_without_commit: i32,
    pub temporary_rows_promoted: i32,
}

impl From<crate::host::devql::IngestionProgressUpdate> for IngestionProgressEvent {
    fn from(value: crate::host::devql::IngestionProgressUpdate) -> Self {
        Self {
            phase: match value.phase {
                crate::host::devql::IngestionProgressPhase::Initializing => {
                    IngestionPhase::Initializing
                }
                crate::host::devql::IngestionProgressPhase::Extracting => {
                    IngestionPhase::Extracting
                }
                crate::host::devql::IngestionProgressPhase::Persisting => {
                    IngestionPhase::Persisting
                }
                crate::host::devql::IngestionProgressPhase::Complete => IngestionPhase::Complete,
                crate::host::devql::IngestionProgressPhase::Failed => IngestionPhase::Failed,
            },
            checkpoints_total: to_graphql_count(value.checkpoints_total),
            checkpoints_processed: to_graphql_count(value.checkpoints_processed),
            current_checkpoint_id: value.current_checkpoint_id,
            current_checkpoint_sha: value.current_commit_sha,
            events_inserted: to_graphql_count(value.counters.events_inserted),
            artefacts_upserted: to_graphql_count(value.counters.artefacts_upserted),
            checkpoints_without_commit: to_graphql_count(value.counters.checkpoints_without_commit),
            temporary_rows_promoted: to_graphql_count(value.counters.temporary_rows_promoted),
        }
    }
}

fn to_graphql_count(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
