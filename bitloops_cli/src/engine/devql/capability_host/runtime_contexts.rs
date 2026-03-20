use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use crate::engine::adapters::connectors::BuiltinConnectorRegistry;
use crate::engine::db::SqliteConnectionPool;
use crate::engine::devql::RepoIdentity;
use crate::engine::devql::capabilities::knowledge::storage::{
    BlobKnowledgePayloadStore, DuckdbKnowledgeDocumentStore, SqliteKnowledgeRelationalStore,
};
use crate::store_config::{
    ProviderConfig, StoreBackendConfig, resolve_provider_config_for_repo,
    resolve_store_backend_config_for_repo,
};

use super::config_view::CapabilityConfigView;
use super::contexts::{
    CapabilityExecutionContext, CapabilityHealthContext, CapabilityIngestContext,
    CapabilityMigrationContext,
};
use super::gateways::{
    BlobPayloadGateway, CanonicalGraphGateway, ConnectorContext, ConnectorRegistry,
    KnowledgeDocumentGateway, KnowledgeRelationalGateway, ProvenanceBuilder, StoreHealthGateway,
};

pub struct LocalCapabilityRuntimeResources {
    pub repo_root: PathBuf,
    pub repo: RepoIdentity,
    pub config_root: Value,
    pub backends: StoreBackendConfig,
    pub provider_config: ProviderConfig,
    pub knowledge_relational: SqliteKnowledgeRelationalStore,
    pub knowledge_documents: DuckdbKnowledgeDocumentStore,
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

        let sqlite_path = backends.relational.resolve_sqlite_db_path()?;
        let knowledge_relational =
            SqliteKnowledgeRelationalStore::new(SqliteConnectionPool::connect(sqlite_path)?);
        let knowledge_documents =
            DuckdbKnowledgeDocumentStore::new(backends.events.duckdb_path_or_default());
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
            knowledge_relational,
            knowledge_documents,
            blob_payloads,
            connectors,
            provenance: DefaultProvenanceBuilder,
            graph: LocalCanonicalGraphGateway,
            stores,
        })
    }

    pub fn runtime(&self) -> LocalCapabilityRuntime<'_> {
        LocalCapabilityRuntime::new(
            &self.repo_root,
            &self.repo,
            &self.config_root,
            &self.knowledge_relational,
            &self.knowledge_documents,
            &self.blob_payloads,
            &self.connectors,
            &self.provenance,
            &self.graph,
            &self.stores,
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
        }
    })
}

pub struct LocalCapabilityRuntime<'a> {
    repo_root: &'a Path,
    repo: &'a RepoIdentity,
    config_root: &'a Value,
    knowledge_relational: &'a dyn KnowledgeRelationalGateway,
    knowledge_documents: &'a dyn KnowledgeDocumentGateway,
    blob_payloads: &'a dyn BlobPayloadGateway,
    connectors: &'a dyn ConnectorRegistry,
    provenance: &'a dyn ProvenanceBuilder,
    graph: &'a dyn CanonicalGraphGateway,
    stores: &'a dyn StoreHealthGateway,
}

impl<'a> LocalCapabilityRuntime<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_root: &'a Path,
        repo: &'a RepoIdentity,
        config_root: &'a Value,
        knowledge_relational: &'a dyn KnowledgeRelationalGateway,
        knowledge_documents: &'a dyn KnowledgeDocumentGateway,
        blob_payloads: &'a dyn BlobPayloadGateway,
        connectors: &'a dyn ConnectorRegistry,
        provenance: &'a dyn ProvenanceBuilder,
        graph: &'a dyn CanonicalGraphGateway,
        stores: &'a dyn StoreHealthGateway,
    ) -> Self {
        Self {
            repo_root,
            repo,
            config_root,
            knowledge_relational,
            knowledge_documents,
            blob_payloads,
            connectors,
            provenance,
            graph,
            stores,
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

    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalGateway {
        self.knowledge_relational
    }

    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentGateway {
        self.knowledge_documents
    }

    fn graph(&self) -> &dyn CanonicalGraphGateway {
        self.graph
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

    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalGateway {
        self.knowledge_relational
    }

    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentGateway {
        self.knowledge_documents
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
}

impl CapabilityMigrationContext for LocalCapabilityRuntime<'_> {
    fn repo(&self) -> &RepoIdentity {
        self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root
    }

    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalGateway {
        self.knowledge_relational
    }

    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentGateway {
        self.knowledge_documents
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
