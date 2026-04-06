use async_graphql::{Context, Error, ErrorExtensions, InputObject, Object, Result, SimpleObject};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use super::{
    DevqlGraphqlContext,
    types::{
        Checkpoint, DateTimeScalar, IngestionProgressEvent, KnowledgeItem, KnowledgeRelation,
        SyncTaskObject,
    },
};

#[derive(Default)]
pub struct MutationRoot;

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

#[derive(Debug, Clone, InputObject)]
pub struct SyncInput {
    /// Run a full workspace reconciliation.
    #[graphql(default = false)]
    pub full: bool,
    /// Reconcile only the specified workspace paths (comma-delimited values accepted).
    #[graphql(default)]
    pub paths: Option<Vec<String>>,
    /// Rebuild sync state from the current workspace, ignoring stored manifest trust.
    #[graphql(default = false)]
    pub repair: bool,
    /// Validate current-state tables against a full read-only workspace reconciliation.
    #[graphql(default = false)]
    pub validate: bool,
}

#[derive(Debug, Clone, InputObject)]
pub struct EnqueueSyncInput {
    #[graphql(default = false)]
    pub full: bool,
    #[graphql(default)]
    pub paths: Option<Vec<String>>,
    #[graphql(default = false)]
    pub repair: bool,
    #[graphql(default = false)]
    pub validate: bool,
    #[graphql(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, InputObject)]
pub struct IngestInput {
    #[graphql(default)]
    pub backfill: Option<i32>,
}

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
pub struct EnqueueSyncResult {
    pub task: SyncTaskObject,
    pub merged: bool,
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
    async fn update_cli_telemetry_consent(
        &self,
        ctx: &Context<'_>,
        cli_version: String,
        telemetry: Option<bool>,
    ) -> Result<UpdateCliTelemetryConsentResult> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        context.require_global_write_scope().map_err(|err| {
            operation_error(
                "BAD_USER_INPUT",
                "validation",
                "updateCliTelemetryConsent",
                err,
            )
        })?;

        let cli_version =
            require_non_empty_input(cli_version, "cliVersion", "updateCliTelemetryConsent")?;
        let runtime = crate::daemon::status()
            .await
            .map_err(|err| {
                operation_error(
                    "BACKEND_ERROR",
                    "configuration",
                    "updateCliTelemetryConsent",
                    err,
                )
            })?
            .runtime
            .ok_or_else(|| {
                operation_error(
                    "BACKEND_ERROR",
                    "configuration",
                    "updateCliTelemetryConsent",
                    "Bitloops daemon runtime state is unavailable",
                )
            })?;

        let state = crate::config::update_daemon_telemetry_consent(
            Some(runtime.config_path.as_path()),
            &cli_version,
            telemetry,
        )
        .map_err(|err| {
            operation_error(
                "BACKEND_ERROR",
                "configuration",
                "updateCliTelemetryConsent",
                err,
            )
        })?;

        Ok(UpdateCliTelemetryConsentResult {
            telemetry: state.telemetry,
            needs_prompt: state.needs_prompt,
        })
    }

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

