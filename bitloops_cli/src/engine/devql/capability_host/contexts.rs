use std::path::Path;

use anyhow::Result;

use crate::engine::devql::RepoIdentity;

use super::config_view::CapabilityConfigView;
use super::gateways::{
    BlobPayloadGateway, CanonicalGraphGateway, ConnectorRegistry, KnowledgeDocumentGateway,
    KnowledgeRelationalGateway, ProvenanceBuilder, StoreHealthGateway,
};

pub trait CapabilityExecutionContext: Send {
    fn repo(&self) -> &RepoIdentity;
    fn repo_root(&self) -> &Path;
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalGateway;
    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentGateway;
    fn graph(&self) -> &dyn CanonicalGraphGateway;
}

pub trait CapabilityIngestContext: Send {
    fn repo(&self) -> &RepoIdentity;
    fn repo_root(&self) -> &Path;
    fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView>;
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalGateway;
    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentGateway;
    fn blob_payloads(&self) -> &dyn BlobPayloadGateway;
    fn connectors(&self) -> &dyn ConnectorRegistry;
    fn connector_context(&self) -> &dyn super::gateways::ConnectorContext;
    fn provenance(&self) -> &dyn ProvenanceBuilder;
}

pub trait CapabilityMigrationContext: Send {
    fn repo(&self) -> &RepoIdentity;
    fn repo_root(&self) -> &Path;
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalGateway;
    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentGateway;
}

pub trait CapabilityHealthContext: Send + Sync {
    fn repo(&self) -> &RepoIdentity;
    fn repo_root(&self) -> &Path;
    fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView>;
    fn connectors(&self) -> &dyn ConnectorRegistry;
    fn stores(&self) -> &dyn StoreHealthGateway;
}
