use std::path::{Path, PathBuf};

use async_graphql::ID;
use sha2::{Digest, Sha256};

pub(super) fn target_id(kind: &str, path: &Path) -> ID {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    hasher.update(b"\n");
    hasher.update(path.to_string_lossy().as_bytes());
    ID::from(hex_digest(hasher.finalize().as_slice()))
}

pub(super) fn revision_for_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_digest(hasher.finalize().as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

pub(super) fn canonicalize_lossy(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
