use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::host::checkpoints::strategy::manual_commit::{
    read_commit_checkpoint_mappings, read_latest_session_content,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeOutcome {
    pub message: String,
    pub restored_log_path: Option<PathBuf>,
    pub used_remote_metadata: bool,
}

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
    let head_sha = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(repo_root)
        .output()
        .context("failed to resolve HEAD commit")?;
    if !head_sha.status.success() {
        bail!(
            "failed to resolve HEAD commit: {}",
            String::from_utf8_lossy(&head_sha.stderr).trim()
        );
    }
    let head_sha = String::from_utf8_lossy(&head_sha.stdout).trim().to_string();

    let mappings = read_commit_checkpoint_mappings(repo_root)?;
    let Some(checkpoint_id) = mappings.get(&head_sha) else {
        return Ok(ResumeOutcome {
            message: format!("No checkpoint mapping found on branch '{branch}'"),
            restored_log_path: None,
            used_remote_metadata: false,
        });
    };

    let content = read_latest_session_content(repo_root, checkpoint_id).with_context(|| {
        format!("failed to read checkpoint {checkpoint_id} session content from storage")
    })?;
    let transcript = content.transcript;
    if transcript.trim().is_empty() {
        return Ok(ResumeOutcome {
            message: format!("Checkpoint '{checkpoint_id}' has no transcript content"),
            restored_log_path: None,
            used_remote_metadata: false,
        });
    }

    let restore_path = repo_root
        .join("claude-projects")
        .join("restored-session.jsonl");
    if let Some(parent) = restore_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create restore directory {}", parent.display()))?;
    }
    fs::write(&restore_path, transcript.as_bytes()).with_context(|| {
        format!(
            "failed to write restored transcript {}",
            restore_path.display()
        )
    })?;

    Ok(ResumeOutcome {
        message: format!("Restored session log from checkpoint '{checkpoint_id}'."),
        restored_log_path: Some(restore_path),
        used_remote_metadata: false,
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

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::{
        branch_exists_locally, checkout_branch, first_line, get_current_branch,
        resume_from_current_branch, run_resume,
    };
    use crate::config::{resolve_sqlite_db_path_for_repo, resolve_store_backend_config_for_repo};
    use crate::storage::SqliteConnectionPool;
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

    fn checkpoint_sqlite_path(repo_root: &Path) -> std::path::PathBuf {
        let cfg = resolve_store_backend_config_for_repo(repo_root).expect("resolve backend config");
        if let Some(path) = cfg.relational.sqlite_path.as_deref() {
            resolve_sqlite_db_path_for_repo(repo_root, Some(path))
                .expect("resolve configured sqlite path")
        } else {
            crate::utils::paths::default_relational_db_path(repo_root)
        }
    }

    fn ensure_relational_store_file(repo_root: &Path) {
        let sqlite = SqliteConnectionPool::connect(checkpoint_sqlite_path(repo_root))
            .expect("create relational sqlite file");
        sqlite
            .initialise_checkpoint_schema()
            .expect("initialise checkpoint schema");
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
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "initial commit",
            ],
        );
        assert!(ok, "initial commit failed: {err}");
        ensure_relational_store_file(root);

        if create_feature_branch {
            let (ok, _, err) = run_git(root, &["branch", "feature"]);
            assert!(ok, "create feature branch failed: {err}");
        }

        let (ok, _, err) = run_git(root, &["branch", "bitloops/checkpoints/v1"]);
        assert!(ok, "create metadata branch failed: {err}");

        dir
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
                "-c",
                "commit.gpgsign=false",
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
            outcome.message.contains("No checkpoint mapping found"),
            "should report missing checkpoint mapping, got: {}",
            outcome.message
        );
    }

    // CLI-568
    #[test]
    fn TestResumeFromCurrentBranch_WithCheckpointTrailer() {
        let repo = setup_resume_test_repo(false);
        let root = repo.path();

        let checkpoint_id = "abc123def456";
        create_commit_with_checkpoint_trailer(root, checkpoint_id);

        let restore_dir = root.join("claude-projects");
        fs::create_dir_all(&restore_dir).expect("restore dir");

        let outcome = resume_from_current_branch(root, "master", false).expect("resume call");
        assert!(
            outcome.restored_log_path.is_none(),
            "expected no restored session log path without DB commit mapping"
        );
        assert!(
            outcome.message.contains("No checkpoint mapping found"),
            "expected missing mapping message, got: {}",
            outcome.message
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
}
