use async_graphql::{Context, Error, ErrorExtensions, InputObject, Object, Result, SimpleObject};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;

use super::{
    DevqlGraphqlContext,
    types::{
        DateTimeScalar, KnowledgeItem, KnowledgeRelation, TaskKind, TaskObject,
        TaskQueueControlResultObject,
    },
};

#[derive(Default)]
pub struct MutationRoot;

fn ensure_knowledge_document_schema(repo_root: &Path) -> anyhow::Result<()> {
    let backends = crate::config::resolve_store_backend_config_for_repo(repo_root)?;
    let documents = crate::capability_packs::knowledge::storage::DuckdbKnowledgeDocumentStore::new(
        backends.events.resolve_duckdb_db_path_for_repo(repo_root),
    );
    documents.initialise_schema()
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

#[derive(Debug, Clone, InputObject)]
pub struct EnqueueSyncTaskInput {
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
pub struct EnqueueIngestTaskInput {
    #[graphql(default)]
    pub backfill: Option<i32>,
}

#[derive(Debug, Clone, InputObject)]
pub struct EnqueueTaskInput {
    pub kind: TaskKind,
    #[graphql(default)]
    pub sync: Option<EnqueueSyncTaskInput>,
    #[graphql(default)]
    pub ingest: Option<EnqueueIngestTaskInput>,
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
pub struct EnqueueTaskResult {
    pub task: TaskObject,
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
        let state = crate::config::update_daemon_telemetry_consent(
            Some(context.daemon_config_path().as_path()),
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

    #[graphql(name = "enqueueTask")]
    async fn enqueue_task(
        &self,
        ctx: &Context<'_>,
        input: EnqueueTaskInput,
    ) -> Result<EnqueueTaskResult> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        context
            .require_repo_write_scope()
            .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "enqueueTask", err))?;
        let cfg = context
            .devql_config()
            .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "enqueueTask", err))?;
        let (source, spec) = resolve_enqueue_task_input(input, "enqueueTask")?;

        crate::daemon::shared_devql_task_coordinator()
            .register_subscription_hub(context.subscriptions());
        let queued = crate::daemon::enqueue_task_for_config(&cfg, source, spec)
            .map_err(|err| operation_error("BACKEND_ERROR", "task", "enqueueTask", err))?;
        Ok(EnqueueTaskResult {
            task: queued.task.into(),
            merged: queued.merged,
        })
    }

    #[graphql(name = "pauseTaskQueue")]
    async fn pause_task_queue(
        &self,
        ctx: &Context<'_>,
        reason: Option<String>,
    ) -> Result<TaskQueueControlResultObject> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        context.require_repo_write_scope().map_err(|err| {
            operation_error("BAD_USER_INPUT", "validation", "pauseTaskQueue", err)
        })?;
        let cfg = context.devql_config().map_err(|err| {
            operation_error("BACKEND_ERROR", "configuration", "pauseTaskQueue", err)
        })?;
        let reason = normalise_optional_input(reason, "reason", "pauseTaskQueue")?;
        crate::daemon::pause_devql_tasks(cfg.repo.repo_id.as_str(), reason)
            .map(Into::into)
            .map_err(|err| operation_error("BACKEND_ERROR", "task", "pauseTaskQueue", err))
    }

    #[graphql(name = "resumeTaskQueue")]
    async fn resume_task_queue(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: Option<String>,
    ) -> Result<TaskQueueControlResultObject> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        context.require_repo_write_scope().map_err(|err| {
            operation_error("BAD_USER_INPUT", "validation", "resumeTaskQueue", err)
        })?;
        let cfg = context.devql_config().map_err(|err| {
            operation_error("BACKEND_ERROR", "configuration", "resumeTaskQueue", err)
        })?;
        let requested_repo_id = repo_id
            .map(|value| require_non_empty_input(value, "repoId", "resumeTaskQueue"))
            .transpose()?;
        if let Some(requested_repo_id) = requested_repo_id
            && requested_repo_id != cfg.repo.repo_id
        {
            return Err(operation_error(
                "BAD_USER_INPUT",
                "validation",
                "resumeTaskQueue",
                format!(
                    "repoId `{requested_repo_id}` does not match the current repository `{}`",
                    cfg.repo.repo_id
                ),
            ));
        }

        crate::daemon::resume_devql_tasks(cfg.repo.repo_id.as_str())
            .map(Into::into)
            .map_err(|err| operation_error("BACKEND_ERROR", "task", "resumeTaskQueue", err))
    }

    #[graphql(name = "cancelTask")]
    async fn cancel_task(&self, ctx: &Context<'_>, id: String) -> Result<TaskObject> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        context
            .require_repo_write_scope()
            .map_err(|err| operation_error("BAD_USER_INPUT", "validation", "cancelTask", err))?;
        let cfg = context
            .devql_config()
            .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "cancelTask", err))?;
        let task = crate::daemon::devql_task(id.as_str())
            .map_err(|err| operation_error("BACKEND_ERROR", "task", "cancelTask", err))?
            .ok_or_else(|| {
                operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    "cancelTask",
                    format!("unknown task `{id}`"),
                )
            })?;
        if task.repo_id != cfg.repo.repo_id {
            return Err(operation_error(
                "BAD_USER_INPUT",
                "validation",
                "cancelTask",
                format!(
                    "task `{id}` belongs to repository `{}` and is outside the current repo scope",
                    task.repo_id
                ),
            ));
        }

        crate::daemon::cancel_devql_task(id.as_str())
            .map(Into::into)
            .map_err(|err| operation_error("BACKEND_ERROR", "task", "cancelTask", err))
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
        ensure_knowledge_document_schema(host.repo_root())
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

