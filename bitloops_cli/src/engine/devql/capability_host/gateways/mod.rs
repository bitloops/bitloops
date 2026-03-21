pub mod blob_payloads;
pub mod documents;
pub mod relational;
pub mod test_harness;

use anyhow::Result;
use serde_json::Value;

pub use crate::engine::adapters::connectors::{
    ConnectorContext, ConnectorRegistry, ExternalKnowledgeRecord, KnowledgeConnectorAdapter,
};
pub use blob_payloads::BlobPayloadGateway;
pub use documents::DocumentStoreGateway;
pub use relational::RelationalGateway;
pub use test_harness::TestHarnessCoverageGateway;

pub trait CanonicalGraphGateway: Send + Sync {}

pub trait ProvenanceBuilder: Send + Sync {
    fn build(&self, capability_id: &str, operation: &str, details: Value) -> Value;
}

pub trait StoreHealthGateway: Send + Sync {
    fn check_relational(&self) -> Result<()>;
    fn check_documents(&self) -> Result<()>;
    fn check_blobs(&self) -> Result<()>;
}
