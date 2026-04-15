use super::*;

// Language detection and git blob access utilities.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DecodedFileContent {
    pub(super) raw_bytes: Vec<u8>,
    pub(super) text: Option<String>,
    pub(super) decode_degraded: bool,
}

impl DecodedFileContent {
    pub(super) fn from_raw_bytes(raw_bytes: Vec<u8>) -> Self {
        let text = std::str::from_utf8(&raw_bytes).ok().map(str::to_owned);
        let decode_degraded = text.is_none();
        Self {
            raw_bytes,
            text,
            decode_degraded,
        }
    }

    pub(super) fn line_count(&self) -> i32 {
        line_count_from_bytes(&self.raw_bytes)
    }

    pub(super) fn byte_count(&self) -> i32 {
        i32::try_from(self.raw_bytes.len()).unwrap_or(i32::MAX)
    }
}

pub(super) fn git_blob_sha_at_commit(
    repo_root: &Path,
    commit_sha: &str,
    path: &str,
) -> Option<String> {
    let spec = format!("{commit_sha}:{path}");
    run_git(repo_root, &["rev-parse", &spec])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub(super) fn git_blob_bytes(repo_root: &Path, blob_sha: &str) -> Option<Vec<u8>> {
    run_git_bytes(repo_root, &["cat-file", "-p", blob_sha]).ok()
}

pub(super) fn git_blob_decoded_content(
    repo_root: &Path,
    blob_sha: &str,
) -> Option<DecodedFileContent> {
    git_blob_bytes(repo_root, blob_sha).map(DecodedFileContent::from_raw_bytes)
}

pub(super) fn detect_language(path: &str) -> String {
    indexing_language_for_path(path)
}

pub(super) fn line_count_from_bytes(raw_bytes: &[u8]) -> i32 {
    if raw_bytes.is_empty() {
        return 1;
    }

    let mut count = raw_bytes.iter().filter(|byte| **byte == b'\n').count() as i32;
    if !raw_bytes.ends_with(b"\n") {
        count += 1;
    }
    count.max(1)
}