    async fn ingest(&self, ctx: &Context<'_>, input: Option<IngestInput>) -> Result<IngestResult> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        context
            .require_repo_write_scope()
            .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "ingest", err))?;
        let cfg = context
            .devql_config()
            .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "ingest", err))?;
        crate::daemon::require_current_repo_runtime(cfg.repo_root.as_path(), "GraphQL ingest")
            .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "ingest", err))?;
        let observer = GraphqlIngestionObserver::new(context);
        let backfill = match input.and_then(|input| input.backfill) {
            Some(backfill) if backfill <= 0 => {
                return Err(operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    "ingest",
                    "`backfill` must be greater than zero",
                ));
            }
            Some(backfill) => Some(usize::try_from(backfill).map_err(|_| {
                operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    "ingest",
                    "`backfill` must be greater than zero",
                )
            })?),
            None => None,
        };
        let summary = if let Some(backfill) = backfill {
            crate::host::devql::execute_ingest_with_backfill_window(
                &cfg,
                false,
                backfill,
                Some(&observer),
                Some(crate::daemon::shared_enrichment_coordinator()),
            )
            .await
        } else {
            crate::host::devql::execute_ingest_with_observer(
                &cfg,
                false,
                0,
                Some(&observer),
                Some(crate::daemon::shared_enrichment_coordinator()),
            )
            .await
        }
        .map_err(|err| operation_error("BACKEND_ERROR", "ingestion", "ingest", err))?;
        Ok(summary.into())
    }

    async fn sync(&self, ctx: &Context<'_>, input: SyncInput) -> Result<SyncResult> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        context
            .require_repo_write_scope()
            .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "sync", err))?;
        let cfg = context
            .devql_config()
            .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "sync", err))?;

        let mode = resolve_sync_mode_input(
            input.full,
            input.paths,
            input.repair,
            input.validate,
            "sync",
        )?;
        let schema_outcome = crate::host::devql::prepare_sync_execution_schema(
            &cfg,
            "GraphQL mutation `sync`",
            &mode,
        )
        .await
        .map_err(|err| operation_error("BACKEND_ERROR", "initialisation", "sync", err))?;
        let mode =
            crate::host::devql::effective_sync_mode_after_schema_preparation(mode, schema_outcome);

        let summary = crate::host::devql::run_sync_with_summary(&cfg, mode)
            .await
            .map_err(|err| operation_error("BACKEND_ERROR", "sync", "sync", err))?;
        Ok(summary.into())
    }

    #[graphql(name = "enqueueSync")]
    async fn enqueue_sync(
        &self,
        ctx: &Context<'_>,
        input: EnqueueSyncInput,
    ) -> Result<EnqueueSyncResult> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        context
            .require_repo_write_scope()
            .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "enqueueSync", err))?;
        let cfg = context
            .devql_config()
            .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "enqueueSync", err))?;

        let EnqueueSyncInput {
            full,
            paths,
            repair,
            validate,
            source,
        } = input;
        let mode = resolve_sync_mode_input(full, paths, repair, validate, "enqueueSync")?;
        let source = parse_sync_source(source.as_deref())
            .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "enqueueSync", err))?;

        crate::daemon::shared_sync_coordinator().register_subscription_hub(context.subscriptions());
        let queued = crate::daemon::enqueue_sync_for_config(&cfg, source, mode)
            .map_err(|err| operation_error("BACKEND_ERROR", "sync", "enqueueSync", err))?;
        Ok(EnqueueSyncResult {
            task: queued.task.into(),
            merged: queued.merged,
        })
    }

    async fn add_knowledge(
        &self,
        ctx: &Context<'_>,
        input: AddKnowledgeInput,
    ) -> Result<AddKnowledgeMutationResult> {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .require_repo_write_scope()
            .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "addKnowledge", err))?;
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
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .require_repo_write_scope()
            .map_err(|err| {
                operation_error("BAD_USER_INPUT", "validation", "associateKnowledge", err)
            })?;
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
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .require_repo_write_scope()
            .map_err(|err| {
                operation_error("BAD_USER_INPUT", "validation", "refreshKnowledge", err)
            })?;
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
        let host = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .capability_host_arc()
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

fn parse_sync_source(
    raw: Option<&str>,
) -> std::result::Result<crate::daemon::SyncTaskSource, String> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(crate::daemon::SyncTaskSource::ManualCli),
        Some("init") => Ok(crate::daemon::SyncTaskSource::Init),
        Some("manual_cli") | Some("manual-cli") | Some("manual") => {
            Ok(crate::daemon::SyncTaskSource::ManualCli)
        }
        Some("watcher") => Ok(crate::daemon::SyncTaskSource::Watcher),
        Some("post_commit") | Some("post-commit") => Ok(crate::daemon::SyncTaskSource::PostCommit),
        Some("post_merge") | Some("post-merge") => Ok(crate::daemon::SyncTaskSource::PostMerge),
        Some("post_checkout") | Some("post-checkout") => {
            Ok(crate::daemon::SyncTaskSource::PostCheckout)
        }
        Some(other) => Err(format!(
            "unsupported sync source `{other}`; expected one of: init, manual_cli, watcher, post_commit, post_merge, post_checkout"
        )),
    }
}

fn resolve_sync_mode_input(
    full: bool,
    paths: Option<Vec<String>>,
    repair: bool,
    validate: bool,
    operation: &'static str,
) -> Result<crate::host::devql::SyncMode> {
    let selected_modes = usize::from(full)
        + usize::from(paths.is_some())
        + usize::from(repair)
        + usize::from(validate);
    if selected_modes > 1 {
        return Err(operation_error(
            "BAD_USER_INPUT",
            "validation",
            operation,
            "at most one of `full`, `paths`, `repair`, or `validate` may be specified",
        ));
    }

    Ok(if validate {
        crate::host::devql::SyncMode::Validate
    } else if repair {
        crate::host::devql::SyncMode::Repair
    } else if let Some(paths) = paths {
        crate::host::devql::SyncMode::Paths(paths)
    } else if full {
        crate::host::devql::SyncMode::Full
    } else {
        crate::host::devql::SyncMode::Auto
    })
}

