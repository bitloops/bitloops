use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use serde_json::{Map, Value, json};
use tokio_postgres::{NoTls, config::SslMode};

use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::extension_descriptor as semantic_clones_pack;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::capability_packs::semantic_clones::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
    clear_repo_symbol_embedding_rows, load_pre_stage_artefacts_for_blob,
    load_pre_stage_dependencies_for_blob, upsert_semantic_feature_rows,
    upsert_symbol_embedding_rows,
};
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, EventsBackendConfig, RelationalBackendConfig,
    StoreBackendConfig, resolve_embedding_capability_config_for_repo, resolve_store_backend_config,
    resolve_store_backend_config_for_repo,
};
use crate::host::checkpoints::strategy::manual_commit::{
    CommittedInfo, is_missing_head_error, list_committed, read_commit_checkpoint_mappings,
    read_committed, read_session_content, run_git,
};
use crate::host::db_status::{
    DatabaseConnectionStatus, DatabaseStatusRow, classify_connection_error,
};
use crate::host::extension_host::{
    CapabilityIngestContext, CoreExtensionHost, LanguagePackContext, LanguagePackResolutionInput,
};
use crate::host::language_adapter::{LanguageAdapterContext, LanguageAdapterRegistry};
use crate::utils::terminal::print_db_status_table;

#[path = "devql/artefact_sql.rs"]
pub(crate) mod artefact_sql;
#[path = "devql/checkpoint_file_snapshots.rs"]
pub(crate) mod checkpoint_file_snapshots;
#[path = "devql/commands_ingest.rs"]
mod commands_ingest;
#[path = "devql/commands_projection.rs"]
mod commands_projection;
#[path = "devql/commands_query.rs"]
mod commands_query;
#[path = "devql/commands_refresh.rs"]
mod commands_refresh;
#[path = "devql/commands_sync.rs"]
mod commands_sync;
mod connection_status;
pub(crate) mod identity;
#[path = "devql/sync/mod.rs"]
pub(crate) mod sync;
mod types;

pub(crate) use self::commands_ingest::execute_ingest_with_observer;
pub use self::commands_ingest::run_ingest;
#[cfg(test)]
pub(crate) use self::commands_projection::execute_checkpoint_file_snapshot_backfill_with_relational;
pub use self::commands_projection::{
    CheckpointFileSnapshotBackfillOptions, CheckpointFileSnapshotBackfillSummary,
    run_checkpoint_file_snapshot_backfill,
};
pub(crate) use self::commands_query::{
    RegisteredStageCompositionContext, compile_query_document, execute_query_json_with_composition,
    format_query_output, use_raw_graphql_mode,
};
pub use self::commands_query::{execute_query_json_for_repo_root, run_query};
pub use self::commands_refresh::{
    PostCommitArtefactRefreshStats, QueuedSyncTaskMetadata, run_post_checkout_branch_seed,
    run_post_commit_artefact_refresh, run_post_commit_checkpoint_projection_refresh,
    run_post_merge_artefact_refresh,
};
pub use self::commands_sync::{
    SyncObserver, SyncProgressPhase, SyncProgressUpdate, SyncSummary, SyncValidationFileDrift,
    SyncValidationSummary, run_sync, run_sync_with_summary, run_sync_with_summary_and_observer,
};
pub use self::connection_status::run_connection_status;
pub use self::query_dsl_compiler::compile_devql_query_to_graphql;
pub use self::sync::types::SyncMode;
pub use self::types::{DevqlConfig, RelationalDialect, RelationalStorage, RepoIdentity};
pub(crate) use identity::deterministic_uuid;
pub mod watch;

#[cfg(test)]
pub(crate) use self::commands_sync::execute_sync;
#[cfg(test)]
pub(crate) use self::commands_sync::execute_sync_validation;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use self::commands_sync::execute_sync_with_stats;
#[cfg(test)]
pub(crate) use self::connection_status::{
    EVENTS_DUCKDB_LABEL, RELATIONAL_SQLITE_LABEL, collect_connection_status_rows,
};

