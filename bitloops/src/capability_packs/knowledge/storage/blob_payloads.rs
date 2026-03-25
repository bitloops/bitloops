use anyhow::{Context, Result};

use crate::config::StoreBackendConfig;
use crate::host::capability_host::gateways::{BlobPayloadGateway, BlobPayloadRef};
use crate::storage::blob::{BlobStore, create_blob_store_with_backend_for_repo};

use super::models::KnowledgePayloadRef;

pub struct BlobKnowledgePayloadStore {
    store: Box<dyn BlobStore>,
    backend: String,
}

impl BlobKnowledgePayloadStore {
    pub fn from_backend_config(
        repo_root: &std::path::Path,
        cfg: &StoreBackendConfig,
    ) -> Result<Self> {
        let resolved = create_blob_store_with_backend_for_repo(&cfg.blobs, repo_root)
            .context("initialising knowledge payload blob store")?;
        Ok(Self {
            store: resolved.store,
            backend: resolved.backend.to_string(),
        })
    }

    pub fn write_payload(&self, key: &str, bytes: &[u8]) -> Result<KnowledgePayloadRef> {
        self.store
            .write(key, bytes)
            .context("writing knowledge payload blob")?;
        Ok(KnowledgePayloadRef {
            storage_backend: self.backend.clone(),
            storage_path: key.to_string(),
            mime_type: "application/json".to_string(),
            size_bytes: bytes.len() as i64,
        })
    }

    pub fn delete_payload(&self, payload: &KnowledgePayloadRef) -> Result<()> {
        self.store
            .delete(&payload.storage_path)
            .context("deleting knowledge payload blob")
    }

    pub fn payload_exists(&self, storage_path: &str) -> Result<bool> {
        self.store
            .exists(storage_path)
            .context("checking knowledge payload blob existence")
    }
}

impl BlobPayloadGateway for BlobKnowledgePayloadStore {
    fn write_payload(&self, key: &str, bytes: &[u8]) -> Result<BlobPayloadRef> {
        BlobKnowledgePayloadStore::write_payload(self, key, bytes)
    }

    fn delete_payload(&self, payload: &BlobPayloadRef) -> Result<()> {
        BlobKnowledgePayloadStore::delete_payload(self, payload)
    }

    fn payload_exists(&self, storage_path: &str) -> Result<bool> {
        BlobKnowledgePayloadStore::payload_exists(self, storage_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        BlobStorageConfig, EventsBackendConfig, RelationalBackendConfig, StoreBackendConfig,
    };
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn blob_payload_store_uses_local_backend() {
        let temp = TempDir::new().expect("temp dir");
        let repo_root = temp.path().join("repo");
        fs::create_dir_all(&repo_root).expect("repo root");
        let backends = StoreBackendConfig {
            relational: RelationalBackendConfig {
                sqlite_path: Some(
                    temp.path()
                        .join("relational.db")
                        .to_string_lossy()
                        .to_string(),
                ),
                postgres_dsn: None,
            },
            events: EventsBackendConfig {
                duckdb_path: Some(
                    temp.path()
                        .join("events.duckdb")
                        .to_string_lossy()
                        .to_string(),
                ),
                clickhouse_url: None,
                clickhouse_user: None,
                clickhouse_password: None,
                clickhouse_database: None,
            },
            blobs: BlobStorageConfig {
                local_path: Some(temp.path().join("blobs").to_string_lossy().to_string()),
                s3_bucket: None,
                s3_region: None,
                s3_access_key_id: None,
                s3_secret_access_key: None,
                gcs_bucket: None,
                gcs_credentials_path: None,
            },
        };
        let store =
            BlobKnowledgePayloadStore::from_backend_config(&repo_root, &backends).expect("store");
        let payload = store
            .write_payload(
                "knowledge/repo-1/item-1/version-1/payload.json",
                b"{\"ok\":true}",
            )
            .expect("write payload");

        assert!(store.payload_exists(&payload.storage_path).expect("exists"));

        store.delete_payload(&payload).expect("delete payload");
        assert!(
            !store
                .payload_exists(&payload.storage_path)
                .expect("exists after delete")
        );
    }
}
