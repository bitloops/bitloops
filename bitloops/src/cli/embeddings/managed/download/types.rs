//! Shared data types for the managed runtime download pipeline.

use std::fs;
use std::path::{Path, PathBuf};

/// A managed runtime asset that has been downloaded into a temporary file.
///
/// The file is removed automatically when the value is dropped, so callers
/// must move it to its final location before the value goes out of scope.
#[derive(Debug)]
pub(crate) struct DownloadedManagedAsset {
    pub(crate) path: PathBuf,
    pub(crate) bytes_downloaded: u64,
    pub(crate) bytes_total: Option<u64>,
    pub(crate) sha256_hex: String,
}

impl DownloadedManagedAsset {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DownloadedManagedAsset {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// An inclusive byte range used for HTTP `Range` requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DownloadByteRange {
    pub(crate) start: u64,
    pub(crate) end: u64,
}

impl DownloadByteRange {
    pub(crate) fn len(self) -> u64 {
        self.end.saturating_sub(self.start).saturating_add(1)
    }
}