pub fn build_capability_host(
    repo_root: &Path,
    repo: RepoIdentity,
) -> anyhow::Result<crate::host::capability_host::DevqlCapabilityHost> {
    crate::host::capability_host::DevqlCapabilityHost::builtin(repo_root.to_path_buf(), repo)
}
#[cfg(test)]
const RUST_LANGUAGE_PACK_ID: &str = "rust-language-pack";
#[cfg(test)]
const TS_JS_LANGUAGE_PACK_ID: &str = "ts-js-language-pack";
#[cfg(test)]
const PYTHON_LANGUAGE_PACK_ID: &str = "python-language-pack";
#[cfg(test)]
const GO_LANGUAGE_PACK_ID: &str = "go-language-pack";
#[cfg(test)]
const JAVA_LANGUAGE_PACK_ID: &str = "java-language-pack";
const KNOWLEDGE_CAPABILITY_INGESTER_ID: &str = "knowledge-ingester";
const TEST_HARNESS_CAPABILITY_INGESTER_ID: &str = "test-harness-ingester";
pub(crate) const DEVQL_POSTGRES_DSN_REQUIRED_PREFIX: &str = "DevQL Postgres DSN is required";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitSchemaSummary {
    pub success: bool,
    pub repo_identity: String,
    pub repo_id: String,
    pub relational_backend: String,
    pub events_backend: String,
}

pub(crate) fn format_init_schema_summary(summary: &InitSchemaSummary) -> String {
    format!(
        "DevQL schema ready for repo {} ({})",
        summary.repo_identity, summary.repo_id
    )
}

pub(crate) fn format_ingestion_summary(summary: &IngestionCounters) -> String {
    format!(
        "DevQL ingest complete: checkpoints_processed={}, events_inserted={}, artefacts_upserted={}, checkpoints_without_commit={}, temporary_rows_promoted={}, semantic_feature_rows_upserted={}, semantic_feature_rows_skipped={}, symbol_embedding_rows_upserted={}, symbol_embedding_rows_skipped={}, symbol_clone_edges_upserted={}, symbol_clone_sources_scored={}",
        summary.checkpoints_processed,
        summary.events_inserted,
        summary.artefacts_upserted,
        summary.checkpoints_without_commit,
        summary.temporary_rows_promoted,
        summary.semantic_feature_rows_upserted,
        summary.semantic_feature_rows_skipped,
        summary.symbol_embedding_rows_upserted,
        summary.symbol_embedding_rows_skipped,
        summary.symbol_clone_edges_upserted,
        summary.symbol_clone_sources_scored
    )
}

pub(crate) fn format_checkpoint_file_snapshot_backfill_summary(
    summary: &CheckpointFileSnapshotBackfillSummary,
) -> String {
    format!(
        "Checkpoint file snapshot backfill complete: dry_run={}, checkpoints_scanned={}, checkpoints_processed={}, checkpoints_without_commit={}, rows_projected={}, rows_already_present={}, stale_rows_deleted={}, stale_rows_detected={}, unresolved_files={}, last_checkpoint_id={}",
        summary.dry_run,
        summary.checkpoints_scanned,
        summary.checkpoints_processed,
        summary.checkpoints_without_commit,
        summary.rows_projected,
        summary.rows_already_present,
        summary.stale_rows_deleted,
        summary.stale_rows_detected,
        summary.unresolved_files,
        summary.last_checkpoint_id.as_deref().unwrap_or("-"),
    )
}

fn core_extension_host() -> Result<&'static CoreExtensionHost> {
    static CORE_EXTENSION_HOST: OnceLock<Result<CoreExtensionHost, String>> = OnceLock::new();
    let host_result = CORE_EXTENSION_HOST.get_or_init(|| {
        CoreExtensionHost::with_builtins()
            .map_err(|err| format!("bootstrapping built-in extension packs: {err}"))
    });
    match host_result {
        Ok(host) => Ok(host),
        Err(error_message) => Err(anyhow!(
            "initialising Core extension host for DevQL runtime: {error_message}"
        )),
    }
}

