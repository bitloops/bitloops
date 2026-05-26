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
    pub commits_total: i32,
    pub commits_processed: i32,
    pub checkpoint_companions_processed: i32,
    pub current_checkpoint_id: Option<String>,
    pub current_commit_sha: Option<String>,
    pub events_inserted: i32,
    pub artefacts_upserted: i32,
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
            commits_total: to_graphql_count(value.commits_total),
            commits_processed: to_graphql_count(value.commits_processed),
            checkpoint_companions_processed: to_graphql_count(
                value.counters.checkpoint_companions_processed,
            ),
            current_checkpoint_id: value.current_checkpoint_id,
            current_commit_sha: value.current_commit_sha,
            events_inserted: to_graphql_count(value.counters.events_inserted),
            artefacts_upserted: to_graphql_count(value.counters.artefacts_upserted),
        }
    }
}

fn to_graphql_count(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
