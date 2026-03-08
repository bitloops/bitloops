use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetConfig {
    pub repo_root: PathBuf,
    pub force: bool,
    pub session_id: Option<String>,
    pub strategy_name: String,
}

pub fn run_reset_cmd(config: &ResetConfig) -> Result<()> {
    if config.strategy_name != "manual-commit" {
        bail!("strategy {} does not support reset", config.strategy_name);
    }

    ensure_git_repository(&config.repo_root)?;

    if !config.force {
        return Ok(());
    }

    cleanup_session_files(&config.repo_root, config.session_id.as_deref())?;
    cleanup_shadow_branches(&config.repo_root)?;
    Ok(())
}

fn ensure_git_repository(repo_root: &Path) -> Result<()> {
    let out = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(repo_root)
        .output()
        .context("failed to check git repository")?;

    if !out.status.success() {
        bail!("not a git repository");
    }

    Ok(())
}

fn cleanup_session_files(repo_root: &Path, target_session_id: Option<&str>) -> Result<()> {
    let session_dir = repo_root.join(".git").join("bitloops-sessions");
    if !session_dir.exists() {
        return Ok(());
    }

    if let Some(session_id) = target_session_id {
        let file = session_dir.join(format!("{session_id}.json"));
        if file.exists() {
            fs::remove_file(&file)
                .with_context(|| format!("failed to delete session state {}", file.display()))?;
        }
        return Ok(());
    }

    for entry in fs::read_dir(&session_dir)
        .with_context(|| format!("failed to read session directory {}", session_dir.display()))?
    {
        let entry = entry.context("failed to read session directory entry")?;
        let path = entry.path();
        if path.is_file() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to delete session state {}", path.display()))?;
        }
    }

    Ok(())
}