fn language_adapter_registry() -> Result<&'static LanguageAdapterRegistry> {
    static REGISTRY: OnceLock<Result<LanguageAdapterRegistry, String>> = OnceLock::new();
    let registry_result = REGISTRY.get_or_init(|| {
        let packs = crate::adapters::languages::builtin_language_adapter_packs();
        LanguageAdapterRegistry::with_builtins(packs).map_err(|err| err.to_string())
    });
    match registry_result {
        Ok(registry) => Ok(registry),
        Err(error_message) => {
            bail!("failed to initialize language adapter registry: {error_message}")
        }
    }
}

struct LanguageAdapterLifecycleCollection {
    summary: crate::host::capability_host::diagnostics::LanguageAdapterLifecycleSummary,
    readiness_reports: Vec<crate::host::extension_host::ExtensionReadinessReport>,
}

fn extension_readiness_status_label(
    status: crate::host::extension_host::ExtensionReadinessStatus,
) -> String {
    match status {
        crate::host::extension_host::ExtensionReadinessStatus::Ready => "ready".to_string(),
        crate::host::extension_host::ExtensionReadinessStatus::NotReady => "not_ready".to_string(),
    }
}

fn extension_lifecycle_state_label(
    state: crate::host::extension_host::ExtensionLifecycleState,
) -> String {
    match state {
        crate::host::extension_host::ExtensionLifecycleState::Discovered => "discovered",
        crate::host::extension_host::ExtensionLifecycleState::Validated => "validated",
        crate::host::extension_host::ExtensionLifecycleState::Registered => "registered",
        crate::host::extension_host::ExtensionLifecycleState::Migrated => "migrated",
        crate::host::extension_host::ExtensionLifecycleState::Ready => "ready",
        crate::host::extension_host::ExtensionLifecycleState::Failed => "failed",
    }
    .to_string()
}

fn collect_language_adapter_lifecycle(
    cfg: &DevqlConfig,
    runtime: &str,
    apply_migrations: bool,
    with_health: bool,
) -> Result<LanguageAdapterLifecycleCollection> {
    let registry = language_adapter_registry()?;
    if apply_migrations {
        let context =
            LanguageAdapterContext::new(cfg.repo_root.clone(), cfg.repo.repo_id.clone(), None);
        let _ = registry.run_migrations(&context);
    }

    let readiness_reports = registry.readiness_reports(runtime, with_health);
    let readiness = readiness_reports
        .iter()
        .map(|report| {
            crate::host::capability_host::diagnostics::LanguageAdapterReadinessSummary {
                family: report.family.clone(),
                id: report.id.clone(),
                registered: report.registered,
                ready: report.ready,
                status: extension_readiness_status_label(report.status),
                lifecycle_state: extension_lifecycle_state_label(report.lifecycle_state),
                failures: report
                    .failures
                    .iter()
                    .map(|failure| {
                        crate::host::capability_host::diagnostics::LanguageAdapterReadinessFailureSummary {
                            code: failure.code.clone(),
                            message: failure.message.clone(),
                        }
                    })
                    .collect(),
            }
        })
        .collect();

    let packs = registry
        .registered_pack_ids()
        .into_iter()
        .filter_map(|pack_id| {
            registry
                .get(pack_id)
                .map(|pack| (pack_id.to_string(), pack))
        })
        .map(|(pack_id, pack)| {
            let descriptor = pack.descriptor();
            crate::host::capability_host::diagnostics::LanguageAdapterPackRegistryEntry {
                id: pack_id,
                display_name: descriptor.display_name.to_string(),
                version: descriptor.version.to_string(),
                api_version: descriptor.api_version,
                supported_languages: descriptor
                    .supported_languages
                    .iter()
                    .map(|language| (*language).to_string())
                    .collect(),
                migration_count: registry.migration_count_for(descriptor.id),
                health_check_names: registry.health_check_names_for(descriptor.id),
            }
        })
        .collect();

    let migration_plan = registry
        .migration_plan()
        .into_iter()
        .map(|step| {
            crate::host::capability_host::diagnostics::LanguageAdapterMigrationStepSummary {
                pack_id: step.pack_id,
                migration_id: step.migration_id,
                order: step.order,
                description: step.description,
            }
        })
        .collect();

    let applied_migrations = registry
        .applied_migrations()
        .into_iter()
        .map(|execution| {
            crate::host::capability_host::diagnostics::LanguageAdapterMigrationExecutionSummary {
                pack_id: execution.pack_id,
                migration_id: execution.migration_id,
                order: execution.order,
            }
        })
        .collect();

    let health = if with_health {
        registry
            .collect_health_outcomes(runtime)
            .into_iter()
            .map(
                |(check_id, result)| crate::host::capability_host::diagnostics::HealthOutcome {
                    check_id,
                    healthy: result.healthy,
                    message: result.message,
                    details: result.details,
                },
            )
            .collect()
    } else {
        Vec::new()
    };

    Ok(LanguageAdapterLifecycleCollection {
        summary: crate::host::capability_host::diagnostics::LanguageAdapterLifecycleSummary {
            runtime: runtime.to_string(),
            packs,
            migration_plan,
            migrated_pack_ids: registry.migrated_pack_ids(),
            applied_migrations,
            readiness,
            health,
        },
        readiness_reports,
    })
}

