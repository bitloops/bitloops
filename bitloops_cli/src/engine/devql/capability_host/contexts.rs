use std::path::Path;

use anyhow::{Result, anyhow, bail};

use crate::engine::devql::RepoIdentity;

use super::config_view::CapabilityConfigView;
use super::gateways::{
    BlobPayloadGateway, CanonicalGraphGateway, ConnectorRegistry, DocumentStoreGateway,
    ProvenanceBuilder, RelationalGateway, StoreHealthGateway,
};

pub trait CapabilityExecutionContext: Send {
    fn repo(&self) -> &RepoIdentity;
    fn repo_root(&self) -> &Path;
    fn graph(&self) -> &dyn CanonicalGraphGateway;
}

/// Knowledge pack relational + document stores. Only knowledge stages/handlers use this trait.
pub trait KnowledgeExecutionContext: CapabilityExecutionContext {
    fn relational(&self) -> &dyn RelationalGateway;
    fn documents(&self) -> &dyn DocumentStoreGateway;
}

pub trait CapabilityIngestContext: Send {
    fn repo(&self) -> &RepoIdentity;
    fn repo_root(&self) -> &Path;
    fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView>;
    fn blob_payloads(&self) -> &dyn BlobPayloadGateway;
    fn connectors(&self) -> &dyn ConnectorRegistry;
    fn connector_context(&self) -> &dyn super::gateways::ConnectorContext;
    fn provenance(&self) -> &dyn ProvenanceBuilder;

    /// Open DevQL relational connection held by the ingest pipeline (SQLite/Postgres). `None` for
    /// host-only entrypoints such as the knowledge CLI.
    fn devql_relational(&self) -> Option<&crate::engine::devql::RelationalStorage> {
        None
    }

    /// Capability id for the ingester invocation currently running (`None` outside ingester dispatch).
    fn invoking_capability_id(&self) -> Option<&str> {
        None
    }

    /// DevQL relational store only when `capability_id` matches the active ingester invocation.
    fn devql_relational_scoped(
        &self,
        capability_id: &str,
    ) -> Result<&crate::engine::devql::RelationalStorage> {
        let Some(inv) = self.invoking_capability_id() else {
            bail!(
                "[devql_relational_scoped] no active ingester invocation (expected_capability_id={capability_id})"
            );
        };
        if inv != capability_id {
            bail!(
                "[devql_relational_scoped] invoking_capability_id={inv} does not match expected_capability_id={capability_id}"
            );
        }
        self.devql_relational()
            .ok_or_else(|| {
                anyhow!(
                    "[devql_relational_scoped] relational store not attached for this ingest (expected_capability_id={capability_id})"
                )
            })
    }
}

/// Knowledge pack relational + document stores on ingest paths. Only knowledge ingesters use this trait.
pub trait KnowledgeIngestContext: CapabilityIngestContext {
    fn relational(&self) -> &dyn RelationalGateway;
    fn documents(&self) -> &dyn DocumentStoreGateway;
}

pub trait CapabilityMigrationContext: Send {
    fn repo(&self) -> &RepoIdentity;
    fn repo_root(&self) -> &Path;
    fn relational(&self) -> &dyn RelationalGateway;
    fn documents(&self) -> &dyn DocumentStoreGateway;

    /// Applies DDL to the DevQL SQLite relational file when the backend is SQLite. No-op for
    /// Postgres (pack tables are ensured via `devql init` / relational bootstrap).
    fn apply_devql_sqlite_ddl(&self, sql: &str) -> Result<()>;
}

pub trait CapabilityHealthContext: Send + Sync {
    fn repo(&self) -> &RepoIdentity;
    fn repo_root(&self) -> &Path;
    fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView>;
    fn connectors(&self) -> &dyn ConnectorRegistry;
    fn stores(&self) -> &dyn StoreHealthGateway;
}
