use anyhow::Result;

use crate::engine::devql::capabilities::knowledge::storage::KnowledgePayloadRef;

pub trait BlobPayloadGateway: Send + Sync {
    fn write_payload(
        &self,
        repo_id: &str,
        knowledge_item_id: &str,
        knowledge_item_version_id: &str,
        bytes: &[u8],
    ) -> Result<KnowledgePayloadRef>;

    fn delete_payload(&self, payload: &KnowledgePayloadRef) -> Result<()>;

    fn payload_exists(&self, storage_path: &str) -> Result<bool>;
}
