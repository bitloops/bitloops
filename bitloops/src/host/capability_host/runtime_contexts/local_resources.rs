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
    InferenceCapabilityConfig, ProviderConfig, StoreBackendConfig,
    resolve_inference_capability_config_for_repo, resolve_provider_config_for_repo,
    resolve_store_backend_config_for_repo,
};
use crate::host::capability_host::CapabilityMailboxRegistration;
use crate::host::capability_host::gateways::SqliteRelationalGateway;
use crate::host::devql::RelationalStorage;
use crate::host::devql::RepoIdentity;
use crate::host::inference::LocalInferenceGateway;
use crate::host::relational_store::DefaultRelationalStore;

use super::capability_config::build_capability_config_root;
use super::language_services::{BuiltinLanguageServicesGateway, builtin_language_services};
use super::local_gateways::{
    DefaultProvenanceBuilder, LocalCanonicalGraphGateway, LocalCapabilityWorkplaneGateway,
    LocalStoreHealthGateway,
};
use super::local_runtime::LocalCapabilityRuntime;

pub struct LocalCapabilityRuntimeResources {
    pub repo_root: PathBuf,
    pub repo: RepoIdentity,
    pub config_root: serde_json::Value,
    pub backends: StoreBackendConfig,
    pub provider_config: ProviderConfig,
    pub inference_config: InferenceCapabilityConfig,
    pub relational: SqliteRelationalGateway,
    pub knowledge_relational: SqliteKnowledgeRelationalRepository,
    pub knowledge_documents: DuckdbKnowledgeDocumentStore,
    pub blob_payloads: BlobKnowledgePayloadStore,
    pub connectors: BuiltinConnectorRegistry,
    pub provenance: DefaultProvenanceBuilder,
    pub graph: LocalCanonicalGraphGateway,
    pub stores: LocalStoreHealthGateway,
    pub inference: LocalInferenceGateway,
    pub test_harness: Option<std::sync::Mutex<BitloopsTestHarnessRepository>>,
    pub languages: &'static BuiltinLanguageServicesGateway,
}

impl LocalCapabilityRuntimeResources {
    pub fn new(repo_root: &Path, repo: RepoIdentity) -> Result<Self> {
        let backends = resolve_store_backend_config_for_repo(repo_root)?;
        let provider_config = resolve_provider_config_for_repo(repo_root)?;
        let inference_config = resolve_inference_capability_config_for_repo(repo_root);

        let relational_store = DefaultRelationalStore::open_local_for_repo_root(repo_root)?;
        let sqlite_pool = relational_store.local_sqlite_pool_allow_create()?;
        let relational = SqliteRelationalGateway::new(sqlite_pool.clone());
        let knowledge_relational = SqliteKnowledgeRelationalRepository::new(sqlite_pool);
        let knowledge_documents =
            DuckdbKnowledgeDocumentStore::new(backends.events.duckdb_path_or_default());
        let blob_payloads = BlobKnowledgePayloadStore::from_backend_config(repo_root, &backends)?;
        let connectors = BuiltinConnectorRegistry::new(provider_config.clone())?;

        let config_root = build_capability_config_root(
            &backends,
            &provider_config,
            &inference_config.semantic_clones,
            &inference_config.inference,
        );
        let stores = LocalStoreHealthGateway;
        let test_harness = open_repository_for_repo(repo_root)
            .ok()
            .map(std::sync::Mutex::new);
        let inference = LocalInferenceGateway::new(
            repo_root,
            inference_config.inference.clone(),
            build_slot_bindings(&inference_config),
        );

        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            repo,
            config_root,
            backends,
            provider_config,
            inference_config,
            relational,
            knowledge_relational,
            knowledge_documents,
            blob_payloads,
            connectors,
            provenance: DefaultProvenanceBuilder,
            graph: LocalCanonicalGraphGateway,
            stores,
            inference,
            test_harness,
            languages: builtin_language_services()?,
        })
    }

    pub fn runtime(&self) -> LocalCapabilityRuntime<'_> {
        self.runtime_with_relational(None, None, None, &[])
    }

    pub fn runtime_for_capability<'a>(
        &'a self,
        capability_id: &'a str,
        declared_mailboxes: &'a [CapabilityMailboxRegistration],
    ) -> LocalCapabilityRuntime<'a> {
        self.runtime_with_relational(None, Some(capability_id), None, declared_mailboxes)
    }

    pub fn runtime_with_relational<'a>(
        &'a self,
        devql_relational: Option<&'a RelationalStorage>,
        invoking_capability_id: Option<&'a str>,
        invoking_ingester_id: Option<&'a str>,
        declared_mailboxes: &'a [CapabilityMailboxRegistration],
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
            &self.inference,
            self.test_harness.as_ref(),
            self.languages,
            devql_relational,
            invoking_capability_id,
            invoking_ingester_id,
            invoking_capability_id.and_then(|capability_id| {
                LocalCapabilityWorkplaneGateway::new(
                    &self.repo_root,
                    capability_id,
                    declared_mailboxes,
                )
                .ok()
            }),
        )
    }

    pub fn workplane_gateway_for_capability(
        &self,
        capability_id: &str,
        declared_mailboxes: &[CapabilityMailboxRegistration],
    ) -> Result<LocalCapabilityWorkplaneGateway> {
        LocalCapabilityWorkplaneGateway::new(&self.repo_root, capability_id, declared_mailboxes)
    }
}

fn build_slot_bindings(
    config: &InferenceCapabilityConfig,
) -> std::collections::HashMap<String, std::collections::BTreeMap<String, String>> {
    let mut bindings = std::collections::HashMap::new();
    let mut semantic_clones = std::collections::BTreeMap::new();
    if let Some(profile) = config.semantic_clones.inference.summary_generation.as_ref() {
        semantic_clones.insert("summary_generation".to_string(), profile.clone());
    }
    if let Some(profile) = config.semantic_clones.inference.code_embeddings.as_ref() {
        semantic_clones.insert("code_embeddings".to_string(), profile.clone());
    }
    if let Some(profile) = config.semantic_clones.inference.summary_embeddings.as_ref() {
        semantic_clones.insert("summary_embeddings".to_string(), profile.clone());
    }
    bindings.insert("semantic_clones".to_string(), semantic_clones);
    bindings
}
