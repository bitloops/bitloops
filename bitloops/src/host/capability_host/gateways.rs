pub mod blob_payloads;
pub mod documents;
pub mod relational;
pub mod sqlite_relational;

use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;

pub use crate::adapters::connectors::{
    ConnectorContext, ConnectorRegistry, ExternalKnowledgeRecord, KnowledgeConnectorAdapter,
};
use crate::host::language_adapter::LanguageTestSupport;
pub use blob_payloads::{BlobPayloadGateway, BlobPayloadRef};
pub use documents::DocumentStoreGateway;
pub use relational::RelationalGateway;
pub use sqlite_relational::SqliteRelationalGateway;

pub trait CanonicalGraphGateway: Send + Sync {}

pub trait ProvenanceBuilder: Send + Sync {
    fn build(&self, capability_id: &str, operation: &str, details: Value) -> Value;
}

pub trait StoreHealthGateway: Send + Sync {
    fn check_relational(&self) -> Result<()>;
    fn check_documents(&self) -> Result<()>;
    fn check_blobs(&self) -> Result<()>;
}

pub trait LanguageServicesGateway: Send + Sync {
    fn test_supports(&self) -> Vec<Arc<dyn LanguageTestSupport>> {
        Vec::new()
    }

    fn resolve_test_support_for_path(
        &self,
        relative_path: &str,
    ) -> Option<Arc<dyn LanguageTestSupport>> {
        let _ = relative_path;
        None
    }
}

pub struct EmptyLanguageServicesGateway;

impl LanguageServicesGateway for EmptyLanguageServicesGateway {}
