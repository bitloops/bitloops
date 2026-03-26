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
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_REBUILD_INGESTER_ID,
    load_pre_stage_artefacts_for_blob, load_pre_stage_dependencies_for_blob,
    upsert_semantic_feature_rows, upsert_symbol_embedding_rows,
};
use crate::config::{
    EventsBackendConfig, RelationalBackendConfig, StoreBackendConfig, resolve_store_backend_config,
    resolve_store_backend_config_for_repo, resolve_store_embedding_config,
    resolve_store_semantic_config,
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
use crate::utils::terminal::print_db_status_table;

#[path = "devql/commands_ingest.rs"]
mod commands_ingest;
#[path = "devql/commands_query.rs"]
mod commands_query;
#[path = "devql/commands_refresh.rs"]
mod commands_refresh;
mod connection_status;
pub(crate) mod identity;
mod types;

pub(crate) use self::commands_ingest::execute_ingest;
pub use self::commands_ingest::run_ingest;
pub(crate) use self::commands_query::{
    RegisteredStageCompositionContext, execute_query_json_with_composition,
};
pub use self::commands_query::{execute_query_json_for_repo_root, run_query};
pub use self::commands_refresh::{
    PostCommitArtefactRefreshStats, run_post_checkout_branch_seed,
    run_post_commit_artefact_refresh, run_post_merge_artefact_refresh,
};
pub use self::connection_status::run_connection_status;
pub use self::types::{DevqlConfig, RelationalDialect, RelationalStorage, RepoIdentity};
pub(crate) use identity::deterministic_uuid;
pub mod watch;

#[cfg(test)]
pub(crate) use self::commands_ingest::promote_temporary_current_rows_for_head_commit;
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
const RUST_LANGUAGE_PACK_ID: &str = "rust-language-pack";
const TS_JS_LANGUAGE_PACK_ID: &str = "ts-js-language-pack";
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
    let mut host = build_capability_host(&cfg.repo_root, cfg.repo.clone())?;
    if apply_migrations {
        host.ensure_migrations_applied_sync()?;
    }
    let mut devql_report = host.registry_report();
    if with_health {
        devql_report.health = crate::host::capability_host::collect_health_outcomes(&host);
    }

    let (core_extension_host, core_extension_host_error) = if with_extensions {
        match crate::host::extension_host::CoreExtensionHost::with_builtins() {
            Ok(ext_host) => (Some(ext_host.registry_report()), None),
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
    init_sqlite_schema(&relational.local.path).await?;
    if let Some(remote) = relational.remote.as_ref() {
        init_postgres_schema(cfg, &remote.client).await?;
    }

    let mut capability_host = build_capability_host(&cfg.repo_root, cfg.repo.clone())?;
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
    Ok(())
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
    embeddings::EmbeddingProviderConfig {
        embedding_provider: cfg.embedding_provider.clone(),
        embedding_model: cfg.embedding_model.clone(),
        embedding_api_key: cfg.embedding_api_key.clone(),
    }
}

async fn initialise_devql_schema_for_command(
    cfg: &DevqlConfig,
    command: &str,
) -> Result<(RelationalStorage, InitSchemaSummary)> {
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .with_context(|| format!("resolving DevQL backend config for `{command}`"))?;
    let relational = RelationalStorage::connect(cfg, &backends.relational, command).await?;

    if backends.events.has_clickhouse() {
        init_clickhouse_schema(cfg).await?;
    } else {
        init_duckdb_schema(&cfg.repo_root, &backends.events).await?;
    }
    init_relational_schema(cfg, &relational).await?;
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
    ))
}

pub(crate) async fn execute_init_schema(
    cfg: &DevqlConfig,
    command: &str,
) -> Result<InitSchemaSummary> {
    let (_relational, summary) = initialise_devql_schema_for_command(cfg, command).await?;
    Ok(summary)
}

