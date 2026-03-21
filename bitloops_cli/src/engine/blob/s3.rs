use std::sync::Arc;

use anyhow::{Context, Result, bail};
use object_store::ObjectStore;
use object_store::aws::AmazonS3Builder;
use object_store::path::Path as ObjectStorePath;

use crate::config::BlobStorageConfig;
use crate::engine::blob::{BlobStore, block_on_blob, normalize_blob_key};

#[derive(Debug, Clone)]
pub struct S3BlobStore {
    store: Arc<dyn ObjectStore>,
}

impl S3BlobStore {
    pub fn from_config(cfg: &BlobStorageConfig) -> Result<Self> {
        let bucket = cfg
            .s3_bucket
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("S3 blob provider requires `s3_bucket`"))?;

        if cfg.s3_access_key_id.is_some() ^ cfg.s3_secret_access_key.is_some() {
            bail!(
                "S3 blob provider requires both `s3_access_key_id` and `s3_secret_access_key` when either is set"
            );
        }

        let mut builder = AmazonS3Builder::from_env().with_bucket_name(bucket);
        if let Some(region) = cfg
            .s3_region
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            builder = builder.with_region(region);
        }

        if let Some(access_key_id) = cfg
            .s3_access_key_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            builder = builder.with_access_key_id(access_key_id);
        }
        if let Some(secret_access_key) = cfg
            .s3_secret_access_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            builder = builder.with_secret_access_key(secret_access_key);
        }

        let store = builder.build().context("building S3 object store client")?;
        Ok(Self {
            store: Arc::new(store),
        })
    }

    fn parse_object_path(key: &str) -> Result<ObjectStorePath> {
        let normalized = normalize_blob_key(key)?;
        ObjectStorePath::parse(normalized).context("parsing S3 blob object key")
    }
}

impl BlobStore for S3BlobStore {
    fn write(&self, key: &str, data: &[u8]) -> Result<()> {
        let object_path = Self::parse_object_path(key)?;
        block_on_blob(async {
            self.store.put(&object_path, data.to_vec().into()).await?;
            Ok(())
        })
        .context("writing blob to S3")
    }

    fn read(&self, key: &str) -> Result<Vec<u8>> {
        let object_path = Self::parse_object_path(key)?;
        block_on_blob(async {
            let response = self.store.get(&object_path).await?;
            let bytes = response.bytes().await?;
            Ok(bytes.to_vec())
        })
        .context("reading blob from S3")
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
        .context("checking blob existence in S3")
    }

    fn delete(&self, key: &str) -> Result<()> {
        let object_path = Self::parse_object_path(key)?;
        block_on_blob(async {
            self.store.delete(&object_path).await?;
            Ok(())
        })
        .context("deleting blob from S3")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BlobStorageProvider;

    fn base_config() -> BlobStorageConfig {
        BlobStorageConfig {
            provider: BlobStorageProvider::S3,
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
    fn s3_blob_store_requires_bucket() {
        let cfg = base_config();
        let err = S3BlobStore::from_config(&cfg).expect_err("missing bucket must fail");
        assert!(err.to_string().contains("s3_bucket"));
    }

    #[test]
    fn s3_blob_store_requires_complete_static_credentials_when_set() {
        let mut cfg = base_config();
        cfg.s3_bucket = Some("test-bucket".to_string());
        cfg.s3_access_key_id = Some("AKIAEXAMPLE".to_string());

        let err = S3BlobStore::from_config(&cfg).expect_err("partial static credentials must fail");
        assert!(err.to_string().contains("both `s3_access_key_id`"));
    }
}
