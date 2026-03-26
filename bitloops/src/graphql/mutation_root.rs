use async_graphql::{Context, Error, ErrorExtensions, InputObject, Object, Result, SimpleObject};

use super::DevqlGraphqlContext;

#[derive(Default)]
pub struct MutationRoot;

#[derive(Debug, Clone, InputObject)]
pub struct IngestInput {
    #[graphql(default = true)]
    pub init: bool,
    #[graphql(default = 500)]
    pub max_checkpoints: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct InitSchemaResult {
    pub success: bool,
    pub repo_identity: String,
    pub repo_id: String,
    pub relational_backend: String,
    pub events_backend: String,
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
    pub init_requested: bool,
    pub checkpoints_processed: i32,
    pub events_inserted: i32,
    pub artefacts_upserted: i32,
    pub checkpoints_without_commit: i32,
    pub temporary_rows_promoted: i32,
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
            init_requested: value.init_requested,
            checkpoints_processed: to_graphql_count(value.checkpoints_processed),
            events_inserted: to_graphql_count(value.events_inserted),
            artefacts_upserted: to_graphql_count(value.artefacts_upserted),
            checkpoints_without_commit: to_graphql_count(value.checkpoints_without_commit),
            temporary_rows_promoted: to_graphql_count(value.temporary_rows_promoted),
            semantic_feature_rows_upserted: to_graphql_count(value.semantic_feature_rows_upserted),
            semantic_feature_rows_skipped: to_graphql_count(value.semantic_feature_rows_skipped),
            symbol_embedding_rows_upserted: to_graphql_count(value.symbol_embedding_rows_upserted),
            symbol_embedding_rows_skipped: to_graphql_count(value.symbol_embedding_rows_skipped),
            symbol_clone_edges_upserted: to_graphql_count(value.symbol_clone_edges_upserted),
            symbol_clone_sources_scored: to_graphql_count(value.symbol_clone_sources_scored),
        }
    }
}

#[Object]
impl MutationRoot {
    async fn init_schema(&self, ctx: &Context<'_>) -> Result<InitSchemaResult> {
        let cfg = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .devql_config()
            .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "initSchema", err))?;
        let summary =
            crate::host::devql::execute_init_schema(&cfg, "GraphQL mutation `initSchema`")
                .await
                .map_err(|err| {
                    operation_error("BACKEND_ERROR", "initialisation", "initSchema", err)
                })?;
        Ok(summary.into())
    }

    async fn ingest(&self, ctx: &Context<'_>, input: IngestInput) -> Result<IngestResult> {
        if input.max_checkpoints < 0 {
            return Err(operation_error(
                "BAD_USER_INPUT",
                "validation",
                "ingest",
                "maxCheckpoints must be zero or greater",
            ));
        }

        let cfg = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .devql_config()
            .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "ingest", err))?;
        let summary =
            crate::host::devql::execute_ingest(&cfg, input.init, input.max_checkpoints as usize)
                .await
                .map_err(|err| operation_error("BACKEND_ERROR", "ingestion", "ingest", err))?;
        Ok(summary.into())
    }
}

fn operation_error(
    code: &'static str,
    kind: &'static str,
    operation: &'static str,
    error: impl std::fmt::Display,
) -> Error {
    Error::new(error.to_string()).extend_with(|_, extensions| {
        extensions.set("code", code);
        extensions.set("kind", kind);
        extensions.set("operation", operation);
    })
}

fn to_graphql_count(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