fn normalise_optional_commit_sha(commit_sha: Option<&str>) -> Option<String> {
    commit_sha
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn resolve_language_pack_owner_for_input(
    language: &str,
    file_path: Option<&str>,
) -> Option<&'static str> {
    core_extension_host().ok().and_then(|host| {
        if let Some(path) = file_path
            && let Ok(resolved) = host
                .language_packs()
                .resolve(LanguagePackResolutionInput::for_file_path(path))
        {
            return Some(resolved.pack.id);
        }

        let input = file_path
            .map(|path| LanguagePackResolutionInput::for_language(language).with_file_path(path))
            .unwrap_or_else(|| LanguagePackResolutionInput::for_language(language));

        host.language_packs()
            .resolve(input)
            .ok()
            .map(|resolved| resolved.pack.id)
            .or_else(|| {
                host.language_packs()
                    .owner_for_language(language)
                    .and_then(|pack_key| host.language_packs().resolve_pack(pack_key))
                    .map(|descriptor| descriptor.id)
            })
    })
}

fn resolve_language_pack_owner(language: &str) -> Option<&'static str> {
    resolve_language_pack_owner_for_input(language, None)
}

fn resolve_language_id_for_file_path(file_path: &str) -> Option<&'static str> {
    core_extension_host().ok().and_then(|host| {
        host.language_packs()
            .resolve(LanguagePackResolutionInput::for_file_path(file_path))
            .ok()
            .map(|resolved| resolved.profile.language_id)
    })
}

fn language_pack_context_for_language(
    cfg: &DevqlConfig,
    commit_sha: Option<&str>,
    language: &str,
    file_path: Option<&str>,
) -> Result<Option<(LanguagePackContext, &'static str)>> {
    let Some(pack_id) = resolve_language_pack_owner_for_input(language, file_path) else {
        return Ok(None);
    };
    Ok(Some((
        LanguagePackContext::new(
            cfg.repo_root.clone(),
            cfg.repo.repo_id.clone(),
            normalise_optional_commit_sha(commit_sha),
        ),
        pack_id,
    )))
}

fn capability_ingest_context_for_ingester(
    cfg: &DevqlConfig,
    commit_sha: Option<&str>,
    ingester_id: &str,
) -> Result<CapabilityIngestContext> {
    let host = core_extension_host()?;
    let capability_pack_id = host.resolve_ingester_owner_for_ingest(ingester_id)?;
    Ok(CapabilityIngestContext::new(
        cfg.repo_root.clone(),
        cfg.repo.repo_id.clone(),
        normalise_optional_commit_sha(commit_sha),
        capability_pack_id,
        ingester_id,
    ))
}

