use super::*;

pub(crate) fn commit_file(repo_root: &Path, filename: &str, content: &str) {
    fs::write(repo_root.join(filename), content).unwrap();
    git_ok(repo_root, &["add", filename]);
    git_ok(repo_root, &["commit", "-m", "test commit"]);
}
