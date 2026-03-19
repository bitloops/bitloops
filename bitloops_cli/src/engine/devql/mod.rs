use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio_postgres::{NoTls, config::SslMode};

use crate::engine::db_status::{
    DatabaseConnectionStatus, DatabaseStatusRow, classify_connection_error,
};
use crate::engine::providers::embeddings::EmbeddingProvider;
use crate::engine::semantic_clones;
use crate::engine::semantic_embeddings;
use crate::engine::semantic_features as semantic;
use crate::engine::strategy::manual_commit::{
    CommittedInfo, list_committed, read_commit_checkpoint_mappings, read_committed,
    read_session_content, run_git,
};
use crate::store_config::{
    EventsBackendConfig, EventsProvider, RelationalBackendConfig, RelationalProvider,
    StoreBackendConfig, resolve_store_backend_config, resolve_store_backend_config_for_repo,
    resolve_store_embedding_config, resolve_store_semantic_config,
};
use crate::terminal::db_status_table::print_db_status_table;

pub mod watch;

#[derive(Debug, Clone)]
pub struct RepoIdentity {
    pub(crate) provider: String,
    pub(crate) organization: String,
    pub(crate) name: String,
    pub(crate) identity: String,
    pub(crate) repo_id: String,
}

#[derive(Debug, Clone)]
pub struct DevqlConfig {
    pub(crate) repo_root: PathBuf,
    pub(crate) repo: RepoIdentity,
    pub(crate) pg_dsn: Option<String>,
    pub(crate) clickhouse_url: String,
    pub(crate) clickhouse_user: Option<String>,
    pub(crate) clickhouse_password: Option<String>,
    pub(crate) clickhouse_database: String,
    pub(crate) semantic_provider: Option<String>,
    pub(crate) semantic_model: Option<String>,
    pub(crate) semantic_api_key: Option<String>,
    pub(crate) semantic_base_url: Option<String>,
    pub(crate) embedding_provider: Option<String>,
    pub(crate) embedding_model: Option<String>,
    pub(crate) embedding_api_key: Option<String>,
}

impl DevqlConfig {
    pub fn from_env(repo_root: PathBuf, repo: RepoIdentity) -> Result<Self> {
        let backend_cfg = resolve_store_backend_config_for_repo(&repo_root)
            .context("resolving backend config for DevQL runtime")?;
        let semantic_cfg = resolve_store_semantic_config();
        let embedding_cfg = resolve_store_embedding_config();
        Ok(Self {
            repo_root,
            repo,
            pg_dsn: backend_cfg.relational.postgres_dsn,
            clickhouse_url: backend_cfg
                .events
                .clickhouse_url
                .unwrap_or_else(|| "http://localhost:8123".to_string()),
            clickhouse_user: backend_cfg.events.clickhouse_user,
            clickhouse_password: backend_cfg.events.clickhouse_password,
            clickhouse_database: backend_cfg
                .events
                .clickhouse_database
                .unwrap_or_else(|| "default".to_string()),
            semantic_provider: semantic_cfg.semantic_provider,
            semantic_model: semantic_cfg.semantic_model,
            semantic_api_key: semantic_cfg.semantic_api_key,
            semantic_base_url: semantic_cfg.semantic_base_url,
            embedding_provider: embedding_cfg.embedding_provider,
            embedding_model: embedding_cfg.embedding_model,
            embedding_api_key: embedding_cfg.embedding_api_key,
        })
    }

    fn clickhouse_endpoint(&self) -> String {
        let base = self.clickhouse_url.trim_end_matches('/');
        format!("{base}/?database={}", self.clickhouse_database)
    }
}

pub async fn run_query(cfg: &DevqlConfig, query: &str, compact: bool) -> Result<()> {
    let output = execute_query_json(cfg, query).await?;
    if compact {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }

    Ok(())
}

#[allow(dead_code)] // Compiled in both bin/lib crates; used by lib hook runtime path.
pub async fn execute_query_json_for_repo_root(repo_root: &Path, query: &str) -> Result<Value> {
    let repo = resolve_repo_identity(repo_root)?;
    let cfg = DevqlConfig::from_env(repo_root.to_path_buf(), repo)?;
    execute_query_json(&cfg, query).await
}

