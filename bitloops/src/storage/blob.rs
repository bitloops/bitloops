mod gcs;
mod local;
mod s3;

pub use gcs::GcsBlobStore;
pub use local::LocalBlobStore;
pub use s3::S3BlobStore;

use std::cell::RefCell;
use std::future::Future;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use tokio::runtime::{Builder, Runtime};

use crate::config::{BlobStorageConfig, StoreBackendConfig, resolve_blob_local_path_for_repo};
use crate::storage::SqliteConnectionPool;

thread_local! {
    static BLOB_SYNC_RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

pub trait BlobStore: Send + Sync {
    fn write(&self, key: &str, data: &[u8]) -> Result<()>;
    fn read(&self, key: &str) -> Result<Vec<u8>>;
    fn exists(&self, key: &str) -> Result<bool>;
    fn delete(&self, key: &str) -> Result<()>;
}

pub struct ResolvedBlobStore {
    pub store: Box<dyn BlobStore>,
    pub backend: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobType {
    Transcript,
    Prompts,
    Context,
}

impl BlobType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transcript => "transcript",
            Self::Prompts => "prompts",
            Self::Context => "context",
        }
    }

    pub const fn default_file_name(self) -> &'static str {
        match self {
            Self::Transcript => "transcript.jsonl",
            Self::Prompts => "prompts.txt",
            Self::Context => "context.md",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointBlobReference {
    pub blob_id: String,
    pub checkpoint_id: String,
    pub session_index: i64,
    pub blob_type: String,
    pub storage_backend: String,
    pub storage_path: String,
    pub content_hash: String,
    pub size_bytes: i64,
}

impl CheckpointBlobReference {
    pub fn new(
        checkpoint_id: impl Into<String>,
        session_index: i64,
        blob_type: BlobType,
        storage_backend: impl Into<String>,
        storage_path: impl Into<String>,
        content_hash: impl Into<String>,
        size_bytes: i64,
    ) -> Self {
        let checkpoint_id = checkpoint_id.into();
        let blob_type_str = blob_type.as_str().to_string();
        Self {
            blob_id: format!("{checkpoint_id}/{session_index}/{}", blob_type_str),
            checkpoint_id,
            session_index,
            blob_type: blob_type_str,
            storage_backend: storage_backend.into(),
            storage_path: storage_path.into(),
            content_hash: content_hash.into(),
            size_bytes,
        }
    }
}

pub fn build_blob_key(
    repo_id: &str,
    checkpoint_id: &str,
    session_index: i64,
    blob_type: BlobType,
) -> String {
    format!(
        "{repo_id}/{checkpoint_id}/{session_index}/{}",
        blob_type.default_file_name()
    )
}

pub fn create_blob_store_from_backend_config(
    cfg: &StoreBackendConfig,
) -> Result<Box<dyn BlobStore>> {
    create_blob_store(&cfg.blobs)
}

pub fn create_blob_store(cfg: &BlobStorageConfig) -> Result<Box<dyn BlobStore>> {
    Ok(create_blob_store_with_backend(cfg)?.store)
}

fn reject_conflicting_remote_blob_backends(cfg: &BlobStorageConfig) -> Result<()> {
    if cfg.s3_bucket.is_some() && cfg.gcs_bucket.is_some() {
        bail!(
            "blob storage configuration conflict: both s3_bucket and gcs_bucket are set; \
             configure exactly one remote backend (or neither for local storage)"
        );
    }
    Ok(())
}

pub fn create_blob_store_with_backend(cfg: &BlobStorageConfig) -> Result<ResolvedBlobStore> {
    reject_conflicting_remote_blob_backends(cfg)?;
    if cfg.s3_bucket.is_some() {
        Ok(ResolvedBlobStore {
            store: Box::new(
                S3BlobStore::from_config(cfg).context("initialising S3 blob storage backend")?,
            ),
            backend: "s3",
        })
    } else if cfg.gcs_bucket.is_some() {
        Ok(ResolvedBlobStore {
            store: Box::new(
                GcsBlobStore::from_config(cfg).context("initialising GCS blob storage backend")?,
            ),
            backend: "gcs",
        })
    } else {
        Ok(ResolvedBlobStore {
            store: Box::new(LocalBlobStore::from_config(cfg)?),
            backend: "local",
        })
    }
}

pub fn create_blob_store_with_backend_for_repo(
    cfg: &BlobStorageConfig,
    repo_root: &Path,
) -> Result<ResolvedBlobStore> {
    reject_conflicting_remote_blob_backends(cfg)?;
    if cfg.s3_bucket.is_some() {
        Ok(ResolvedBlobStore {
            store: Box::new(
                S3BlobStore::from_config(cfg).context("initialising S3 blob storage backend")?,
            ),
            backend: "s3",
        })
    } else if cfg.gcs_bucket.is_some() {
        Ok(ResolvedBlobStore {
            store: Box::new(
                GcsBlobStore::from_config(cfg).context("initialising GCS blob storage backend")?,
            ),
            backend: "gcs",
        })
    } else {
        let root = resolve_blob_local_path_for_repo(repo_root, cfg.local_path.as_deref())
            .context("resolving local blob store path for repository")?;
        Ok(ResolvedBlobStore {
            store: Box::new(LocalBlobStore::new(root)?),
            backend: "local",
        })
    }
}

pub fn upsert_checkpoint_blob_reference(
    sqlite: &SqliteConnectionPool,
    reference: &CheckpointBlobReference,
) -> Result<()> {
    sqlite.with_connection(|conn| {
        conn.execute(
            "INSERT INTO checkpoint_blobs (
                blob_id, checkpoint_id, session_index, blob_type, storage_backend,
                storage_path, content_hash, size_bytes, created_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
             ON CONFLICT(blob_id) DO UPDATE SET
                checkpoint_id = excluded.checkpoint_id,
                session_index = excluded.session_index,
                blob_type = excluded.blob_type,
                storage_backend = excluded.storage_backend,
                storage_path = excluded.storage_path,
                content_hash = excluded.content_hash,
                size_bytes = excluded.size_bytes",
            params![
                reference.blob_id.as_str(),
                reference.checkpoint_id.as_str(),
                reference.session_index,
                reference.blob_type.as_str(),
                reference.storage_backend.as_str(),
                reference.storage_path.as_str(),
                reference.content_hash.as_str(),
                reference.size_bytes,
            ],
        )
        .context("upserting checkpoint blob reference row")?;
        Ok(())
    })
}

pub fn load_checkpoint_blob_reference(
    sqlite: &SqliteConnectionPool,
    checkpoint_id: &str,
    session_index: i64,
    blob_type: &str,
) -> Result<Option<CheckpointBlobReference>> {
    sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT blob_id, checkpoint_id, session_index, blob_type, storage_backend, storage_path, content_hash, size_bytes
             FROM checkpoint_blobs
             WHERE checkpoint_id = ?1 AND session_index = ?2 AND blob_type = ?3
             LIMIT 1",
            params![checkpoint_id, session_index, blob_type],
            |row| {
                Ok(CheckpointBlobReference {
                    blob_id: row.get(0)?,
                    checkpoint_id: row.get(1)?,
                    session_index: row.get(2)?,
                    blob_type: row.get(3)?,
                    storage_backend: row.get(4)?,
                    storage_path: row.get(5)?,
                    content_hash: row.get(6)?,
                    size_bytes: row.get(7)?,
                })
            },
        )
        .optional()
        .context("loading checkpoint blob reference row")
    })
}

