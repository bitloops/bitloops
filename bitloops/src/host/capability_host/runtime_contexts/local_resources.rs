use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::adapters::connectors::BuiltinConnectorRegistry;
use crate::capability_packs::knowledge::storage::{
    BlobKnowledgePayloadStore, DuckdbKnowledgeDocumentStore, SqliteKnowledgeRelationalRepository,
};
use crate::capability_packs::test_harness::storage::{
    BitloopsTestHarnessRepository, open_repository_for_repo,
};
use crate::config::{
    ProviderConfig, StoreBackendConfig, resolve_provider_config_for_repo,
    resolve_store_backend_config_for_repo,
};
use crate::host::capability_host::gateways::SqliteRelationalGateway;
use crate::host::devql::RelationalStorage;
use crate::host::devql::RepoIdentity;
use crate::host::relational_store::DefaultRelationalStore;

use super::capability_config::build_capability_config_root;
use super::language_services::{BuiltinLanguageServicesGateway, builtin_language_services};
use super::local_gateways::{
    DefaultProvenanceBuilder, LocalCanonicalGraphGateway, LocalStoreHealthGateway,
};
use super::local_runtime::LocalCapabilityRuntime;

pub struct LocalCapabilityRuntimeResources {
    pub repo_root: PathBuf,
    pub repo: RepoIdentity,
    pub config_root: serde_json::Value,
    pub backends: StoreBackendConfig,
    pub provider_config: ProviderConfig,
    pub relational: SqliteRelationalGateway,
    pub knowledge_relational: SqliteKnowledgeRelationalRepository,
    pub knowledge_documents: DuckdbKnowledgeDocumentStore,
    pub blob_payloads: BlobKnowledgePayloadStore,
    pub connectors: BuiltinConnectorRegistry,
    pub provenance: DefaultProvenanceBuilder,
    pub graph: LocalCanonicalGraphGateway,
    pub stores: LocalStoreHealthGateway,
    pub test_harness: Option<std::sync::Mutex<BitloopsTestHarnessRepository>>,
    pub languages: &'static BuiltinLanguageServicesGateway,
}

impl LocalCapabilityRuntimeResources {
    pub fn new(repo_root: &Path, repo: RepoIdentity) -> Result<Self> {
        let backends = resolve_store_backend_config_for_repo(repo_root)?;
        let provider_config = resolve_provider_config_for_repo(repo_root)?;

        let relational_store = DefaultRelationalStore::open_local_for_repo_root(repo_root)?;
        let sqlite_pool = relational_store.local_sqlite_pool_allow_create()?;
        let relational = SqliteRelationalGateway::new(sqlite_pool.clone());
        let knowledge_relational = SqliteKnowledgeRelationalRepository::new(sqlite_pool);
        let knowledge_documents =
            DuckdbKnowledgeDocumentStore::new(backends.events.duckdb_path_or_default());
        let blob_payloads = BlobKnowledgePayloadStore::from_backend_config(repo_root, &backends)?;
        let connectors = BuiltinConnectorRegistry::new(provider_config.clone())?;

        let config_root = build_capability_config_root(&backends, &provider_config);
        let stores = LocalStoreHealthGateway;
        let test_harness = open_repository_for_repo(repo_root)
            .ok()
            .map(std::sync::Mutex::new);

        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            repo,
            config_root,
            backends,
            provider_config,
            relational,
            knowledge_relational,
            knowledge_documents,
            blob_payloads,
            connectors,
            provenance: DefaultProvenanceBuilder,
            graph: LocalCanonicalGraphGateway,
            stores,
            test_harness,
            languages: builtin_language_services()?,
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
            &self.knowledge_relational,
            &self.knowledge_documents,
            &self.blob_payloads,
            &self.connectors,
            &self.provenance,
            &self.graph,
            &self.stores,
            self.test_harness.as_ref(),
            self.languages,
            devql_relational,
            invoking_capability_id,
            invoking_ingester_id,
        )
    }
}
