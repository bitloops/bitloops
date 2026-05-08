use async_graphql::SimpleObject;

use crate::graphql::types::{
    CodeCitySnapshotStatusResult, DateTimeScalar, KnowledgeItem, KnowledgeRelation, TaskObject,
};

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct InitSchemaResult {
    pub success: bool,
    pub repo_identity: String,
    pub repo_id: String,
    pub relational_backend: String,
    pub events_backend: String,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct UpdateCliTelemetryConsentResult {
    pub telemetry: Option<bool>,
    pub needs_prompt: bool,
}

impl From<crate::host::devql::InitSchemaSummary> for InitSchemaResult {
    fn from(value: crate::host::devql::InitSchemaSummary) -> Self {
        Self {
            success: value.success,
            repo_identity: value.repo_identity,
            repo_id: value.repo_id,
            relational_backend: value.relational_backend,
            events_backend: value.events_backend,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct IngestResult {
    pub success: bool,
    pub commits_processed: i32,
    pub checkpoint_companions_processed: i32,
    pub events_inserted: i32,
    pub artefacts_upserted: i32,
    pub semantic_feature_rows_upserted: i32,
    pub semantic_feature_rows_skipped: i32,
    pub symbol_embedding_rows_upserted: i32,
    pub symbol_embedding_rows_skipped: i32,
    pub symbol_clone_edges_upserted: i32,
    pub symbol_clone_sources_scored: i32,
}

impl From<crate::host::devql::IngestionCounters> for IngestResult {
    fn from(value: crate::host::devql::IngestionCounters) -> Self {
        Self {
            success: value.success,
            commits_processed: to_graphql_count(value.commits_processed),
            checkpoint_companions_processed: to_graphql_count(
                value.checkpoint_companions_processed,
            ),
            events_inserted: to_graphql_count(value.events_inserted),
            artefacts_upserted: to_graphql_count(value.artefacts_upserted),
            semantic_feature_rows_upserted: to_graphql_count(value.semantic_feature_rows_upserted),
            semantic_feature_rows_skipped: to_graphql_count(value.semantic_feature_rows_skipped),
            symbol_embedding_rows_upserted: to_graphql_count(value.symbol_embedding_rows_upserted),
            symbol_embedding_rows_skipped: to_graphql_count(value.symbol_embedding_rows_skipped),
            symbol_clone_edges_upserted: to_graphql_count(value.symbol_clone_edges_upserted),
            symbol_clone_sources_scored: to_graphql_count(value.symbol_clone_sources_scored),
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct SyncResult {
    pub success: bool,
    pub mode: String,
    pub parser_version: String,
    pub extractor_version: String,
    pub active_branch: Option<String>,
    pub head_commit_sha: Option<String>,
    pub head_tree_sha: Option<String>,
    pub paths_unchanged: i32,
    pub paths_added: i32,
    pub paths_changed: i32,
    pub paths_removed: i32,
    pub cache_hits: i32,
    pub cache_misses: i32,
    pub parse_errors: i32,
    pub validation: Option<SyncValidationResult>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct EnqueueTaskResult {
    pub task: TaskObject,
    pub merged: bool,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "CodeCityRefreshResult")]
pub struct CodeCityRefreshResultObject {
    pub success: bool,
    pub queued: bool,
    pub run_id: Option<String>,
    pub message: String,
    pub snapshot_status: CodeCitySnapshotStatusResult,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct SyncValidationResult {
    pub valid: bool,
    pub expected_artefacts: i32,
    pub actual_artefacts: i32,
    pub expected_edges: i32,
    pub actual_edges: i32,
    pub missing_artefacts: i32,
    pub stale_artefacts: i32,
    pub mismatched_artefacts: i32,
    pub missing_edges: i32,
    pub stale_edges: i32,
    pub mismatched_edges: i32,
    pub files_with_drift: Vec<SyncValidationFileDriftResult>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct SyncValidationFileDriftResult {
    pub path: String,
    pub missing_artefacts: i32,
    pub stale_artefacts: i32,
    pub mismatched_artefacts: i32,
    pub missing_edges: i32,
    pub stale_edges: i32,
    pub mismatched_edges: i32,
}

impl From<crate::host::devql::SyncSummary> for SyncResult {
    fn from(value: crate::host::devql::SyncSummary) -> Self {
        Self {
            success: value.success,
            mode: value.mode,
            parser_version: value.parser_version,
            extractor_version: value.extractor_version,
            active_branch: value.active_branch,
            head_commit_sha: value.head_commit_sha,
            head_tree_sha: value.head_tree_sha,
            paths_unchanged: to_graphql_count(value.paths_unchanged),
            paths_added: to_graphql_count(value.paths_added),
            paths_changed: to_graphql_count(value.paths_changed),
            paths_removed: to_graphql_count(value.paths_removed),
            cache_hits: to_graphql_count(value.cache_hits),
            cache_misses: to_graphql_count(value.cache_misses),
            parse_errors: to_graphql_count(value.parse_errors),
            validation: value.validation.map(|validation| SyncValidationResult {
                valid: validation.valid,
                expected_artefacts: to_graphql_count(validation.expected_artefacts),
                actual_artefacts: to_graphql_count(validation.actual_artefacts),
                expected_edges: to_graphql_count(validation.expected_edges),
                actual_edges: to_graphql_count(validation.actual_edges),
                missing_artefacts: to_graphql_count(validation.missing_artefacts),
                stale_artefacts: to_graphql_count(validation.stale_artefacts),
                mismatched_artefacts: to_graphql_count(validation.mismatched_artefacts),
                missing_edges: to_graphql_count(validation.missing_edges),
                stale_edges: to_graphql_count(validation.stale_edges),
                mismatched_edges: to_graphql_count(validation.mismatched_edges),
                files_with_drift: validation
                    .files_with_drift
                    .into_iter()
                    .map(|file| SyncValidationFileDriftResult {
                        path: file.path,
                        missing_artefacts: to_graphql_count(file.missing_artefacts),
                        stale_artefacts: to_graphql_count(file.stale_artefacts),
                        mismatched_artefacts: to_graphql_count(file.mismatched_artefacts),
                        missing_edges: to_graphql_count(file.missing_edges),
                        stale_edges: to_graphql_count(file.stale_edges),
                        mismatched_edges: to_graphql_count(file.mismatched_edges),
                    })
                    .collect(),
            }),
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "AddKnowledgeResult")]
pub struct AddKnowledgeMutationResult {
    pub success: bool,
    pub knowledge_item_version_id: String,
    pub item_created: bool,
    pub new_version_created: bool,
    pub knowledge_item: KnowledgeItem,
    pub association: Option<KnowledgeRelation>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "AssociateKnowledgeResult")]
pub struct AssociateKnowledgeMutationResult {
    pub success: bool,
    pub relation: KnowledgeRelation,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "RefreshKnowledgeResult")]
pub struct RefreshKnowledgeMutationResult {
    pub success: bool,
    pub latest_document_version_id: String,
    pub content_changed: bool,
    pub new_version_created: bool,
    pub knowledge_item: KnowledgeItem,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "ApplyMigrationsResult")]
pub struct ApplyMigrationsMutationResult {
    pub success: bool,
    pub migrations_applied: Vec<MigrationRecord>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct MigrationRecord {
    pub pack_id: String,
    pub migration_name: String,
    pub description: String,
    pub applied_at: DateTimeScalar,
}

fn to_graphql_count(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_graphql_count_clamps_large_values() {
        assert_eq!(to_graphql_count(0), 0);
        assert_eq!(to_graphql_count(42), 42);
        assert_eq!(
            to_graphql_count((i32::MAX as usize) + 10),
            i32::MAX,
            "values larger than i32::MAX should clamp"
        );
    }
}
