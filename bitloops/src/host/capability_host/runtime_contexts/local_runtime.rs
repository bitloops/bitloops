use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::capability_packs::knowledge::storage::{
    KnowledgeDocumentRepository, KnowledgeRelationalRepository,
};
use crate::capability_packs::test_harness::storage::BitloopsTestHarnessRepository;
use crate::config::StoreBackendConfig;
use crate::host::capability_host::config_view::CapabilityConfigView;
use crate::host::capability_host::contexts::{
    CapabilityExecutionContext, CapabilityHealthContext, CapabilityIngestContext,
    CapabilityMigrationContext, KnowledgeExecutionContext, KnowledgeIngestContext,
    KnowledgeMigrationContext,
};
use crate::host::capability_host::gateways::{
    BlobPayloadGateway, CanonicalGraphGateway, CapabilityWorkplaneGateway, ConnectorContext,
    ConnectorRegistry, LanguageServicesGateway, ProvenanceBuilder, RelationalGateway,
    StoreHealthGateway,
};
use crate::host::devql::RelationalStorage;
use crate::host::devql::RepoIdentity;
use crate::host::inference::{InferenceGateway, LocalInferenceGateway, ScopedInferenceGateway};
use crate::host::relational_store::DefaultRelationalStore;

use super::language_services::BuiltinLanguageServicesGateway;
use super::local_gateways::LocalCapabilityWorkplaneGateway;

pub struct LocalCapabilityRuntime<'a> {
    repo_root: &'a Path,
    repo: &'a RepoIdentity,
    config_root: &'a Value,
    backends: &'a StoreBackendConfig,
    relational: &'a dyn RelationalGateway,
    knowledge_relational: &'a dyn KnowledgeRelationalRepository,
    knowledge_documents: &'a dyn KnowledgeDocumentRepository,
    blob_payloads: &'a dyn BlobPayloadGateway,
    connectors: &'a dyn ConnectorRegistry,
    provenance: &'a dyn ProvenanceBuilder,
    graph: &'a dyn CanonicalGraphGateway,
    stores: &'a dyn StoreHealthGateway,
    inference: ScopedInferenceGateway<'a>,
    test_harness: Option<&'a std::sync::Mutex<BitloopsTestHarnessRepository>>,
    languages: &'a BuiltinLanguageServicesGateway,
    devql_relational: Option<&'a RelationalStorage>,
    invoking_capability_id: Option<&'a str>,
    invoking_ingester_id: Option<&'a str>,
    workplane: Option<LocalCapabilityWorkplaneGateway>,
}

impl<'a> LocalCapabilityRuntime<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_root: &'a Path,
        repo: &'a RepoIdentity,
        config_root: &'a Value,
        backends: &'a StoreBackendConfig,
        relational: &'a dyn RelationalGateway,
        knowledge_relational: &'a dyn KnowledgeRelationalRepository,
        knowledge_documents: &'a dyn KnowledgeDocumentRepository,
        blob_payloads: &'a dyn BlobPayloadGateway,
        connectors: &'a dyn ConnectorRegistry,
        provenance: &'a dyn ProvenanceBuilder,
        graph: &'a dyn CanonicalGraphGateway,
        stores: &'a dyn StoreHealthGateway,
        inference: &'a LocalInferenceGateway,
        test_harness: Option<&'a std::sync::Mutex<BitloopsTestHarnessRepository>>,
        languages: &'a BuiltinLanguageServicesGateway,
        devql_relational: Option<&'a RelationalStorage>,
        invoking_capability_id: Option<&'a str>,
        invoking_ingester_id: Option<&'a str>,
        workplane: Option<LocalCapabilityWorkplaneGateway>,
    ) -> Self {
        Self {
            repo_root,
            repo,
            config_root,
            backends,
            relational,
            knowledge_relational,
            knowledge_documents,
            blob_payloads,
            connectors,
            provenance,
            graph,
            stores,
            inference: inference.scoped(invoking_capability_id),
            test_harness,
            languages,
            devql_relational,
            invoking_capability_id,
            invoking_ingester_id,
            workplane,
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

    fn host_relational(&self) -> &dyn RelationalGateway {
        self.relational
    }

    fn inference(&self) -> &dyn InferenceGateway {
        &self.inference
    }

    fn languages(&self) -> &dyn LanguageServicesGateway {
        self.languages
    }

    fn test_harness_store(&self) -> Option<&std::sync::Mutex<BitloopsTestHarnessRepository>> {
        self.test_harness
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

    fn host_relational(&self) -> &dyn RelationalGateway {
        self.relational
    }

    fn inference(&self) -> &dyn InferenceGateway {
        &self.inference
    }

    fn languages(&self) -> &dyn LanguageServicesGateway {
        self.languages
    }

    fn test_harness_store(&self) -> Option<&std::sync::Mutex<BitloopsTestHarnessRepository>> {
        self.test_harness
    }

    fn clone_edges_rebuild_relational(&self) -> Result<&RelationalStorage> {
        let Some(relational) = self.devql_relational else {
            anyhow::bail!("clone-edge rebuild relational store is not attached to this ingest");
        };
        Ok(relational)
    }

    fn devql_relational(&self) -> Option<&RelationalStorage> {
        self.devql_relational
    }

    fn workplane(&self) -> Option<&dyn CapabilityWorkplaneGateway> {
        self.workplane
            .as_ref()
            .map(|gateway| gateway as &dyn CapabilityWorkplaneGateway)
    }

    fn invoking_capability_id(&self) -> Option<&str> {
        self.invoking_capability_id
    }

    fn invoking_ingester_id(&self) -> Option<&str> {
        self.invoking_ingester_id
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
        if self.backends.relational.has_postgres() {
            return Ok(());
        }
        let relational = DefaultRelationalStore::open_local_for_backend_config(
            self.repo_root,
            &self.backends.relational,
        )
        .context("opening local relational store for DevQL DDL")?;
        relational.execute_local_sqlite_batch_allow_create(sql)
    }
}

impl KnowledgeExecutionContext for LocalCapabilityRuntime<'_> {
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalRepository {
        self.knowledge_relational
    }

    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentRepository {
        self.knowledge_documents
    }
}

impl KnowledgeIngestContext for LocalCapabilityRuntime<'_> {
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalRepository {
        self.knowledge_relational
    }

    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentRepository {
        self.knowledge_documents
    }
}

impl KnowledgeMigrationContext for LocalCapabilityRuntime<'_> {
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalRepository {
        self.knowledge_relational
    }

    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentRepository {
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

    fn inference(&self) -> &dyn InferenceGateway {
        &self.inference
    }
}