pub fn run_capability_packs_report(
    cfg: &DevqlConfig,
    json: bool,
    apply_migrations: bool,
    with_health: bool,
    with_extensions: bool,
) -> Result<()> {
    let host = build_capability_host(&cfg.repo_root, cfg.repo.clone())?;
    if apply_migrations {
        host.ensure_migrations_applied_sync()?;
    }
    let runtime = core_extension_host()
        .map(|host| host.compatibility_context().runtime.as_str().to_string())
        .unwrap_or_else(|_| "local-cli".to_string());
    let language_adapter_lifecycle =
        collect_language_adapter_lifecycle(cfg, &runtime, apply_migrations, with_health)?;
    let mut devql_report = host.registry_report();
    devql_report.language_adapters = language_adapter_lifecycle.summary.clone();
    if with_health {
        devql_report.health = crate::host::capability_host::collect_health_outcomes(&host);
    }

    let (core_extension_host, core_extension_host_error) = if with_extensions {
        match crate::host::extension_host::CoreExtensionHost::with_builtins() {
            Ok(ext_host) => {
                let snapshot = ext_host
                    .readiness_snapshot()
                    .with_language_adapter_readiness(
                        language_adapter_lifecycle
                            .summary
                            .packs
                            .iter()
                            .map(|pack| pack.id.clone())
                            .collect(),
                        language_adapter_lifecycle.readiness_reports.clone(),
                    );
                (Some(ext_host.registry_report_with_snapshot(snapshot)), None)
            }
            Err(err) => (None, Some(err.to_string())),
        }
    } else {
        (None, None)
    };

    let combined = crate::host::capability_host::PackLifecycleReport {
        devql_capability_host: devql_report,
        core_extension_host,
        core_extension_host_error,
    };

    if json {
        if with_extensions {
            println!("{}", serde_json::to_string_pretty(&combined)?);
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&combined.devql_capability_host)?
            );
        }
    } else {
        println!(
            "{}",
            crate::host::capability_host::format_pack_lifecycle_report_human(&combined)
        );
    }
    Ok(())
}

async fn init_relational_schema(cfg: &DevqlConfig, relational: &RelationalStorage) -> Result<()> {
    init_relational_schema_with_mode(cfg, relational, RelationalSchemaInitMode::SafeBootstrap)
        .await
        .map(|_| ())
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SyncExecutionSchemaOutcome {
    pub remote_current_state_rebuilt: bool,
}

#[derive(Debug, Clone, Copy)]
enum RelationalSchemaInitMode {
    SafeBootstrap,
    SyncExecution,
}

async fn init_relational_schema_with_mode(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    mode: RelationalSchemaInitMode,
) -> Result<SyncExecutionSchemaOutcome> {
    init_sqlite_schema(&relational.local.path).await?;
    let mut outcome = SyncExecutionSchemaOutcome::default();
    if let Some(remote) = relational.remote.as_ref() {
        let init_outcome = match mode {
            RelationalSchemaInitMode::SafeBootstrap => {
                init_postgres_schema(cfg, &remote.client).await?
            }
            RelationalSchemaInitMode::SyncExecution => {
                init_postgres_schema_for_sync_execution(cfg, &remote.client).await?
            }
        };
        outcome.remote_current_state_rebuilt = init_outcome.rebuilt_current_state;
    }

    let capability_host = build_capability_host(&cfg.repo_root, cfg.repo.clone())?;
    capability_host
        .ensure_migrations_applied_sync()
        .context("running built-in DevQL capability pack migrations")?;

    let test_harness_context =
        capability_ingest_context_for_ingester(cfg, None, TEST_HARNESS_CAPABILITY_INGESTER_ID)
            .context("resolving test-harness capability ingester owner")?;
    crate::capability_packs::test_harness::storage::init_schema_for_repo(&cfg.repo_root)
        .with_context(|| {
            format!(
                "initialising test-harness schema for capability pack `{}`",
                test_harness_context.capability_pack_id
            )
        })?;
    Ok(outcome)
}

fn semantic_provider_config(cfg: &DevqlConfig) -> semantic::SemanticSummaryProviderConfig {
    semantic::SemanticSummaryProviderConfig {
        semantic_provider: cfg.semantic_provider.clone(),
        semantic_model: cfg.semantic_model.clone(),
        semantic_api_key: cfg.semantic_api_key.clone(),
        semantic_base_url: cfg.semantic_base_url.clone(),
    }
}

fn embedding_provider_config(cfg: &DevqlConfig) -> embeddings::EmbeddingProviderConfig {
    let capability = resolve_embedding_capability_config_for_repo(&cfg.config_root);
    embeddings::EmbeddingProviderConfig {
        daemon_config_path: crate::config::default_daemon_config_path()
            .unwrap_or_else(|_| cfg.config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH)),
        embedding_profile: capability.semantic_clones.embedding_profile,
        runtime_command: capability.embeddings.runtime.command,
        runtime_args: capability.embeddings.runtime.args,
        startup_timeout_secs: capability.embeddings.runtime.startup_timeout_secs,
        request_timeout_secs: capability.embeddings.runtime.request_timeout_secs,
        warnings: capability.embeddings.warnings,
    }
}

