use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::adapters::connectors::BuiltinConnectorRegistry;
use crate::capability_packs::knowledge::storage::{
    BlobKnowledgePayloadStore, DuckdbKnowledgeDocumentStore, SqliteKnowledgeRelationalStore,
};
use crate::config::{
    ProviderConfig, RelationalProvider, StoreBackendConfig, resolve_provider_config_for_repo,
    resolve_store_backend_config_for_repo,
};
use crate::host::devql::RelationalStorage;
use crate::host::devql::RepoIdentity;
use crate::storage::SqliteConnectionPool;

use super::config_view::CapabilityConfigView;
use super::contexts::{
    CapabilityExecutionContext, CapabilityHealthContext, CapabilityIngestContext,
    CapabilityMigrationContext,
};
use super::gateways::{
    BlobPayloadGateway, CanonicalGraphGateway, ConnectorContext, ConnectorRegistry,
    DocumentStoreGateway, ProvenanceBuilder, RelationalGateway, StoreHealthGateway,
};

pub struct LocalCapabilityRuntimeResources {
    pub repo_root: PathBuf,
    pub repo: RepoIdentity,
    pub config_root: Value,
    pub backends: StoreBackendConfig,
    pub provider_config: ProviderConfig,
    pub relational: SqliteKnowledgeRelationalStore,
    pub documents: DuckdbKnowledgeDocumentStore,
    pub blob_payloads: BlobKnowledgePayloadStore,
    pub connectors: BuiltinConnectorRegistry,
    pub provenance: DefaultProvenanceBuilder,
    pub graph: LocalCanonicalGraphGateway,
    pub stores: LocalStoreHealthGateway,
}

impl LocalCapabilityRuntimeResources {
    pub fn new(repo_root: &Path, repo: RepoIdentity) -> Result<Self> {
        let backends = resolve_store_backend_config_for_repo(repo_root)?;
        let provider_config = resolve_provider_config_for_repo(repo_root)?;

        let sqlite_path = backends
            .relational
            .resolve_sqlite_db_path_for_repo(repo_root)?;
        let relational =
            SqliteKnowledgeRelationalStore::new(SqliteConnectionPool::connect(sqlite_path)?);
        let documents = DuckdbKnowledgeDocumentStore::new(backends.events.duckdb_path_or_default());
        let blob_payloads = BlobKnowledgePayloadStore::from_backend_config(repo_root, &backends)?;
        let connectors = BuiltinConnectorRegistry::new(provider_config.clone())?;

        let config_root = build_capability_config_root(&backends, &provider_config);
        let stores = LocalStoreHealthGateway;

        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            repo,
            config_root,
            backends,
            provider_config,
            relational,
            documents,
            blob_payloads,
            connectors,
            provenance: DefaultProvenanceBuilder,
            graph: LocalCanonicalGraphGateway,
            stores,
        })
    }

    pub fn runtime(&self) -> LocalCapabilityRuntime<'_> {
        self.runtime_with_relational(None, None, None)
    }

    pub fn runtime_with_relational<'a>(
        &'a self,
        devql_relational: Option<&'a RelationalStorage>,
        invoking_capability_id: Option<&'a str>,
        invoking_ingester_id: Option<&'a str>,
    ) -> LocalCapabilityRuntime<'a> {
        LocalCapabilityRuntime::new(
            &self.repo_root,
            &self.repo,
            &self.config_root,
            &self.backends,
            &self.relational,
            &self.documents,
            &self.blob_payloads,
            &self.connectors,
            &self.provenance,
            &self.graph,
            &self.stores,
            devql_relational,
            invoking_capability_id,
            invoking_ingester_id,
        )
    }
}

fn build_capability_config_root(
    backends: &StoreBackendConfig,
    providers: &ProviderConfig,
) -> Value {
    serde_json::json!({
        "knowledge": {
            "providers": {
                "github": providers.github.as_ref().map(|_| serde_json::json!({ "configured": true })),
                "jira": providers.jira.as_ref().map(|cfg| serde_json::json!({ "site_url": cfg.site_url })),
                "confluence": providers.confluence.as_ref().map(|cfg| serde_json::json!({ "site_url": cfg.site_url })),
                "atlassian": providers.atlassian.as_ref().map(|cfg| serde_json::json!({ "site_url": cfg.site_url })),
            },
            "backends": {
                "relational": backends.relational.provider.as_str(),
                "events": backends.events.provider.as_str(),
            }
        },
        "host": {
            "invocation": {
                "stage_timeout_secs": 120,
                "ingester_timeout_secs": 300,
                "subquery_timeout_secs": 60
            },
            "cross_pack_access": []
        }
    })
}

pub struct LocalCapabilityRuntime<'a> {
    repo_root: &'a Path,
    repo: &'a RepoIdentity,
    config_root: &'a Value,
    backends: &'a StoreBackendConfig,
    relational: &'a dyn RelationalGateway,
    documents: &'a dyn DocumentStoreGateway,
    blob_payloads: &'a dyn BlobPayloadGateway,
    connectors: &'a dyn ConnectorRegistry,
    provenance: &'a dyn ProvenanceBuilder,
    graph: &'a dyn CanonicalGraphGateway,
    stores: &'a dyn StoreHealthGateway,
    devql_relational: Option<&'a RelationalStorage>,
    invoking_capability_id: Option<&'a str>,
    invoking_ingester_id: Option<&'a str>,
}

