use std::path::{Path, PathBuf};

use crate::engine::paths;
use crate::test_support::process_state::git_command;

pub(crate) fn git_ok(repo_root: &Path, args: &[&str]) -> String {
    let out = git_command()
        .args(args)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("failed to start git {:?}: {err}", args));
    assert!(
        out.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub(crate) fn init_test_repo(repo_root: &Path, branch: &str, user_name: &str, user_email: &str) {
    git_ok(repo_root, &["init"]);
    git_ok(repo_root, &["checkout", "-B", branch]);
    git_ok(repo_root, &["config", "user.name", user_name]);
    git_ok(repo_root, &["config", "user.email", user_email]);
    git_ok(repo_root, &["config", "commit.gpgsign", "false"]);
}

pub(crate) fn repo_local_blob_root(repo_root: &Path) -> PathBuf {
    repo_root.join(paths::BITLOOPS_DIR).join("blobs")
}