pub(crate) fn normalize_blob_key(raw_key: &str) -> Result<String> {
    let normalized = raw_key.trim().replace('\\', "/");
    let trimmed = normalized.trim_start_matches('/');
    if trimmed.is_empty() {
        bail!("blob key must not be empty");
    }

    let mut segments: Vec<&str> = Vec::new();
    for segment in trimmed.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            bail!("blob key must not contain parent path traversal");
        }
        segments.push(segment);
    }

    if segments.is_empty() {
        bail!("blob key must not be empty");
    }

    Ok(segments.join("/"))
}

pub(crate) fn block_on_blob<T>(future: impl Future<Output = Result<T>>) -> Result<T> {
    with_blob_runtime(|runtime| runtime.block_on(future))
}

fn with_blob_runtime<T>(operation: impl FnOnce(&Runtime) -> Result<T>) -> Result<T> {
    BLOB_SYNC_RUNTIME.with(|runtime_slot| {
        if runtime_slot.borrow().is_none() {
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| anyhow!("creating blob storage runtime: {err:#}"))?;
            *runtime_slot.borrow_mut() = Some(runtime);
        }

        let runtime_borrow = runtime_slot.borrow();
        let runtime = runtime_borrow
            .as_ref()
            .ok_or_else(|| anyhow!("blob storage runtime was not initialised"))?;
        operation(runtime)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SqliteConnectionPool;
    use tempfile::TempDir;

    fn test_blob_config(local_path: String) -> BlobStorageConfig {
        BlobStorageConfig {
            local_path: Some(local_path),
            s3_bucket: None,
            s3_region: None,
            s3_access_key_id: None,
            s3_secret_access_key: None,
            gcs_bucket: None,
            gcs_credentials_path: None,
        }
    }

    #[test]
    fn normalize_blob_key_rejects_parent_traversal() {
        let err = normalize_blob_key("../secret.txt").expect_err("must reject parent traversal");
        assert!(err.to_string().contains("parent path traversal"));
    }

    #[test]
    fn create_blob_store_dispatches_to_s3_when_bucket_set() {
        let temp = TempDir::new().expect("temp dir");
        let mut cfg = test_blob_config(temp.path().to_string_lossy().to_string());
        cfg.s3_bucket = Some("test-bucket".to_string());

        let resolved = create_blob_store_with_backend(&cfg).expect("S3 dispatch should succeed");
        assert_eq!(resolved.backend, "s3");
    }

    #[test]
    fn create_blob_store_dispatches_to_gcs_when_bucket_set() {
        let temp = TempDir::new().expect("temp dir");
        let mut cfg = test_blob_config(temp.path().to_string_lossy().to_string());
        cfg.gcs_bucket = Some("test-bucket".to_string());

        let resolved = create_blob_store_with_backend(&cfg).expect("GCS dispatch should succeed");
        assert_eq!(resolved.backend, "gcs");
    }

    #[test]
    fn create_blob_store_rejects_both_s3_and_gcs() {
        let temp = TempDir::new().expect("temp dir");
        let mut cfg = test_blob_config(temp.path().to_string_lossy().to_string());
        cfg.s3_bucket = Some("s3-bucket".to_string());
        cfg.gcs_bucket = Some("gcs-bucket".to_string());

        let err = create_blob_store_with_backend(&cfg)
            .err()
            .expect("should reject conflicting remote backends");
        assert!(
            err.to_string().contains("s3_bucket") && err.to_string().contains("gcs_bucket"),
            "error should name the conflicting fields, got: {err}"
        );
    }

    #[test]
    fn checkpoint_blob_reference_roundtrip_in_sqlite() {
        let temp = TempDir::new().expect("temp dir");
        let sqlite_path = temp.path().join("checkpoints.sqlite");
        let sqlite = SqliteConnectionPool::connect(sqlite_path).expect("connect sqlite");
        sqlite
            .initialise_checkpoint_schema()
            .expect("initialise schema");

        let reference = CheckpointBlobReference::new(
            "cp-123",
            0,
            BlobType::Transcript,
            "local",
            "repo/cp-123/0/transcript.jsonl",
            "sha256:abc",
            42,
        );

        upsert_checkpoint_blob_reference(&sqlite, &reference).expect("upsert reference");
        let loaded = load_checkpoint_blob_reference(&sqlite, "cp-123", 0, "transcript")
            .expect("load reference")
            .expect("reference should exist");

        assert_eq!(loaded, reference);
    }
}