impl<'a> LocalCapabilityRuntime<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_root: &'a Path,
        repo: &'a RepoIdentity,
        config_root: &'a Value,
        backends: &'a StoreBackendConfig,
        relational: &'a dyn RelationalGateway,
        documents: &'a dyn DocumentStoreGateway,
        blob_payloads: &'a dyn BlobPayloadGateway,
        connectors: &'a dyn ConnectorRegistry,
        provenance: &'a dyn ProvenanceBuilder,
        graph: &'a dyn CanonicalGraphGateway,
        stores: &'a dyn StoreHealthGateway,
        devql_relational: Option<&'a RelationalStorage>,
        invoking_capability_id: Option<&'a str>,
        invoking_ingester_id: Option<&'a str>,
    ) -> Self {
        Self {
            repo_root,
            repo,
            config_root,
            backends,
            relational,
            documents,
            blob_payloads,
            connectors,
            provenance,
            graph,
            stores,
            devql_relational,
            invoking_capability_id,
            invoking_ingester_id,
        }
    }
}

impl CapabilityExecutionContext for LocalCapabilityRuntime<'_> {
    fn repo(&self) -> &RepoIdentity {
        self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root
    }

    fn graph(&self) -> &dyn CanonicalGraphGateway {
        self.graph
    }

    fn relational(&self) -> Option<&dyn RelationalGateway> {
        Some(self.relational)
    }

    fn documents(&self) -> Option<&dyn DocumentStoreGateway> {
        Some(self.documents)
    }
}

impl CapabilityIngestContext for LocalCapabilityRuntime<'_> {
    fn repo(&self) -> &RepoIdentity {
        self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root
    }

    fn config_view(&self, capability_id: &str) -> anyhow::Result<CapabilityConfigView> {
        Ok(CapabilityConfigView::new(
            capability_id.to_string(),
            self.config_root.clone(),
        ))
    }

    fn blob_payloads(&self) -> &dyn BlobPayloadGateway {
        self.blob_payloads
    }

    fn connectors(&self) -> &dyn ConnectorRegistry {
        self.connectors
    }

    fn connector_context(&self) -> &dyn ConnectorContext {
        self.connectors
    }

    fn provenance(&self) -> &dyn ProvenanceBuilder {
        self.provenance
    }

    fn devql_relational(&self) -> Option<&RelationalStorage> {
        self.devql_relational
    }

    fn invoking_capability_id(&self) -> Option<&str> {
        self.invoking_capability_id
    }

    fn invoking_ingester_id(&self) -> Option<&str> {
        self.invoking_ingester_id
    }

    fn relational(&self) -> Option<&dyn RelationalGateway> {
        Some(self.relational)
    }

    fn documents(&self) -> Option<&dyn DocumentStoreGateway> {
        Some(self.documents)
    }
}

impl CapabilityMigrationContext for LocalCapabilityRuntime<'_> {
    fn repo(&self) -> &RepoIdentity {
        self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root
    }

    fn apply_devql_sqlite_ddl(&self, sql: &str) -> Result<()> {
        if self.backends.relational.provider != RelationalProvider::Sqlite {
            return Ok(());
        }
        let path = self
            .backends
            .relational
            .resolve_sqlite_db_path_for_repo(self.repo_root)
            .context("resolving SQLite path for DevQL relational DDL")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
        )
        .with_context(|| format!("opening SQLite at {}", path.display()))?;
        conn.execute_batch(sql)
            .context("applying DevQL SQLite DDL")?;
        Ok(())
    }

    fn relational(&self) -> Option<&dyn RelationalGateway> {
        Some(self.relational)
    }

    fn documents(&self) -> Option<&dyn DocumentStoreGateway> {
        Some(self.documents)
    }
}

impl CapabilityHealthContext for LocalCapabilityRuntime<'_> {
    fn repo(&self) -> &RepoIdentity {
        self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root
    }

    fn config_view(&self, capability_id: &str) -> anyhow::Result<CapabilityConfigView> {
        Ok(CapabilityConfigView::new(
            capability_id.to_string(),
            self.config_root.clone(),
        ))
    }

    fn connectors(&self) -> &dyn ConnectorRegistry {
        self.connectors
    }

    fn stores(&self) -> &dyn StoreHealthGateway {
        self.stores
    }
}

pub struct LocalCanonicalGraphGateway;

impl CanonicalGraphGateway for LocalCanonicalGraphGateway {}

pub struct DefaultProvenanceBuilder;

impl ProvenanceBuilder for DefaultProvenanceBuilder {
    fn build(&self, capability_id: &str, operation: &str, details: Value) -> Value {
        serde_json::json!({
            "capability": capability_id,
            "operation": operation,
            "details": details,
        })
    }
}

pub struct LocalStoreHealthGateway;

impl StoreHealthGateway for LocalStoreHealthGateway {
    fn check_relational(&self) -> Result<()> {
        Ok(())
    }

    fn check_documents(&self) -> Result<()> {
        Ok(())
    }

    fn check_blobs(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_provenance_builder_wraps_details() {
        let builder = DefaultProvenanceBuilder;
        let value = builder.build("knowledge", "ingest", json!({ "id": 1 }));

        assert_eq!(value["capability"], json!("knowledge"));
        assert_eq!(value["operation"], json!("ingest"));
        assert_eq!(value["details"]["id"], json!(1));
    }

    #[test]
    fn local_store_health_gateway_returns_ok() {
        let gateway = LocalStoreHealthGateway;

        assert!(gateway.check_relational().is_ok());
        assert!(gateway.check_documents().is_ok());
        assert!(gateway.check_blobs().is_ok());
    }
}