fn parse_task_source(raw: Option<&str>) -> std::result::Result<crate::daemon::DevqlTaskSource, String> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(crate::daemon::DevqlTaskSource::ManualCli),
        Some("init") => Ok(crate::daemon::DevqlTaskSource::Init),
        Some("manual_cli") | Some("manual-cli") | Some("manual") => {
            Ok(crate::daemon::DevqlTaskSource::ManualCli)
        }
        Some("watcher") => Ok(crate::daemon::DevqlTaskSource::Watcher),
        Some("post_commit") | Some("post-commit") => {
            Ok(crate::daemon::DevqlTaskSource::PostCommit)
        }
        Some("post_merge") | Some("post-merge") => Ok(crate::daemon::DevqlTaskSource::PostMerge),
        Some("post_checkout") | Some("post-checkout") => {
            Ok(crate::daemon::DevqlTaskSource::PostCheckout)
        }
        Some(other) => Err(format!(
            "unsupported task source `{other}`; expected one of: init, manual_cli, watcher, post_commit, post_merge, post_checkout"
        )),
    }
}

fn resolve_enqueue_task_input(
    input: EnqueueTaskInput,
    operation: &'static str,
) -> Result<(crate::daemon::DevqlTaskSource, crate::daemon::DevqlTaskSpec)> {
    match input.kind {
        TaskKind::Sync => {
            let sync = input.sync.ok_or_else(|| {
                operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    operation,
                    "`sync` input is required when kind is SYNC",
                )
            })?;
            if input.ingest.is_some() {
                return Err(operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    operation,
                    "`ingest` must not be provided when kind is SYNC",
                ));
            }
            let mode = resolve_sync_mode_input(
                sync.full,
                sync.paths,
                sync.repair,
                sync.validate,
                operation,
            )?;
            let source = parse_task_source(sync.source.as_deref())
                .map_err(|err| operation_error("BAD_USER_INPUT", "validation", operation, err))?;
            Ok((
                source,
                crate::daemon::DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                    mode: match mode {
                        crate::host::devql::SyncMode::Auto => crate::daemon::SyncTaskMode::Auto,
                        crate::host::devql::SyncMode::Full => crate::daemon::SyncTaskMode::Full,
                        crate::host::devql::SyncMode::Paths(paths) => {
                            crate::daemon::SyncTaskMode::Paths { paths }
                        }
                        crate::host::devql::SyncMode::Repair => {
                            crate::daemon::SyncTaskMode::Repair
                        }
                        crate::host::devql::SyncMode::Validate => {
                            crate::daemon::SyncTaskMode::Validate
                        }
                    },
                }),
            ))
        }
        TaskKind::Ingest => {
            let ingest = input.ingest.ok_or_else(|| {
                operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    operation,
                    "`ingest` input is required when kind is INGEST",
                )
            })?;
            if input.sync.is_some() {
                return Err(operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    operation,
                    "`sync` must not be provided when kind is INGEST",
                ));
            }
            let backfill = match ingest.backfill {
                Some(backfill) if backfill <= 0 => {
                    return Err(operation_error(
                        "BAD_USER_INPUT",
                        "validation",
                        operation,
                        "`backfill` must be greater than zero",
                    ));
                }
                Some(backfill) => Some(usize::try_from(backfill).map_err(|_| {
                    operation_error(
                        "BAD_USER_INPUT",
                        "validation",
                        operation,
                        "`backfill` must be greater than zero",
                    )
                })?),
                None => None,
            };
            Ok((
                crate::daemon::DevqlTaskSource::ManualCli,
                crate::daemon::DevqlTaskSpec::Ingest(crate::daemon::IngestTaskSpec { backfill }),
            ))
        }
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

#[cfg(test)]
mod tests;
