use super::*;

// Language detection and git blob access utilities.

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

pub(super) fn git_blob_content(repo_root: &Path, blob_sha: &str) -> Option<String> {
    run_git(repo_root, &["cat-file", "-p", blob_sha]).ok()
}

pub(super) fn git_blob_line_count(repo_root: &Path, blob_sha: &str) -> Option<i32> {
    let output = git_blob_content(repo_root, blob_sha)?;
    if output.is_empty() {
        return Some(1);
    }
    let mut count = output.lines().count() as i32;
    if !output.ends_with('\n') {
        count += 1;
    }
    Some(count.max(1))
}

pub(super) fn detect_language(path: &str) -> String {
    indexing_language_for_path(path)
}