async fn initialise_devql_schema_for_command(
    cfg: &DevqlConfig,
    command: &str,
) -> Result<(RelationalStorage, InitSchemaSummary)> {
    let (relational, summary, _outcome) = initialise_devql_schema_for_command_with_mode(
        cfg,
        command,
        RelationalSchemaInitMode::SafeBootstrap,
    )
    .await?;
    Ok((relational, summary))
}

async fn initialise_devql_schema_for_command_with_mode(
    cfg: &DevqlConfig,
    command: &str,
    mode: RelationalSchemaInitMode,
) -> Result<(
    RelationalStorage,
    InitSchemaSummary,
    SyncExecutionSchemaOutcome,
)> {
    let backends = resolve_store_backend_config_for_repo(&cfg.config_root)
        .with_context(|| format!("resolving DevQL backend config for `{command}`"))?;
    let relational = RelationalStorage::connect(cfg, &backends.relational, command).await?;

    if backends.events.has_clickhouse() {
        init_clickhouse_schema(cfg).await?;
    } else {
        init_duckdb_schema(&cfg.repo_root, &backends.events).await?;
    }
    let outcome = init_relational_schema_with_mode(cfg, &relational, mode).await?;
    Ok((
        relational,
        InitSchemaSummary {
            success: true,
            repo_identity: cfg.repo.identity.clone(),
            repo_id: cfg.repo.repo_id.clone(),
            relational_backend: if backends.relational.has_postgres() {
                "postgres".to_string()
            } else {
                "sqlite".to_string()
            },
            events_backend: if backends.events.has_clickhouse() {
                "clickhouse".to_string()
            } else {
                "duckdb".to_string()
            },
        },
        outcome,
    ))
}

/// Ensures all DevQL relational and events schemas are up to date.
/// Idempotent and safe to call on every daemon start.
pub async fn ensure_relational_and_events_schema(
    config_root: &Path,
    repo_root: &Path,
    repo: RepoIdentity,
) -> Result<()> {
    let backends = resolve_store_backend_config_for_repo(config_root)
        .context("resolving DevQL backend config for schema bootstrap")?;
    let cfg = DevqlConfig::from_roots(config_root.to_path_buf(), repo_root.to_path_buf(), repo)?;
    let relational =
        RelationalStorage::connect(&cfg, &backends.relational, "daemon schema bootstrap").await?;

    if backends.events.has_clickhouse() {
        init_clickhouse_schema(&cfg).await?;
    } else {
        init_duckdb_schema(repo_root, &backends.events).await?;
    }
    init_relational_schema_with_mode(&cfg, &relational, RelationalSchemaInitMode::SafeBootstrap)
        .await?;
    Ok(())
}

pub(crate) async fn execute_init_schema(
    cfg: &DevqlConfig,
    command: &str,
) -> Result<InitSchemaSummary> {
    let (_relational, summary) = initialise_devql_schema_for_command(cfg, command).await?;
    Ok(summary)
}

pub(crate) async fn prepare_sync_execution_schema(
    cfg: &DevqlConfig,
    command: &str,
    mode: &SyncMode,
) -> Result<SyncExecutionSchemaOutcome> {
    if matches!(mode, SyncMode::Validate) {
        return Ok(SyncExecutionSchemaOutcome::default());
    }
    let (_relational, _summary, outcome) = initialise_devql_schema_for_command_with_mode(
        cfg,
        command,
        RelationalSchemaInitMode::SyncExecution,
    )
    .await?;
    Ok(outcome)
}

