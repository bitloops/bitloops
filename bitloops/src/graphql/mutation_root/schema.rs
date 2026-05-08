use async_graphql::{Context, Result};
use chrono::Utc;
use std::path::Path;

use crate::graphql::DevqlGraphqlContext;
use crate::graphql::types::DateTimeScalar;

use super::errors::operation_error;
use super::results::{ApplyMigrationsMutationResult, InitSchemaResult, MigrationRecord};

pub(super) async fn init_schema(ctx: &Context<'_>) -> Result<InitSchemaResult> {
    let cfg = ctx
        .data_unchecked::<DevqlGraphqlContext>()
        .devql_config()
        .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "initSchema", err))?;
    let summary = crate::host::devql::execute_init_schema(&cfg, "GraphQL mutation `initSchema`")
        .await
        .map_err(|err| operation_error("BACKEND_ERROR", "initialisation", "initSchema", err))?;
    Ok(summary.into())
}

pub(super) async fn apply_migrations(ctx: &Context<'_>) -> Result<ApplyMigrationsMutationResult> {
    let host = ctx
        .data_unchecked::<DevqlGraphqlContext>()
        .capability_host_arc()
        .map_err(|err| operation_error("BACKEND_ERROR", "configuration", "applyMigrations", err))?;

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

fn ensure_knowledge_document_schema(repo_root: &Path) -> anyhow::Result<()> {
    let backends = crate::config::resolve_store_backend_config_for_repo(repo_root)?;
    let documents = crate::capability_packs::knowledge::storage::DuckdbKnowledgeDocumentStore::new(
        backends.events.resolve_duckdb_db_path_for_repo(repo_root),
    );
    documents.initialise_schema()
}
