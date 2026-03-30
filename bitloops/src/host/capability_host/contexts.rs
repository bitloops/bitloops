use std::path::Path;

use anyhow::{Result, anyhow, bail};

use crate::capability_packs::knowledge::storage::{
    KnowledgeDocumentRepository, KnowledgeRelationalRepository,
};
use crate::capability_packs::test_harness::storage::BitloopsTestHarnessRepository;
use crate::host::devql::RepoIdentity;

use super::config_view::CapabilityConfigView;
use super::gateways::{
    BlobPayloadGateway, CanonicalGraphGateway, ConnectorRegistry, EmptyLanguageServicesGateway,
    LanguageServicesGateway, ProvenanceBuilder, RelationalGateway, StoreHealthGateway,
};

pub trait CapabilityExecutionContext: Send {
    fn repo(&self) -> &RepoIdentity;
    fn repo_root(&self) -> &Path;
    fn graph(&self) -> &dyn CanonicalGraphGateway;
    fn host_relational(&self) -> &dyn RelationalGateway;

    fn languages(&self) -> &dyn LanguageServicesGateway {
        static EMPTY: EmptyLanguageServicesGateway = EmptyLanguageServicesGateway;
        &EMPTY
    }

    fn test_harness_store(&self) -> Option<&std::sync::Mutex<BitloopsTestHarnessRepository>> {
        None
    }
}

pub trait CapabilityIngestContext: Send {
    fn repo(&self) -> &RepoIdentity;
    fn repo_root(&self) -> &Path;
    fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView>;
    fn blob_payloads(&self) -> &dyn BlobPayloadGateway;
    fn connectors(&self) -> &dyn ConnectorRegistry;
    fn connector_context(&self) -> &dyn super::gateways::ConnectorContext;
    fn provenance(&self) -> &dyn ProvenanceBuilder;
    fn host_relational(&self) -> &dyn RelationalGateway;
    fn languages(&self) -> &dyn LanguageServicesGateway {
        static EMPTY: EmptyLanguageServicesGateway = EmptyLanguageServicesGateway;
        &EMPTY
    }

    fn test_harness_store(&self) -> Option<&std::sync::Mutex<BitloopsTestHarnessRepository>> {
        None
    }

    fn clone_rebuild_relational(&self) -> Result<&crate::host::devql::RelationalStorage> {
        let capability_id = self.invoking_capability_id().unwrap_or("<unknown>");
        let ingester_id = self.invoking_ingester_id().unwrap_or("<unknown>");
        bail!(
            "clone rebuild relational access is not available for capability `{capability_id}` ingester `{ingester_id}`"
        );
    }

    fn devql_relational(&self) -> Option<&crate::host::devql::RelationalStorage> {
        None
    }

    fn invoking_capability_id(&self) -> Option<&str> {
        None
    }

    fn invoking_ingester_id(&self) -> Option<&str> {
        None
    }

    fn devql_relational_scoped(
        &self,
        capability_id: &str,
    ) -> Result<&crate::host::devql::RelationalStorage> {
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
        self.devql_relational().ok_or_else(|| {
            anyhow!(
                "[devql_relational_scoped] relational store not attached for this ingest (expected_capability_id={capability_id})"
            )
        })
    }
}

pub trait CapabilityMigrationContext: Send {
    fn repo(&self) -> &RepoIdentity;
    fn repo_root(&self) -> &Path;
    fn apply_devql_sqlite_ddl(&self, sql: &str) -> Result<()>;
}

pub trait KnowledgeExecutionContext: CapabilityExecutionContext {
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalRepository;
    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentRepository;
}

pub trait KnowledgeIngestContext: CapabilityIngestContext {
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalRepository;
    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentRepository;
}

pub trait KnowledgeMigrationContext: CapabilityMigrationContext {
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalRepository;
    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentRepository;
}

pub trait CapabilityHealthContext: Send + Sync {
    fn repo(&self) -> &RepoIdentity;
    fn repo_root(&self) -> &Path;
    fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView>;
    fn connectors(&self) -> &dyn ConnectorRegistry;
    fn stores(&self) -> &dyn StoreHealthGateway;
}
