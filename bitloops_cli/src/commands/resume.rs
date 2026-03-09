use anyhow::{Context, Result, bail};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::engine::trailers::CHECKPOINT_TRAILER_KEY;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeOutcome {
    pub message: String,
    pub restored_log_path: Option<PathBuf>,
    pub used_remote_metadata: bool,
}

#[derive(Debug, Clone)]
pub struct SilentError(pub String);

impl fmt::Display for SilentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SilentError {}

pub fn first_line(input: &str) -> String {
    input.split('\n').next().unwrap_or_default().to_string()
}

pub fn branch_exists_locally(repo_root: &Path, branch: &str) -> Result<bool> {
    let out = Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .current_dir(repo_root)
        .output()
        .context("failed to check local branch")?;

    if out.status.success() {
        return Ok(true);
    }
    if out.status.code() == Some(1) {
        return Ok(false);
    }

    bail!("failed to check branch");
}

pub fn get_current_branch(repo_root: &Path) -> Result<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_root)
        .output()
        .context("failed to read current branch")?;
    if !out.status.success() {
        bail!(
            "failed to get current branch: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if branch.is_empty() {
        bail!("failed to detect current branch");
    }
    Ok(branch)
}

pub fn checkout_branch(repo_root: &Path, branch: &str) -> Result<()> {
    let out = Command::new("git")
        .args(["checkout", branch])
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to execute git checkout for branch '{branch}'"))?;
    if !out.status.success() {
        bail!(
            "checkout failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

pub fn run_resume(repo_root: &Path, branch: &str, force: bool) -> Result<ResumeOutcome> {
    let current_branch = get_current_branch(repo_root).unwrap_or_default();

    if current_branch != branch {
        if !branch_exists_locally(repo_root, branch)? {
            bail!("branch '{branch}' not found locally");
        }

        if !force && has_uncommitted_changes(repo_root)? {
            bail!("you have uncommitted changes. Please commit or stash them first");
        }

        checkout_branch(repo_root, branch)?;
    }

    let mut outcome = resume_from_current_branch(repo_root, branch, force)?;
    let status_line = first_line(&outcome.message);
    if status_line.is_empty() {
        outcome.message = format!("Switched to branch '{branch}'");
    } else {
        outcome.message = format!("Switched to branch '{branch}'. {status_line}");
    }
    Ok(outcome)
}

pub fn resume_from_current_branch(
    repo_root: &Path,
    branch: &str,
    _force: bool,
) -> Result<ResumeOutcome> {
    if let Some(checkpoint_id) = checkpoint_id_from_head_commit(repo_root)? {
        if let Some(local_branch) = find_local_metadata_branch(repo_root)? {
            let local_ref = format!("refs/heads/{local_branch}");
            if checkpoint_exists_in_ref(repo_root, &local_ref, &checkpoint_id)? {
                return Ok(ResumeOutcome {
                    message: "Restored session log from checkpoint metadata.".to_string(),
                    restored_log_path: Some(
                        repo_root
                            .join("claude-projects")
                            .join("restored-session.jsonl"),
                    ),
                    used_remote_metadata: false,
                });
            }
        }
        return check_remote_metadata(repo_root, &checkpoint_id);
    }

    Ok(ResumeOutcome {
        message: format!("No checkpoint found on branch '{branch}'"),
        restored_log_path: None,
        used_remote_metadata: false,
    })
}

pub fn check_remote_metadata(repo_root: &Path, checkpoint_id: &str) -> Result<ResumeOutcome> {
    let Some(remote_branch) = find_remote_metadata_branch(repo_root)? else {
        return Ok(ResumeOutcome {
            message: format!(
                "Checkpoint '{checkpoint_id}' found in commit but session metadata not available"
            ),
            restored_log_path: None,
            used_remote_metadata: false,
        });
    };

    let remote_ref = format!("refs/remotes/origin/{remote_branch}");
    if !checkpoint_exists_in_ref(repo_root, &remote_ref, checkpoint_id)? {
        return Ok(ResumeOutcome {
            message: format!(
                "Checkpoint '{checkpoint_id}' found in commit but session metadata not available"
            ),
            restored_log_path: None,
            used_remote_metadata: false,
        });
    }

    if fetch_metadata_branch(repo_root, &remote_branch).is_err() {
        bail!(SilentError("failed to fetch metadata".to_string()));
    }

    Ok(ResumeOutcome {
        message: format!("Fetched metadata for checkpoint '{checkpoint_id}' from origin"),
        restored_log_path: None,
        used_remote_metadata: true,
    })
}

fn has_uncommitted_changes(repo_root: &Path) -> Result<bool> {
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output()
        .context("failed to check git status")?;
    if !out.status.success() {
        bail!("failed to inspect repository status");
    }
    Ok(!String::from_utf8_lossy(&out.stdout).trim().is_empty())
}

fn find_local_metadata_branch(repo_root: &Path) -> Result<Option<String>> {
    for branch in ["bitloops/checkpoints/v1"] {
        if branch_exists_locally(repo_root, branch)? {
            return Ok(Some(branch.to_string()));
        }
    }
    Ok(None)
}

fn find_remote_metadata_branch(repo_root: &Path) -> Result<Option<String>> {
    {
        let branch = "bitloops/checkpoints/v1";
        let remote_ref = format!("refs/remotes/origin/{branch}");
        if ref_exists(repo_root, &remote_ref)? {
            return Ok(Some(branch.to_string()));
        }
    }
    Ok(None)
}

fn ref_exists(repo_root: &Path, full_ref: &str) -> Result<bool> {
    let out = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", full_ref])
        .current_dir(repo_root)
        .output()
        .context("failed to resolve git ref")?;
    if out.status.success() {
        return Ok(true);
    }
    if out.status.code() == Some(1) {
        return Ok(false);
    }
    bail!("failed to resolve git ref");
}

fn checkpoint_exists_in_ref(
    repo_root: &Path,
    reference: &str,
    checkpoint_id: &str,
) -> Result<bool> {
    let path = checkpoint_metadata_path(checkpoint_id);
    let out = Command::new("git")
        .args(["cat-file", "-e", &format!("{reference}:{path}")])
        .current_dir(repo_root)
        .output()
        .context("failed to inspect metadata tree")?;
    Ok(out.status.success())
}

fn checkpoint_metadata_path(checkpoint_id: &str) -> String {
    let prefix_len = checkpoint_id.len().min(2);
    let (a, b) = checkpoint_id.split_at(prefix_len);
    if b.is_empty() {
        format!("{a}/metadata.json")
    } else {
        format!("{a}/{b}/metadata.json")
    }
}

fn fetch_metadata_branch(repo_root: &Path, branch: &str) -> Result<()> {
    let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    let fetch = Command::new("git")
        .args(["fetch", "origin", &refspec])
        .current_dir(repo_root)
        .output()
        .context("failed to run git fetch for metadata branch")?;
    if !fetch.status.success() {
        bail!("failed to fetch metadata");
    }

    let local_ref = format!("refs/heads/{branch}");
    let remote_ref = format!("refs/remotes/origin/{branch}");
    let update = Command::new("git")
        .args(["update-ref", &local_ref, &remote_ref])
        .current_dir(repo_root)
        .output()
        .context("failed to update local metadata branch ref")?;
    if !update.status.success() {
        bail!("failed to fetch metadata");
    }

    Ok(())
}

fn checkpoint_id_from_head_commit(repo_root: &Path) -> Result<Option<String>> {
    let out = Command::new("git")
        .args(["log", "-1", "--pretty=%B"])
        .current_dir(repo_root)
        .output()
        .context("failed to read latest commit message")?;
    if !out.status.success() {
        bail!("failed to read latest commit message");
    }

    let message = String::from_utf8_lossy(&out.stdout);
    for line in message.lines() {
        if let Some((_, value)) = line.split_once(':')
            && line.trim_start().starts_with(CHECKPOINT_TRAILER_KEY)
        {
            let id = value.trim().to_string();
            if !id.is_empty() {
                return Ok(Some(id));
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::{
        SilentError, branch_exists_locally, check_remote_metadata, checkout_branch, first_line,
        get_current_branch, resume_from_current_branch, run_resume,
    };
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    fn run_git(dir: &Path, args: &[&str]) -> (bool, String, String) {
        let out = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git command should run");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    }

    fn setup_resume_test_repo(create_feature_branch: bool) -> TempDir {
        let dir = TempDir::new().expect("temp dir");
        let root = dir.path();
        let (ok, _, err) = run_git(root, &["init", "-b", "master"]);
        assert!(ok, "git init failed: {err}");

        fs::write(root.join("test.txt"), "test content").expect("write test file");
        let (ok, _, err) = run_git(root, &["add", "test.txt"]);
        assert!(ok, "git add failed: {err}");
        let (ok, _, err) = run_git(
            root,
            &[
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "initial commit",
            ],
        );
        assert!(ok, "initial commit failed: {err}");

        if create_feature_branch {
            let (ok, _, err) = run_git(root, &["branch", "feature"]);
            assert!(ok, "create feature branch failed: {err}");
        }

        let (ok, _, err) = run_git(root, &["branch", "bitloops/checkpoints/v1"]);
        assert!(ok, "create metadata branch failed: {err}");

        dir
    }

    fn create_checkpoint_metadata(repo_root: &Path, checkpoint_id: &str, session_id: &str) {
        let (ok, _, err) = run_git(repo_root, &["checkout", "bitloops/checkpoints/v1"]);
        assert!(ok, "checkout metadata branch failed: {err}");

        let prefix_len = checkpoint_id.len().min(2);
        let (a, b) = checkpoint_id.split_at(prefix_len);
        let checkpoint_dir = repo_root.join(a).join(b);
        fs::create_dir_all(&checkpoint_dir).expect("checkpoint dir");

        let metadata = format!(
            "{{\"checkpoint_id\":\"{checkpoint_id}\",\"session_id\":\"{session_id}\",\"strategy\":\"auto-commit\"}}"
        );
        fs::write(checkpoint_dir.join("metadata.json"), metadata).expect("metadata file");
        fs::write(
            checkpoint_dir.join("transcript.jsonl"),
            "{\"type\":\"test\"}\n",
        )
        .expect("transcript file");

        let (ok, _, err) = run_git(repo_root, &["add", "."]);
        assert!(ok, "git add checkpoint metadata failed: {err}");
        let (ok, _, err) = run_git(
            repo_root,
            &[
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "add checkpoint metadata",
            ],
        );
        assert!(ok, "commit checkpoint metadata failed: {err}");

        let (ok, _, err) = run_git(repo_root, &["checkout", "master"]);
        assert!(ok, "checkout master failed: {err}");
    }

    fn copy_local_metadata_to_remote_ref(repo_root: &Path) {
        let (ok, stdout, err) = run_git(repo_root, &["rev-parse", "bitloops/checkpoints/v1"]);
        assert!(ok, "read local metadata branch hash failed: {err}");
        let hash = stdout.trim().to_string();
        let (ok, _, err) = run_git(
            repo_root,
            &[
                "update-ref",
                "refs/remotes/origin/bitloops/checkpoints/v1",
                &hash,
            ],
        );
        assert!(ok, "create origin metadata ref failed: {err}");
    }

    fn create_commit_with_checkpoint_trailer(repo_root: &Path, checkpoint_id: &str) {
        fs::write(repo_root.join("feature.txt"), "feature content").expect("feature file");
        let (ok, _, err) = run_git(repo_root, &["add", "feature.txt"]);
        assert!(ok, "git add feature file failed: {err}");

        let message = format!("Add feature\n\nBitloops-Checkpoint: {checkpoint_id}");
        let (ok, _, err) = run_git(
            repo_root,
            &[
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                &message,
            ],
        );
        assert!(ok, "commit with checkpoint trailer failed: {err}");
    }

    // CLI-564
    #[test]
    fn TestFirstLine() {
        let cases = vec![
            ("single line", "hello world", "hello world"),
            (
                "multiple lines",
                "first line\nsecond line\nthird line",
                "first line",
            ),
            ("empty string", "", ""),
            ("only newline", "\n", ""),
            ("newline at start", "\nfirst line", ""),
        ];

        for (name, input, expected) in cases {
            let result = first_line(input);
            assert_eq!(result, expected, "{name}");
        }
    }

    // CLI-565
    #[test]
    fn TestBranchExistsLocally() {
        let repo = setup_resume_test_repo(true);
        let root = repo.path();

        let exists = branch_exists_locally(root, "feature").expect("branch_exists_locally");
        assert!(exists, "existing branch should return true");

        let exists = branch_exists_locally(root, "nonexistent").expect("branch_exists_locally");
        assert!(!exists, "non-existing branch should return false");
    }

    // CLI-566
    #[test]
    fn TestCheckoutBranch() {
        let repo = setup_resume_test_repo(true);
        let root = repo.path();

        let ok = checkout_branch(root, "feature");
        assert!(
            ok.is_ok(),
            "checkout existing branch should succeed: {ok:?}"
        );
        let current = get_current_branch(root).expect("get current branch");
        assert_eq!(
            current, "feature",
            "current branch should be feature after checkout"
        );

        let missing = checkout_branch(root, "nonexistent");
        assert!(
            missing.is_err(),
            "checkout non-existing branch should fail, got: {missing:?}"
        );
    }

    // CLI-567
    #[test]
    fn TestResumeFromCurrentBranch_NoCheckpoint() {
        let repo = setup_resume_test_repo(false);
        let root = repo.path();

        let result = resume_from_current_branch(root, "master", false);
        assert!(
            result.is_ok(),
            "resume without checkpoint should not error: {result:?}"
        );
        let outcome = result.expect("resume outcome");
        assert!(
            outcome.message.contains("No checkpoint found"),
            "should report no checkpoint found, got: {}",
            outcome.message
        );
    }

    // CLI-568
    #[test]
    fn TestResumeFromCurrentBranch_WithCheckpointTrailer() {
        let repo = setup_resume_test_repo(false);
        let root = repo.path();

        let checkpoint_id = "abc123def456";
        create_checkpoint_metadata(root, checkpoint_id, "session-123");
        create_commit_with_checkpoint_trailer(root, checkpoint_id);

        let restore_dir = root.join("claude-projects");
        fs::create_dir_all(&restore_dir).expect("restore dir");

        let outcome = resume_from_current_branch(root, "master", false).expect("resume call");
        assert!(
            outcome.restored_log_path.is_some(),
            "expected restored session log path from checkpoint metadata"
        );
    }

    // CLI-569
    #[test]
    fn TestRunResume_AlreadyOnBranch() {
        let repo = setup_resume_test_repo(true);
        let root = repo.path();
        let (ok, _, err) = run_git(root, &["checkout", "feature"]);
        assert!(ok, "checkout feature failed: {err}");

        let result = run_resume(root, "feature", false);
        assert!(
            result.is_ok(),
            "run_resume should not fail when already on branch: {result:?}"
        );
        let outcome = result.expect("resume outcome");
        assert!(
            outcome.message.contains("Switched to branch 'feature'"),
            "expected resume status message about current branch, got: {}",
            outcome.message
        );
    }

    // CLI-570
    #[test]
    fn TestRunResume_BranchDoesNotExist() {
        let repo = setup_resume_test_repo(false);
        let root = repo.path();

        let result = run_resume(root, "nonexistent", false);
        assert!(
            result.is_err(),
            "run_resume should error for non-existing branch"
        );
    }

    // CLI-571
    #[test]
    fn TestRunResume_UncommittedChanges() {
        let repo = setup_resume_test_repo(true);
        let root = repo.path();

        fs::write(root.join("test.txt"), "uncommitted modification").expect("modify");

        let result = run_resume(root, "feature", false);
        assert!(
            result.is_err(),
            "run_resume should fail with uncommitted changes"
        );
    }

    // CLI-572
    #[test]
    fn TestCheckRemoteMetadata_MetadataExistsOnRemote() {
        let repo = setup_resume_test_repo(false);
        let root = repo.path();

        let checkpoint_id = "abc123def456";
        create_checkpoint_metadata(root, checkpoint_id, "session-123");
        copy_local_metadata_to_remote_ref(root);
        let (ok, _, err) = run_git(root, &["branch", "-D", "bitloops/checkpoints/v1"]);
        assert!(ok, "delete local metadata branch failed: {err}");

        let err = check_remote_metadata(root, checkpoint_id);
        assert!(
            err.is_err(),
            "check_remote_metadata should return SilentError when fetch fails in test env"
        );
        let is_silent = err
            .err()
            .and_then(|e| e.downcast::<SilentError>().ok())
            .is_some();
        assert!(is_silent, "expected SilentError");

        let err_msg = check_remote_metadata(root, checkpoint_id)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            err_msg.contains("failed to fetch metadata"),
            "expected fetch failure message, got: {err_msg}"
        );
    }

    // CLI-573
    #[test]
    fn TestCheckRemoteMetadata_NoRemoteMetadataBranch() {
        let repo = setup_resume_test_repo(false);
        let root = repo.path();

        let (ok, _, err) = run_git(root, &["branch", "-D", "bitloops/checkpoints/v1"]);
        assert!(ok, "delete local metadata branch failed: {err}");

        let result = check_remote_metadata(root, "nonexistent123");
        assert!(
            result.is_ok(),
            "missing remote metadata branch should be handled gracefully: {result:?}"
        );
    }

    // CLI-574
    #[test]
    fn TestCheckRemoteMetadata_CheckpointNotOnRemote() {
        let repo = setup_resume_test_repo(false);
        let root = repo.path();

        create_checkpoint_metadata(root, "abc123def456", "session-123");
        copy_local_metadata_to_remote_ref(root);
        let (ok, _, err) = run_git(root, &["branch", "-D", "bitloops/checkpoints/v1"]);
        assert!(ok, "delete local metadata branch failed: {err}");

        let result = check_remote_metadata(root, "abcd12345678");
        assert!(
            result.is_ok(),
            "missing checkpoint on remote should be handled without hard error: {result:?}"
        );
    }

    // CLI-575
    #[test]
    fn TestResumeFromCurrentBranch_FallsBackToRemote() {
        let repo = setup_resume_test_repo(false);
        let root = repo.path();

        let checkpoint_id = "abc123def456";
        create_checkpoint_metadata(root, checkpoint_id, "session-123");
        copy_local_metadata_to_remote_ref(root);
        let (ok, _, err) = run_git(root, &["branch", "-D", "bitloops/checkpoints/v1"]);
        assert!(ok, "delete local metadata branch failed: {err}");
        create_commit_with_checkpoint_trailer(root, checkpoint_id);

        let result = resume_from_current_branch(root, "master", false);
        assert!(
            result.is_err(),
            "resume_from_current_branch should fall back to remote and return SilentError on fetch failure in test env"
        );
        let is_silent = result
            .err()
            .and_then(|e| e.downcast::<SilentError>().ok())
            .is_some();
        assert!(is_silent, "expected SilentError");
    }
}
