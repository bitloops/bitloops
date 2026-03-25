use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobPayloadRef {
    pub storage_backend: String,
    pub storage_path: String,
    pub mime_type: String,
    pub size_bytes: i64,
}

pub trait BlobPayloadGateway: Send + Sync {
    fn write_payload(&self, key: &str, bytes: &[u8]) -> Result<BlobPayloadRef>;

    fn delete_payload(&self, payload: &BlobPayloadRef) -> Result<()>;

    fn payload_exists(&self, storage_path: &str) -> Result<bool>;
}