fn cleanup_shadow_branches(repo_root: &Path) -> Result<()> {
    let out = Command::new("git")
        .args(["for-each-ref", "--format=%(refname:short)", "refs/heads"])
        .current_dir(repo_root)
        .output()
        .context("failed to list git branches")?;
    if !out.status.success() {
        bail!(
            "failed to list git branches: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    for branch in String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| is_shadow_branch(name))
    {
        let deleted = Command::new("git")
            .args(["branch", "-D", branch])
            .current_dir(repo_root)
            .output()
            .with_context(|| format!("failed to delete branch {branch}"))?;
        if !deleted.status.success() {
            bail!(
                "failed to delete branch {branch}: {}",
                String::from_utf8_lossy(&deleted.stderr).trim()
            );
        }
    }

    Ok(())
}

fn is_shadow_branch(branch: &str) -> bool {
    if branch == "bitloops/checkpoints/v1" {
        return false;
    }
    if let Some(rest) = branch.strip_prefix("bitloops/") {
        return !rest.is_empty() && !rest.starts_with("checkpoints/");
    }
    false
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::{ResetConfig, run_reset_cmd};
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

    fn setup_reset_test_repo() -> (TempDir, String) {
        let dir = TempDir::new().expect("temp dir");
        let root = dir.path();

        let (ok, _, err) = run_git(root, &["init", "-b", "master"]);
        assert!(ok, "git init failed: {err}");
        let (ok, _, err) = run_git(
            root,
            &[
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-m",
                "initial commit",
            ],
        );
        assert!(ok, "initial commit failed: {err}");

        let (_, stdout, _) = run_git(root, &["rev-parse", "HEAD"]);
        let commit_hash = stdout.trim().to_string();
        (dir, commit_hash)
    }

    fn create_shadow_branch(repo_root: &Path, branch: &str) {
        let (ok, _, err) = run_git(repo_root, &["branch", branch]);
        assert!(ok, "failed to create branch {branch}: {err}");
    }

    fn create_session_state(
        repo_root: &Path,
        file_name: &str,
        session_id: &str,
        base_commit: &str,
        checkpoint_count: i32,
    ) {
        let session_dir = repo_root.join(".git").join("bitloops-sessions");
        fs::create_dir_all(&session_dir).expect("session dir");
        let content = format!(
            "{{\"session_id\":\"{session_id}\",\"base_commit\":\"{base_commit}\",\"checkpoint_count\":{checkpoint_count}}}"
        );
        fs::write(session_dir.join(file_name), content).expect("session file");
    }

    fn default_config(repo_root: &Path) -> ResetConfig {
        ResetConfig {
            repo_root: repo_root.to_path_buf(),
            force: false,
            session_id: None,
            strategy_name: "manual-commit".to_string(),
        }
    }

    // CLI-558
    #[test]
    fn TestResetCmd_NothingToReset() {
        let (repo, _) = setup_reset_test_repo();
        let cfg = default_config(repo.path());

        let result = run_reset_cmd(&cfg);
        assert!(
            result.is_ok(),
            "reset should succeed with nothing to reset: {result:?}"
        );
    }

    // CLI-559
    #[test]
    fn TestResetCmd_WithForce() {
        let (repo, commit_hash) = setup_reset_test_repo();
        let root = repo.path();
        create_shadow_branch(root, "bitloops/abc1234-deadbeef");
        create_session_state(
            root,
            "2026-02-02-test123.json",
            "2026-02-02-test123",
            &commit_hash,
            1,
        );

        let mut cfg = default_config(root);
        cfg.force = true;

        let result = run_reset_cmd(&cfg);
        assert!(result.is_ok(), "reset --force should succeed: {result:?}");

        assert!(
            !root
                .join(".git")
                .join("bitloops-sessions")
                .join("2026-02-02-test123.json")
                .exists(),
            "session state file should be deleted"
        );

        let (ok, _, _) = run_git(
            root,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/heads/bitloops/abc1234-deadbeef",
            ],
        );
        assert!(!ok, "shadow branch should be deleted");
    }

    // CLI-560
    #[test]
    fn TestResetCmd_SessionsWithoutShadowBranch() {
        let (repo, commit_hash) = setup_reset_test_repo();
        let root = repo.path();
        create_session_state(
            root,
            "2026-02-02-orphaned.json",
            "2026-02-02-orphaned",
            &commit_hash,
            1,
        );

        let mut cfg = default_config(root);
        cfg.force = true;

        let result = run_reset_cmd(&cfg);
        assert!(
            result.is_ok(),
            "reset should succeed without shadow branch: {result:?}"
        );
        assert!(
            !root
                .join(".git")
                .join("bitloops-sessions")
                .join("2026-02-02-orphaned.json")
                .exists(),
            "session state file should be deleted even without shadow branch"
        );
    }

    // CLI-561
    #[test]
    fn TestResetCmd_NotGitRepo() {
        let dir = TempDir::new().expect("temp dir");
        let cfg = default_config(dir.path());

        let result = run_reset_cmd(&cfg);
        assert!(result.is_err(), "reset should fail outside git repository");
        let msg = result.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            msg.contains("not a git repository"),
            "error should mention git repository, got: {msg}"
        );
    }

    // CLI-562
    #[test]
    fn TestResetCmd_AutoCommitStrategy() {
        let (repo, _) = setup_reset_test_repo();
        let mut cfg = default_config(repo.path());
        cfg.strategy_name = "auto-commit".to_string();

        let result = run_reset_cmd(&cfg);
        assert!(
            result.is_err(),
            "reset should fail for auto-commit strategy: {result:?}"
        );
        let msg = result.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            msg.contains("strategy auto-commit does not support reset"),
            "expected strategy-specific error, got: {msg}"
        );
    }

    // CLI-563
    #[test]
    fn TestResetCmd_MultipleSessions() {
        let (repo, commit_hash) = setup_reset_test_repo();
        let root = repo.path();
        create_shadow_branch(root, "bitloops/abc1234-deadbeef");
        create_session_state(
            root,
            "2026-02-02-session1.json",
            "2026-02-02-session1",
            &commit_hash,
            1,
        );
        create_session_state(
            root,
            "2026-02-02-session2.json",
            "2026-02-02-session2",
            &commit_hash,
            2,
        );

        let mut cfg = default_config(root);
        cfg.force = true;

        let result = run_reset_cmd(&cfg);
        assert!(
            result.is_ok(),
            "reset should succeed and delete all sessions: {result:?}"
        );

        assert!(
            !root
                .join(".git")
                .join("bitloops-sessions")
                .join("2026-02-02-session1.json")
                .exists(),
            "session1 should be deleted"
        );
        assert!(
            !root
                .join(".git")
                .join("bitloops-sessions")
                .join("2026-02-02-session2.json")
                .exists(),
            "session2 should be deleted"
        );
        let (ok, _, _) = run_git(
            root,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/heads/bitloops/abc1234-deadbeef",
            ],
        );
        assert!(!ok, "shadow branch should be deleted");
    }
}