async fn execute_knowledge_ingester<T: for<'de> Deserialize<'de>>(
    ctx: &Context<'_>,
    operation: &'static str,
    ingester_name: &'static str,
    payload: serde_json::Value,
) -> Result<T> {
    let host = ctx
        .data_unchecked::<DevqlGraphqlContext>()
        .capability_host_arc()
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

struct GraphqlIngestionObserver {
    repo_name: String,
    context: DevqlGraphqlContext,
}

impl GraphqlIngestionObserver {
    fn new(context: &DevqlGraphqlContext) -> Self {
        Self {
            repo_name: context.repo_name().to_string(),
            context: context.clone(),
        }
    }
}

impl crate::host::devql::IngestionObserver for GraphqlIngestionObserver {
    fn on_progress(&self, update: crate::host::devql::IngestionProgressUpdate) {
        self.context
            .subscriptions()
            .publish_progress(self.repo_name.clone(), IngestionProgressEvent::from(update));
    }

    fn on_checkpoint_ingested(
        &self,
        checkpoint: crate::host::devql::IngestedCheckpointNotification,
    ) {
        self.context.subscriptions().publish_checkpoint(
            self.repo_name.clone(),
            Checkpoint::from_ingested(&checkpoint.checkpoint, checkpoint.commit_sha.as_deref()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sync_source_accepts_default_and_aliases() {
        assert_eq!(
            parse_sync_source(None).expect("default source"),
            crate::daemon::SyncTaskSource::ManualCli
        );
        assert_eq!(
            parse_sync_source(Some("   ")).expect("blank source"),
            crate::daemon::SyncTaskSource::ManualCli
        );
        assert_eq!(
            parse_sync_source(Some("manual")).expect("manual alias"),
            crate::daemon::SyncTaskSource::ManualCli
        );
        assert_eq!(
            parse_sync_source(Some("manual-cli")).expect("manual-cli alias"),
            crate::daemon::SyncTaskSource::ManualCli
        );
        assert_eq!(
            parse_sync_source(Some("init")).expect("init source"),
            crate::daemon::SyncTaskSource::Init
        );
        assert_eq!(
            parse_sync_source(Some("watcher")).expect("watcher source"),
            crate::daemon::SyncTaskSource::Watcher
        );
        assert_eq!(
            parse_sync_source(Some("post-commit")).expect("post-commit source"),
            crate::daemon::SyncTaskSource::PostCommit
        );
        assert_eq!(
            parse_sync_source(Some("post_merge")).expect("post_merge source"),
            crate::daemon::SyncTaskSource::PostMerge
        );
        assert_eq!(
            parse_sync_source(Some("post_checkout")).expect("post_checkout source"),
            crate::daemon::SyncTaskSource::PostCheckout
        );
    }

    #[test]
    fn parse_sync_source_rejects_unknown_values() {
        let err = parse_sync_source(Some("cronjob")).expect_err("unknown source should fail");
        assert!(err.contains("unsupported sync source `cronjob`"));
        assert!(err.contains("manual_cli"));
    }

    #[test]
    fn resolve_sync_mode_input_defaults_to_auto_when_no_selector_is_set() {
        let mode =
            resolve_sync_mode_input(false, None, false, false, "sync").expect("default mode");
        assert_eq!(mode, crate::host::devql::SyncMode::Auto);
    }

    #[test]
    fn resolve_sync_mode_input_rejects_conflicting_selectors() {
        let err = resolve_sync_mode_input(
            true,
            Some(vec!["src/lib.rs".to_string()]),
            false,
            false,
            "enqueueSync",
        )
        .expect_err("conflicting selectors should fail");
        assert!(
            err.message.contains(
                "at most one of `full`, `paths`, `repair`, or `validate` may be specified"
            )
        );
    }

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

    #[test]
    fn require_non_empty_input_trims_and_rejects_blank_values() {
        let value =
            require_non_empty_input("  hello  ".to_string(), "field", "operation").expect("trim");
        assert_eq!(value, "hello");

        let err = require_non_empty_input("   ".to_string(), "field", "operation")
            .expect_err("blank input should fail");
        let message = err.message.clone();
        assert!(message.contains("field must not be empty"));
    }
}
