use async_graphql::{Context, Error, ErrorExtensions, InputObject, Object, Result, SimpleObject};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use super::{
    DevqlGraphqlContext,
    types::{DateTimeScalar, KnowledgeItem, KnowledgeRelation},
};

#[derive(Default)]
pub struct MutationRoot;

#[derive(Debug, Clone, InputObject)]
pub struct IngestInput {
    #[graphql(default = true)]
    pub init: bool,
    #[graphql(default = 500)]
    pub max_checkpoints: i32,
}

#[derive(Debug, Clone, InputObject)]
pub struct AddKnowledgeInput {
    pub url: String,
    pub commit_ref: Option<String>,
}

#[derive(Debug, Clone, InputObject)]
pub struct AssociateKnowledgeInput {
    pub source_ref: String,
    pub target_ref: String,
}

#[derive(Debug, Clone, InputObject)]
pub struct RefreshKnowledgeInput {
    pub knowledge_ref: String,
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

#[derive(Debug, Deserialize)]
struct AddKnowledgeIngesterPayload {
    ingest: crate::capability_packs::knowledge::IngestKnowledgeResult,
    association: Option<crate::capability_packs::knowledge::AssociateKnowledgeResult>,
}

#[derive(Debug, Deserialize)]
struct AssociateKnowledgeIngesterPayload {
    association: crate::capability_packs::knowledge::AssociateKnowledgeResult,
}

#[derive(Debug, Deserialize)]
struct RefreshKnowledgeIngesterPayload {
    refresh: crate::capability_packs::knowledge::RefreshSourceResult,
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

    async fn add_knowledge(
        &self,
        ctx: &Context<'_>,
        input: AddKnowledgeInput,
    ) -> Result<AddKnowledgeMutationResult> {
        let url = require_non_empty_input(input.url, "url", "addKnowledge")?;
        let commit_ref = normalise_optional_input(input.commit_ref, "commitRef", "addKnowledge")?;
        let payload: AddKnowledgeIngesterPayload = execute_knowledge_ingester(
            ctx,
            "addKnowledge",
            "knowledge.add",
            json!({
                "url": url,
                "commit": commit_ref,
            }),
        )
        .await?;

        let knowledge_item =
            load_required_knowledge_item(ctx, "addKnowledge", &payload.ingest.knowledge_item_id)
                .await?;
        let association = match payload.association {
            Some(association) => Some(
                load_required_knowledge_relation(
                    ctx,
                    "addKnowledge",
                    &association.relation_assertion_id,
                )
                .await?,
            ),
            None => None,
        };

        Ok(AddKnowledgeMutationResult {
            success: true,
            knowledge_item_version_id: payload.ingest.knowledge_item_version_id,
            item_created: matches!(
                payload.ingest.item_status,
                crate::capability_packs::knowledge::KnowledgeItemStatus::Created
            ),
            new_version_created: matches!(
                payload.ingest.version_status,
                crate::capability_packs::knowledge::KnowledgeVersionStatus::Created
            ),
            knowledge_item,
            association,
        })
    }

    async fn associate_knowledge(
        &self,
        ctx: &Context<'_>,
        input: AssociateKnowledgeInput,
    ) -> Result<AssociateKnowledgeMutationResult> {
        let source_ref =
            require_non_empty_input(input.source_ref, "sourceRef", "associateKnowledge")?;
        let target_ref =
            require_non_empty_input(input.target_ref, "targetRef", "associateKnowledge")?;
        let payload: AssociateKnowledgeIngesterPayload = execute_knowledge_ingester(
            ctx,
            "associateKnowledge",
            "knowledge.associate",
            json!({
                "source_ref": source_ref,
                "target_ref": target_ref,
            }),
        )
        .await?;
        let relation = load_required_knowledge_relation(
            ctx,
            "associateKnowledge",
            &payload.association.relation_assertion_id,
        )
        .await?;

        Ok(AssociateKnowledgeMutationResult {
            success: true,
            relation,
        })
    }

    async fn refresh_knowledge(
        &self,
        ctx: &Context<'_>,
        input: RefreshKnowledgeInput,
    ) -> Result<RefreshKnowledgeMutationResult> {
        let knowledge_ref =
            require_non_empty_input(input.knowledge_ref, "knowledgeRef", "refreshKnowledge")?;
        let payload: RefreshKnowledgeIngesterPayload = execute_knowledge_ingester(
            ctx,
            "refreshKnowledge",
            "knowledge.refresh",
            json!({
                "knowledge_ref": knowledge_ref,
            }),
        )
        .await?;
        let knowledge_item = load_required_knowledge_item(
            ctx,
            "refreshKnowledge",
            &payload.refresh.knowledge_item_id,
        )
        .await?;

        Ok(RefreshKnowledgeMutationResult {
            success: true,
            latest_document_version_id: payload.refresh.latest_document_version_id,
            content_changed: payload.refresh.content_changed,
            new_version_created: payload.refresh.new_version_created,
            knowledge_item,
        })
    }