async fn execute_query_json(cfg: &DevqlConfig, query: &str) -> Result<Value> {
    let parsed = parse_devql_query(query)?;
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .context("resolving DevQL backend config for `devql query`")?;
    let relational = if parsed.has_checkpoints_stage || parsed.has_telemetry_stage {
        None
    } else {
        Some(RelationalStorage::connect(cfg, &backends.relational, "devql query").await?)
    };
    let mut rows = execute_devql_query(cfg, &parsed, &backends.events, relational.as_ref()).await?;

    if !parsed.select_fields.is_empty() {
        rows = project_rows(rows, &parsed.select_fields);
    }

    Ok(Value::Array(rows))
}

include!("canonical_mapping.rs");
include!("vocab.rs");
// connection status checking
include!("connection_status.rs");
// relational storage abstraction
include!("relational_storage.rs");
// ingest orchestration
include!("ingest.rs");
// ingestion: shared types
include!("ingestion/types.rs");
// ingestion: repo identity & git remote parsing
include!("ingestion/repo_identity.rs");
// ingestion: database schema DDL
include!("ingestion/schema.rs");
// ingestion: language detection & git blob utilities
include!("ingestion/language.rs");
// ingestion: artefact symbol identity helpers
include!("ingestion/artefact_identity.rs");
// ingestion: checkpoint / commit / event persistence
include!("ingestion/checkpoint.rs");
// ingestion: shared record types for artefact persistence
include!("ingestion/artefact_persistence_types.rs");
// ingestion: SQL dialect helpers, JSON utilities, timestamp expressions
include!("ingestion/artefact_persistence_sql.rs");
// ingestion: file state row, file artefact upsert, revision management
include!("ingestion/artefact_persistence_file.rs");
// ingestion: symbol record building, content hashing, artefact DB upserts
include!("ingestion/artefact_persistence_symbols.rs");
// ingestion: edge records, current state queries/mutations, row deserialization
include!("ingestion/artefact_persistence_edges.rs");
// ingestion: top-level orchestration (refresh/upsert/delete current state)
include!("ingestion/artefact_persistence.rs");
// ingestion: Stage 1 semantic persistence
include!("ingestion/semantic_features_persistence.rs");
// ingestion: Stage 2 embedding persistence
include!("ingestion/semantic_embeddings_persistence.rs");
// ingestion: Stage 3 clone persistence
include!("ingestion/semantic_clones_persistence.rs");
// ingestion: JS/TS artefact extraction (tree-sitter)
include!("ingestion/extraction_js_ts.rs");
// ingestion: Rust artefact extraction (tree-sitter)
include!("ingestion/extraction_rust.rs");
// ingestion: shared edge-building utilities
include!("ingestion/edges_shared.rs");
// ingestion: export edges (JS/TS + Rust)
include!("ingestion/edges_export.rs");
// ingestion: inheritance edges (JS/TS + Rust)
include!("ingestion/edges_inherits.rs");
// ingestion: reference edges (JS/TS + Rust)
include!("ingestion/edges_reference.rs");
// ingestion: JS/TS dependency edge orchestration
include!("ingestion/edges_js_ts.rs");
// ingestion: Rust dependency edge orchestration
include!("ingestion/edges_rust.rs");
include!("query/parser.rs");
include!("query/executor.rs");
include!("query/utils.rs");
include!("deps_query.rs");
include!("db_utils.rs");

#[cfg(test)]
fn symbol_id_for_artefact(item: &JsTsArtefact) -> String {
    structural_symbol_id_for_artefact(item, None)
}

#[cfg(test)]
#[path = "tests/devql_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/identity_tests.rs"]
mod identity_tests;

#[cfg(test)]
#[path = "tests/mapping_tests.rs"]
mod mapping_tests;

#[cfg(test)]
#[path = "tests/cucumber_world.rs"]
mod cucumber_world;

#[cfg(test)]
#[path = "tests/cucumber_steps.rs"]
mod cucumber_steps;

#[cfg(test)]
#[path = "tests/cucumber_bdd.rs"]
mod cucumber_bdd;