pub(crate) fn effective_sync_mode_after_schema_preparation(
    mode: SyncMode,
    outcome: SyncExecutionSchemaOutcome,
) -> SyncMode {
    match mode {
        SyncMode::Paths(_) if outcome.remote_current_state_rebuilt => SyncMode::Repair,
        other => other,
    }
}

pub async fn run_init(cfg: &DevqlConfig) -> Result<()> {
    let summary = execute_init_schema(cfg, "devql init").await?;
    println!("{}", format_init_schema_summary(&summary));
    Ok(())
}

pub async fn execute_project_bootstrap(
    cfg: &DevqlConfig,
    skip_baseline: bool,
) -> Result<InitSchemaSummary> {
    let (relational, summary) = initialise_devql_schema_for_command(cfg, "bitloops init").await?;

    if skip_baseline {
        return Ok(summary);
    }

    run_baseline_ingestion(cfg, &relational).await?;
    Ok(summary)
}

pub async fn run_init_for_bitloops(cfg: &DevqlConfig, skip_baseline: bool) -> Result<()> {
    let summary = execute_project_bootstrap(cfg, skip_baseline).await?;
    println!("{}", format_init_schema_summary(&summary));

    if skip_baseline {
        println!("Baseline ingestion skipped (`--skip-baseline`).");
    }
    Ok(())
}

mod core_contracts;
mod vocab;
// ingestion: shared types
#[path = "devql/ingestion/types.rs"]
mod ingestion_types;
// ingestion: repo identity & git remote parsing
#[path = "devql/ingestion/repo_identity.rs"]
mod ingestion_repo_identity;
// ingestion: database schema DDL
#[path = "devql/ingestion/schema.rs"]
mod ingestion_schema;
// ingestion: language detection & git blob utilities
#[path = "devql/ingestion/language.rs"]
mod ingestion_language;
// ingestion: artefact symbol identity helpers
#[path = "devql/ingestion/artefact_identity.rs"]
mod ingestion_artefact_identity;
// ingestion: checkpoint / commit / event persistence
#[path = "devql/ingestion/checkpoint.rs"]
mod ingestion_checkpoint;
// ingestion: baseline indexing for full tracked codebase at HEAD
#[path = "devql/ingestion/baseline.rs"]
mod ingestion_baseline;
// ingestion: shared record types for artefact persistence
#[path = "devql/ingestion/artefact_persistence_types.rs"]
mod ingestion_artefact_persistence_types;
// ingestion: SQL dialect helpers, JSON utilities, timestamp expressions
#[path = "devql/ingestion/artefact_persistence_sql.rs"]
mod ingestion_artefact_persistence_sql;
// ingestion: file state row, file artefact upsert, revision management
#[path = "devql/ingestion/artefact_persistence_file.rs"]
mod ingestion_artefact_persistence_file;
// ingestion: symbol record building, content hashing, artefact DB upserts
#[path = "devql/ingestion/artefact_persistence_symbols.rs"]
mod ingestion_artefact_persistence_symbols;
// ingestion: edge records, current state queries/mutations, row deserialization
#[path = "devql/ingestion/artefact_persistence_edges.rs"]
mod ingestion_artefact_persistence_edges;
// ingestion: top-level orchestration (refresh/upsert/delete current state)
#[path = "devql/ingestion/artefact_persistence.rs"]
mod ingestion_artefact_persistence;
// Stages 1–2 semantic feature + embedding persistence: `capabilities::semantic_clones::{stage_semantic_features,stage_embeddings}`
// Stage 3 clone persistence: `capabilities::semantic_clones::pipeline`
// ingestion: Rust dependency edge orchestration
#[path = "devql/db_utils.rs"]
mod db_utils;
#[path = "devql/deps_query.rs"]
mod deps_query;
#[path = "devql/query/dsl_compiler.rs"]
mod query_dsl_compiler;
#[path = "devql/query/executor.rs"]
mod query_executor;
#[path = "devql/query/parser.rs"]
mod query_parser;
#[path = "devql/query/utils.rs"]
mod query_utils;

