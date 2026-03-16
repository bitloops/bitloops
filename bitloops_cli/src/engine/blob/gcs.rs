use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use object_store::ObjectStore;
use object_store::gcp::GoogleCloudStorageBuilder;
use object_store::path::Path as ObjectStorePath;

use crate::devql_config::BlobStorageConfig;
use crate::engine::blob::{BlobStore, block_on_blob, normalize_blob_key};

#[derive(Debug, Clone)]
pub struct GcsBlobStore {
    store: Arc<dyn ObjectStore>,
}

impl GcsBlobStore {
    pub fn from_config(cfg: &BlobStorageConfig) -> Result<Self> {
        let bucket = cfg
            .gcs_bucket
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("GCS blob provider requires `gcs_bucket`"))?;

        let mut builder = GoogleCloudStorageBuilder::from_env().with_bucket_name(bucket);
        if let Some(credentials_path) = cfg
            .gcs_credentials_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            builder = builder.with_application_credentials(credentials_path);
        }

        let store = builder
            .build()
            .context("building GCS object store client")?;
        Ok(Self {
            store: Arc::new(store),
        })
    }

    fn parse_object_path(key: &str) -> Result<ObjectStorePath> {
        let normalized = normalize_blob_key(key)?;
        ObjectStorePath::parse(normalized).context("parsing GCS blob object key")
    }
}

impl BlobStore for GcsBlobStore {
    fn write(&self, key: &str, data: &[u8]) -> Result<()> {
        let object_path = Self::parse_object_path(key)?;
        block_on_blob(async {
            self.store.put(&object_path, data.to_vec().into()).await?;
            Ok(())
        })
        .context("writing blob to GCS")
    }

    fn read(&self, key: &str) -> Result<Vec<u8>> {
        let object_path = Self::parse_object_path(key)?;
        block_on_blob(async {
            let response = self.store.get(&object_path).await?;
            let bytes = response.bytes().await?;
            Ok(bytes.to_vec())
        })
        .context("reading blob from GCS")
    }

    fn exists(&self, key: &str) -> Result<bool> {
        let object_path = Self::parse_object_path(key)?;
        block_on_blob(async {
            match self.store.head(&object_path).await {
                Ok(_) => Ok(true),
                Err(object_store::Error::NotFound { .. }) => Ok(false),
                Err(err) => Err(err.into()),
            }
        })
        .context("checking blob existence in GCS")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devql_config::BlobStorageProvider;

    fn base_config() -> BlobStorageConfig {
        BlobStorageConfig {
            provider: BlobStorageProvider::Gcs,
            local_path: None,
            s3_bucket: None,
            s3_region: None,
            s3_access_key_id: None,
            s3_secret_access_key: None,
            gcs_bucket: None,
            gcs_credentials_path: None,
        }
    }

    #[test]
    fn gcs_blob_store_requires_bucket() {
        let cfg = base_config();
        let err = GcsBlobStore::from_config(&cfg).expect_err("missing bucket must fail");
        assert!(err.to_string().contains("gcs_bucket"));
    }
}
