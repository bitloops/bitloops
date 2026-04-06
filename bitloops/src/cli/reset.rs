use anyhow::{Context, Result, bail};
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

    cleanup_session_states(&config.repo_root, config.session_id.as_deref())?;
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

fn cleanup_session_states(repo_root: &Path, target_session_id: Option<&str>) -> Result<()> {
    let backend = crate::host::checkpoints::session::create_session_backend_or_local(repo_root);
    if let Some(session_id) = target_session_id {
        backend
            .delete_session(session_id)
            .with_context(|| format!("failed to delete session state {session_id}"))?;
        return Ok(());
    }

    let sessions = backend
        .list_sessions()
        .context("failed to list session states for reset")?;
    for session in sessions {
        backend
            .delete_session(&session.session_id)
            .with_context(|| format!("failed to delete session state {}", session.session_id))?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::{ResetConfig, run_reset_cmd};
    use crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE;
    use crate::config::{resolve_sqlite_db_path_for_repo, resolve_store_backend_config_for_repo};
    use crate::host::checkpoints::session::state::SessionState;
    use crate::storage::SqliteConnectionPool;
    use crate::test_support::git_fixtures::write_test_daemon_config;
    use crate::test_support::process_state::{git_command, with_process_state};
    use std::path::Path;
    use tempfile::TempDir;

    fn run_git(dir: &Path, args: &[&str]) -> (bool, String, String) {
        let out = git_command()
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

    fn with_legacy_local_backend<T>(f: impl FnOnce() -> T) -> T {
        let config_dir = TempDir::new().expect("temp daemon config");
        let config_path = write_test_daemon_config(config_dir.path());
        let config_path_string = config_path.to_string_lossy().to_string();
        with_process_state(
            None,
            &[(
                ENV_DAEMON_CONFIG_PATH_OVERRIDE,
                Some(config_path_string.as_str()),
            )],
            f,
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
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "initial commit",
            ],
        );
        assert!(ok, "initial commit failed: {err}");
        ensure_relational_store_file(root);
        write_test_daemon_config(root);

        let (_, stdout, _) = run_git(root, &["rev-parse", "HEAD"]);
        let commit_hash = stdout.trim().to_string();
        (dir, commit_hash)
    }

    fn create_session_state(
        repo_root: &Path,
        file_name: &str,
        session_id: &str,
        base_commit: &str,
        checkpoint_count: i32,
    ) {
        let _ = file_name;
        let _ = checkpoint_count;
        let backend = crate::host::checkpoints::session::create_session_backend_or_local(repo_root);
        backend
            .save_session(&SessionState {
                session_id: session_id.to_string(),
                base_commit: base_commit.to_string(),
                ..Default::default()
            })
            .expect("save session state");
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

        let result = with_legacy_local_backend(|| run_reset_cmd(&cfg));
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
        create_session_state(
            root,
            "2026-02-02-test123.json",
            "2026-02-02-test123",
            &commit_hash,
            1,
        );

        let mut cfg = default_config(root);
        cfg.force = true;

        let result = with_legacy_local_backend(|| run_reset_cmd(&cfg));
        assert!(result.is_ok(), "reset --force should succeed: {result:?}");

        assert!(
            crate::host::checkpoints::session::create_session_backend_or_local(root)
                .load_session("2026-02-02-test123")
                .expect("load session")
                .is_none(),
            "session state file should be deleted"
        );
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

        let result = with_legacy_local_backend(|| run_reset_cmd(&cfg));
        assert!(
            result.is_ok(),
            "reset should succeed without shadow branch: {result:?}"
        );
        assert!(
            crate::host::checkpoints::session::create_session_backend_or_local(root)
                .load_session("2026-02-02-orphaned")
                .expect("load session")
                .is_none(),
            "session state file should be deleted even without shadow branch"
        );
    }

    // CLI-561
    #[test]
    fn TestResetCmd_NotGitRepo() {
        let dir = TempDir::new().expect("temp dir");
        let cfg = default_config(dir.path());

        let result = with_legacy_local_backend(|| run_reset_cmd(&cfg));
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

        let result = with_legacy_local_backend(|| run_reset_cmd(&cfg));
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

        let result = with_legacy_local_backend(|| run_reset_cmd(&cfg));
        assert!(
            result.is_ok(),
            "reset should succeed and delete all sessions: {result:?}"
        );

        assert!(
            crate::host::checkpoints::session::create_session_backend_or_local(root)
                .load_session("2026-02-02-session1")
                .expect("load session1")
                .is_none(),
            "session1 should be deleted"
        );
        assert!(
            crate::host::checkpoints::session::create_session_backend_or_local(root)
                .load_session("2026-02-02-session2")
                .expect("load session2")
                .is_none(),
            "session2 should be deleted"
        );
    }
}