    async fn apply_migrations(&self, ctx: &Context<'_>) -> Result<ApplyMigrationsMutationResult> {
        let mut host = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .capability_host_handle()
            .await
            .map_err(|err| {
                operation_error("BACKEND_ERROR", "configuration", "applyMigrations", err)
            })?;

        let report = host.registry_report();
        let pending_migrations = if report.migrations_applied_this_session {
            Vec::new()
        } else {
            report.migration_plan
        };
        host.ensure_migrations_applied_sync()
            .map_err(|err| operation_error("BACKEND_ERROR", "migration", "applyMigrations", err))?;

        let applied_at = DateTimeScalar::from_rfc3339(Utc::now().to_rfc3339())
            .expect("current UTC timestamp must be RFC 3339");
        let migrations_applied = pending_migrations
            .into_iter()
            .map(|migration| MigrationRecord {
                pack_id: migration.capability_id,
                migration_name: migration.version,
                description: migration.description,
                applied_at: applied_at.clone(),
            })
            .collect();

        Ok(ApplyMigrationsMutationResult {
            success: true,
            migrations_applied,
        })
    }
}

async fn execute_knowledge_ingester<T: for<'de> Deserialize<'de>>(
    ctx: &Context<'_>,
    operation: &'static str,
    ingester_name: &'static str,
    payload: serde_json::Value,
) -> Result<T> {
    let mut host = ctx
        .data_unchecked::<DevqlGraphqlContext>()
        .capability_host_handle()
        .await
        .map_err(|err| operation_error("BACKEND_ERROR", "configuration", operation, err))?;
    let result = host
        .invoke_ingester("knowledge", ingester_name, payload)
        .await
        .map_err(|err| map_knowledge_operation_error(operation, err))?;

    serde_json::from_value(result.payload)
        .map_err(|err| operation_error("BACKEND_ERROR", "serialization", operation, err))
}

async fn load_required_knowledge_item(
    ctx: &Context<'_>,
    operation: &'static str,
    knowledge_item_id: &str,
) -> Result<KnowledgeItem> {
    ctx.data_unchecked::<DevqlGraphqlContext>()
        .find_knowledge_item_by_id(knowledge_item_id)
        .await
        .map_err(|err| operation_error("BACKEND_ERROR", "knowledge", operation, err))?
        .ok_or_else(|| {
            operation_error(
                "BACKEND_ERROR",
                "knowledge",
                operation,
                format!(
                    "knowledge item `{knowledge_item_id}` was not available after `{operation}`"
                ),
            )
        })
}

async fn load_required_knowledge_relation(
    ctx: &Context<'_>,
    operation: &'static str,
    relation_assertion_id: &str,
) -> Result<KnowledgeRelation> {
    ctx.data_unchecked::<DevqlGraphqlContext>()
        .find_knowledge_relation_by_id(relation_assertion_id)
        .await
        .map_err(|err| operation_error("BACKEND_ERROR", "knowledge", operation, err))?
        .ok_or_else(|| {
            operation_error(
                "BACKEND_ERROR",
                "knowledge",
                operation,
                format!(
                    "knowledge relation `{relation_assertion_id}` was not available after `{operation}`"
                ),
            )
        })
}

fn require_non_empty_input(
    value: String,
    field: &'static str,
    operation: &'static str,
) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(operation_error(
            "BAD_USER_INPUT",
            "validation",
            operation,
            format!("{field} must not be empty"),
        ));
    }
    Ok(trimmed.to_string())
}

fn normalise_optional_input(
    value: Option<String>,
    field: &'static str,
    operation: &'static str,
) -> Result<Option<String>> {
    value
        .map(|value| require_non_empty_input(value, field, operation))
        .transpose()
}

fn map_knowledge_operation_error(operation: &'static str, error: impl std::fmt::Display) -> Error {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();

    let (code, kind) = if lower.contains("knowledge fetch failed")
        || lower.contains("sending github knowledge request")
        || lower.contains("sending jira knowledge request")
        || lower.contains("sending confluence knowledge request")
        || lower.contains("parsing github knowledge response")
        || lower.contains("parsing jira knowledge response")
        || lower.contains("parsing confluence knowledge response")
        || lower.contains("missing `knowledge.providers")
        || lower.contains("missing atlassian configuration")
    {
        ("BACKEND_ERROR", "provider")
    } else if lower.contains("target ref")
        || lower.contains("source ref")
        || lower.contains("knowledge ref")
        || lower.contains("knowledge item `")
        || lower.contains("knowledge source `")
    {
        ("BAD_USER_INPUT", "reference")
    } else if lower.contains("invalid knowledge url")
        || lower.contains("unsupported knowledge url")
        || lower.contains("must not be empty")
        || lower.contains("does not match configured")
    {
        ("BAD_USER_INPUT", "validation")
    } else {
        ("BACKEND_ERROR", "knowledge")
    };

    operation_error(code, kind, operation, message)
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