pub(crate) use self::core_contracts::CanonicalKindProjection;
use self::core_contracts::*;
use self::db_utils::*;
pub(crate) use self::db_utils::{
    clickhouse_query_data, duckdb_query_rows_path, duckdb_value_to_json, esc_ch, esc_pg,
    escape_like_pattern, glob_to_sql_like, postgres_exec, sql_like_with_escape,
    sqlite_exec_path_allow_create, sqlite_query_rows_path, sqlite_value_to_json,
};
use self::deps_query::*;
use self::ingestion_artefact_identity::*;
use self::ingestion_artefact_persistence::*;
use self::ingestion_artefact_persistence_edges::*;
use self::ingestion_artefact_persistence_file::*;
use self::ingestion_artefact_persistence_sql::*;
pub(crate) use self::ingestion_artefact_persistence_sql::{sql_json_value, sql_now};
use self::ingestion_artefact_persistence_symbols::*;
use self::ingestion_artefact_persistence_types::*;
use self::ingestion_baseline::*;
use self::ingestion_checkpoint::*;
use self::ingestion_language::*;
pub use self::ingestion_repo_identity::{resolve_repo_id, resolve_repo_identity};
use self::ingestion_schema::*;
pub(crate) use self::ingestion_schema::{
    checkpoint_schema_sql_postgres, checkpoint_schema_sql_sqlite, devql_schema_sql_sqlite,
    knowledge_schema_sql_duckdb, knowledge_schema_sql_sqlite,
};
use self::ingestion_types::*;
pub(crate) use self::ingestion_types::{
    IngestedCheckpointNotification, IngestionCounters, IngestionObserver, IngestionProgressPhase,
    IngestionProgressUpdate,
};
pub(crate) use self::query_dsl_compiler::GraphqlCompileMode;
pub(crate) use self::query_dsl_compiler::compile_devql_to_graphql_with_mode;
use self::query_executor::*;
pub(crate) use self::query_parser::parse_devql_query;
use self::query_parser::*;
pub(crate) use self::query_parser::{AsOfSelector as DevqlAsOfSelector, ParsedDevqlQuery};
pub(crate) use self::query_utils::sql_string_list_pg;
use self::query_utils::*;
use self::vocab::*;
pub(crate) use self::vocab::{
    CallForm, EDGE_KIND_CALLS, EDGE_KIND_EXPORTS, EdgeKind, ExportForm, ImportForm, RefKind,
    Resolution,
};

#[cfg(test)]
use crate::adapters::languages::rust::edges::*;
#[cfg(test)]
use crate::adapters::languages::rust::extraction::*;
#[cfg(test)]
use crate::adapters::languages::ts_js::edges::*;
#[cfg(test)]
use crate::adapters::languages::ts_js::extraction::*;
#[cfg(test)]
pub(crate) use crate::capability_packs::semantic_clones::pipeline::rebuild_symbol_clone_edges;
#[cfg(test)]
use crate::host::language_adapter::EdgeMetadata;
#[cfg(test)]
use crate::host::language_adapter::edges_shared::*;
use crate::host::language_adapter::{DependencyEdge, LanguageArtefact};

#[cfg(test)]
fn symbol_id_for_artefact(item: &LanguageArtefact) -> String {
    structural_symbol_id_for_artefact(item, None)
}

#[cfg(test)]
#[path = "devql/tests/compat_current_state.rs"]
mod compat_current_state;

#[cfg(test)]
pub(crate) use self::compat_current_state::*;

#[cfg(test)]
#[path = "devql/tests/devql_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "devql/tests/identity_tests.rs"]
mod identity_tests;

#[cfg(test)]
#[path = "devql/tests/mapping_tests.rs"]
mod mapping_tests;

#[cfg(test)]
#[path = "devql/tests/core_contract_tests.rs"]
mod core_contract_tests;

#[cfg(test)]
#[path = "devql/tests/cucumber_world.rs"]
mod cucumber_world;

#[cfg(test)]
#[path = "devql/tests/cucumber_steps/mod.rs"]
mod cucumber_steps;

#[cfg(test)]
#[path = "devql/tests/cucumber_bdd.rs"]
mod cucumber_bdd;

#[cfg(test)]
#[path = "devql/tests/knowledge_support.rs"]
mod knowledge_support;

#[cfg(test)]
#[path = "devql/tests/sync_tests.rs"]
mod sync_tests;
