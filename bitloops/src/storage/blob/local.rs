use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{BlobStore, normalize_blob_key};
use crate::config::BlobStorageConfig;

#[derive(Debug, Clone)]
pub struct LocalBlobStore {
    root: PathBuf,
}

impl LocalBlobStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)
            .with_context(|| format!("creating local blob root directory {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn from_config(cfg: &BlobStorageConfig) -> Result<Self> {
        let root = cfg.local_path_or_default()?;
        Self::new(root)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn resolve_path(&self, key: &str) -> Result<PathBuf> {
        let normalized = normalize_blob_key(key)?;
        Ok(self.root.join(normalized))
    }
}

impl BlobStore for LocalBlobStore {
    fn write(&self, key: &str, data: &[u8]) -> Result<()> {
        let path = self.resolve_path(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating blob parent directory {}", parent.display()))?;
        }
        fs::write(&path, data).with_context(|| format!("writing blob file {}", path.display()))
    }

    fn read(&self, key: &str) -> Result<Vec<u8>> {
        let path = self.resolve_path(key)?;
        fs::read(&path).with_context(|| format!("reading blob file {}", path.display()))
    }

    fn exists(&self, key: &str) -> Result<bool> {
        let path = self.resolve_path(key)?;
        Ok(path.exists())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let path = self.resolve_path(key)?;
        if !path.exists() {
            return Ok(());
        }
        fs::remove_file(&path).with_context(|| format!("deleting blob file {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn local_blob_store_roundtrip_write_read_exists() {
        let temp = TempDir::new().expect("temp dir");
        let store = LocalBlobStore::new(temp.path()).expect("local blob store");

        let key = "repo-1/cp-1/0/transcript.jsonl";
        let payload = b"{\"event\":\"hello\"}\n";

        store.write(key, payload).expect("write blob");
        assert!(store.exists(key).expect("exists check"));
        let loaded = store.read(key).expect("read blob");

        assert_eq!(loaded, payload);
    }

    #[test]
    fn local_blob_store_rejects_parent_path_traversal() {
        let temp = TempDir::new().expect("temp dir");
        let store = LocalBlobStore::new(temp.path()).expect("local blob store");

        let err = store
            .write("../outside.txt", b"nope")
            .expect_err("must reject traversal");
        assert!(err.to_string().contains("parent path traversal"));
    }

    #[test]
    fn local_blob_store_delete_removes_blob() {
        let temp = TempDir::new().expect("temp dir");
        let store = LocalBlobStore::new(temp.path()).expect("local blob store");

        let key = "repo-1/cp-1/0/transcript.jsonl";
        store.write(key, b"payload").expect("write blob");
        assert!(store.exists(key).expect("exists before delete"));

        store.delete(key).expect("delete blob");
        assert!(!store.exists(key).expect("exists after delete"));
    }
}