pub async fn run_init(cfg: &DevqlConfig) -> Result<()> {
    let summary = execute_init_schema(cfg, "devql init").await?;
    println!("{}", format_init_schema_summary(&summary));
    Ok(())
}

pub async fn run_init_for_bitloops(cfg: &DevqlConfig, skip_baseline: bool) -> Result<()> {
    let (relational, summary) = initialise_devql_schema_for_command(cfg, "bitloops init").await?;
    println!("{}", format_init_schema_summary(&summary));

    if skip_baseline {
        println!("Baseline ingestion skipped (`--skip-baseline`).");
        return Ok(());
    }

    run_baseline_ingestion(cfg, &relational).await
}

mod canonical_mapping;
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
// ingestion: JS/TS artefact extraction (tree-sitter)
#[path = "devql/ingestion/extraction_js_ts.rs"]
mod ingestion_extraction_js_ts;
// ingestion: Rust artefact extraction (tree-sitter)
#[path = "devql/ingestion/extraction_rust.rs"]
mod ingestion_extraction_rust;
// ingestion: shared edge-building utilities
#[path = "devql/ingestion/edges_shared.rs"]
mod ingestion_edges_shared;
// ingestion: export edges (JS/TS + Rust)
#[path = "devql/ingestion/edges_export.rs"]
mod ingestion_edges_export;
// ingestion: inheritance edges (JS/TS + Rust)
#[path = "devql/ingestion/edges_inherits.rs"]
mod ingestion_edges_inherits;
// ingestion: reference edges (JS/TS + Rust)
#[path = "devql/ingestion/edges_reference.rs"]
mod ingestion_edges_reference;
// ingestion: JS/TS dependency edge orchestration
#[path = "devql/ingestion/edges_js_ts.rs"]
mod ingestion_edges_js_ts;
// ingestion: Rust dependency edge orchestration
#[path = "devql/db_utils.rs"]
mod db_utils;
#[path = "devql/deps_query.rs"]
mod deps_query;
#[path = "devql/ingestion/edges_rust.rs"]
mod ingestion_edges_rust;
#[path = "devql/query/executor.rs"]
mod query_executor;
#[path = "devql/query/parser.rs"]
mod query_parser;
#[path = "devql/query/utils.rs"]
mod query_utils;

use self::canonical_mapping::*;
use self::core_contracts::*;
use self::db_utils::*;
pub(crate) use self::db_utils::{
    clickhouse_query_data, duckdb_query_rows_path, esc_ch, esc_pg, postgres_exec,
    sqlite_exec_path_allow_create, sqlite_query_rows_path,
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
use self::ingestion_edges_export::*;
use self::ingestion_edges_inherits::*;
use self::ingestion_edges_js_ts::*;
use self::ingestion_edges_reference::*;
use self::ingestion_edges_rust::*;
use self::ingestion_edges_shared::*;
use self::ingestion_extraction_js_ts::*;
use self::ingestion_extraction_rust::*;
use self::ingestion_language::*;
pub use self::ingestion_repo_identity::{resolve_repo_id, resolve_repo_identity};
use self::ingestion_schema::*;
pub(crate) use self::ingestion_schema::{
    checkpoint_schema_sql_postgres, checkpoint_schema_sql_sqlite, devql_schema_sql_sqlite,
    knowledge_schema_sql_duckdb, knowledge_schema_sql_sqlite,
};
pub(crate) use self::ingestion_types::IngestionCounters;
use self::ingestion_types::*;
use self::query_executor::*;
use self::query_parser::*;
pub(crate) use self::query_utils::sql_string_list_pg;
use self::query_utils::*;
use self::vocab::*;
pub(crate) use self::vocab::{EDGE_KIND_CALLS, EDGE_KIND_EXPORTS};

#[cfg(test)]
pub(crate) use crate::capability_packs::semantic_clones::pipeline::rebuild_symbol_clone_edges;

#[cfg(test)]
fn symbol_id_for_artefact(item: &JsTsArtefact) -> String {
    structural_symbol_id_for_artefact(item, None)
}

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
