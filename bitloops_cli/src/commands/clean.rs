use anyhow::{Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::engine::paths;
use crate::engine::session::{
    create_session_backend_or_local, delete_legacy_local_session_state,
    list_legacy_local_session_ids,
};
use crate::engine::strategy::manual_commit::{
    delete_shadow_branches_for_cleanup, list_orphaned_session_states_for_cleanup,
    list_shadow_branches_for_cleanup,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupType {
    ShadowBranch,
    SessionState,
    Checkpoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupItem {
    pub item_type: CleanupType,
    pub id: String,
    pub reason: String,
}

pub fn run_clean(w: &mut dyn Write, force: bool) -> Result<()> {
    let repo_root = resolve_repo_root()?;

    let mut items: Vec<CleanupItem> = list_shadow_branches_for_cleanup(&repo_root)?
        .into_iter()
        .map(|branch| CleanupItem {
            item_type: CleanupType::ShadowBranch,
            id: branch,
            reason: "orphaned shadow branch".to_string(),
        })
        .collect();

    items.extend(
        list_orphaned_session_states_for_cleanup(&repo_root)?
            .into_iter()
            .map(|state| CleanupItem {
                item_type: CleanupType::SessionState,
                id: state.id,
                reason: state.reason,
            }),
    );

    items.extend(
        list_orphaned_checkpoint_metadata(&repo_root)
            .into_iter()
            .map(|checkpoint| CleanupItem {
                item_type: CleanupType::Checkpoint,
                id: checkpoint,
                reason: "orphaned checkpoint metadata".to_string(),
            }),
    );

    run_clean_with_items(w, force, &items)
}

pub fn run_clean_with_items(w: &mut dyn Write, force: bool, items: &[CleanupItem]) -> Result<()> {
    run_clean_with_items_at_root(w, force, items, None)
}

fn run_clean_with_items_at_root(
    w: &mut dyn Write,
    force: bool,
    items: &[CleanupItem],
    repo_root_override: Option<&Path>,
) -> Result<()> {
    if items.is_empty() {
        writeln!(w, "No orphaned items to clean up.")?;
        return Ok(());
    }

    let mut branches: Vec<&CleanupItem> = Vec::new();
    let mut states: Vec<&CleanupItem> = Vec::new();
    let mut checkpoints: Vec<&CleanupItem> = Vec::new();
    for item in items {
        match item.item_type {
            CleanupType::ShadowBranch => branches.push(item),
            CleanupType::SessionState => states.push(item),
            CleanupType::Checkpoint => checkpoints.push(item),
        }
    }

    if !force {
        writeln!(w, "Found {} orphaned items:", items.len())?;
        writeln!(w)?;

        if !branches.is_empty() {
            writeln!(w, "Shadow branches ({}):", branches.len())?;
            for item in &branches {
                writeln!(w, "  {}", item.id)?;
            }
            writeln!(w)?;
        }

        if !states.is_empty() {
            writeln!(w, "Session states ({}):", states.len())?;
            for item in &states {
                writeln!(w, "  {}", item.id)?;
            }
            writeln!(w)?;
        }

        if !checkpoints.is_empty() {
            writeln!(w, "Checkpoint metadata ({}):", checkpoints.len())?;
            for item in &checkpoints {
                writeln!(w, "  {}", item.id)?;
            }
            writeln!(w)?;
        }

        writeln!(w, "Run with --force to delete these items.")?;
        return Ok(());
    }

    let mut deleted_branches: Vec<String> = Vec::new();
    let mut deleted_states: Vec<String> = Vec::new();
    let mut deleted_checkpoints: Vec<String> = Vec::new();
    let mut failed_branches: Vec<String> = Vec::new();
    let mut failed_states: Vec<String> = Vec::new();
    let mut failed_checkpoints: Vec<String> = Vec::new();

    let repo_root = repo_root_override
        .map(Path::to_path_buf)
        .or_else(|| resolve_repo_root().ok());

    let shadow_branches: Vec<String> = branches.iter().map(|item| item.id.clone()).collect();
    if !shadow_branches.is_empty() {
        if let Some(root) = repo_root.as_deref() {
            let (deleted, failed) = delete_shadow_branches_for_cleanup(root, &shadow_branches);
            deleted_branches.extend(deleted);
            failed_branches.extend(failed);
        } else {
            failed_branches.extend(shadow_branches);
        }
    }

    for item in states {
        if delete_session_state(repo_root.as_deref(), &item.id) {
            deleted_states.push(item.id.clone());
        } else {
            failed_states.push(item.id.clone());
        }
    }

    for item in checkpoints {
        if delete_checkpoint_metadata(repo_root.as_deref(), &item.id) {
            deleted_checkpoints.push(item.id.clone());
        } else {
            failed_checkpoints.push(item.id.clone());
        }
    }

    let total_deleted = deleted_branches.len() + deleted_states.len() + deleted_checkpoints.len();
    let total_failed = failed_branches.len() + failed_states.len() + failed_checkpoints.len();

    if total_deleted > 0 {
        writeln!(w, "Deleted {} items:", total_deleted)?;

        if !deleted_branches.is_empty() {
            writeln!(w, "\n  Shadow branches ({}):", deleted_branches.len())?;
            for branch in &deleted_branches {
                writeln!(w, "    {}", branch)?;
            }
        }

        if !deleted_states.is_empty() {
            writeln!(w, "\n  Session states ({}):", deleted_states.len())?;
            for state in &deleted_states {
                writeln!(w, "    {}", state)?;
            }
        }

        if !deleted_checkpoints.is_empty() {
            writeln!(w, "\n  Checkpoints ({}):", deleted_checkpoints.len())?;
            for checkpoint in &deleted_checkpoints {
                writeln!(w, "    {}", checkpoint)?;
            }
        }
    }

    if total_failed > 0 {
        writeln!(w, "\nFailed to delete {} items:", total_failed)?;

        if !failed_branches.is_empty() {
            writeln!(w, "\n  Shadow branches:")?;
            for branch in &failed_branches {
                writeln!(w, "    {}", branch)?;
            }
        }

        if !failed_states.is_empty() {
            writeln!(w, "\n  Session states:")?;
            for state in &failed_states {
                writeln!(w, "    {}", state)?;
            }
        }

        if !failed_checkpoints.is_empty() {
            writeln!(w, "\n  Checkpoints:")?;
            for checkpoint in &failed_checkpoints {
                writeln!(w, "    {}", checkpoint)?;
            }
        }

        bail!("failed to delete {} items", total_failed);
    }

    Ok(())
}

fn resolve_repo_root() -> Result<PathBuf> {
    match paths::repo_root() {
        Ok(root) => Ok(root),
        Err(_) => bail!("not a git repository"),
    }
}

fn list_orphaned_checkpoint_metadata(_repo_root: &Path) -> Vec<String> {
    // Auto-commit checkpoint metadata cleanup is not yet wired in Rust runtime.
    // Keep this hook point so `clean` can include checkpoint items when that
    // path is connected.
    vec![]
}

fn delete_session_state(repo_root: Option<&Path>, session_or_path: &str) -> bool {
    let Some(root) = repo_root else {
        return false;
    };
    if !is_valid_cleanup_id(session_or_path) {
        return false;
    }

    let backend = create_session_backend_or_local(root);
    let exists_in_backend = backend
        .load_session(session_or_path)
        .map(|state| state.is_some())
        .unwrap_or(false);
    let exists_in_legacy = list_legacy_local_session_ids(root.to_path_buf())
        .map(|ids| ids.iter().any(|id| id == session_or_path))
        .unwrap_or(false);
    if !exists_in_backend && !exists_in_legacy {
        return false;
    }

    if backend.delete_session(session_or_path).is_err() {
        return false;
    }
    delete_legacy_local_session_state(root.to_path_buf(), session_or_path).is_ok()
}

fn delete_checkpoint_metadata(_repo_root: Option<&Path>, checkpoint_id: &str) -> bool {
    if !is_valid_cleanup_id(checkpoint_id) {
        return false;
    }

    // Auto-commit checkpoint metadata cleanup is not yet wired in Rust runtime.
    false
}

fn is_valid_cleanup_id(id: &str) -> bool {
    if id.contains('\\') {
        return false;
    }
    let mut components = Path::new(id).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::{CleanupItem, CleanupType, run_clean, run_clean_with_items};
    use crate::engine::session::backend::SessionBackend;
    use crate::engine::session::local_backend::LocalFileBackend;
    use crate::engine::session::state::SessionState;
    use crate::test_support::process_state::with_cwd;
    use std::io::Cursor;
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

    fn setup_clean_test_repo() -> TempDir {
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
        dir
    }

    fn branch_exists(repo: &Path, name: &str) -> bool {
        let (ok, _, _) = run_git(
            repo,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{name}"),
            ],
        );
        ok
    }

    fn create_orphan_session_state(repo: &Path, session_id: &str) {
        let backend = LocalFileBackend::new(repo);
        let state = SessionState {
            session_id: session_id.to_string(),
            base_commit: "1234567abcdef".to_string(),
            started_at: "2025-01-01T00:00:00Z".to_string(),
            ..Default::default()
        };
        backend
            .save_session(&state)
            .expect("save orphan session state");
    }

    // CLI-537
    #[test]
    fn TestRunClean_NoOrphanedItems() {
        let repo = setup_clean_test_repo();
        let mut stdout = Cursor::new(Vec::new());

        let err = with_cwd(repo.path(), || run_clean(&mut stdout, false));
        assert!(err.is_ok(), "run_clean returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("No orphaned items"),
            "expected 'No orphaned items' message, got: {output}"
        );
    }

    // CLI-538
    #[test]
    fn TestRunClean_PreviewMode() {
        let repo = setup_clean_test_repo();
        let root = repo.path().to_path_buf();

        let _ = run_git(&root, &["branch", "bitloops/abc1234"]);
        let _ = run_git(&root, &["branch", "bitloops/def5678"]);
        let _ = run_git(&root, &["branch", "bitloops/checkpoints/v1"]);

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(&root, || run_clean(&mut stdout, false));
        assert!(err.is_ok(), "run_clean returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("orphaned items"),
            "missing orphaned header: {output}"
        );
        assert!(
            output.contains("bitloops/abc1234"),
            "missing branch: {output}"
        );
        assert!(
            output.contains("bitloops/def5678"),
            "missing branch: {output}"
        );
        assert!(
            !output.contains("bitloops/checkpoints/v1"),
            "metadata branch must be hidden: {output}"
        );
        assert!(
            output.contains("--force"),
            "missing --force prompt: {output}"
        );
        assert!(
            branch_exists(&root, "bitloops/abc1234"),
            "branch should still exist"
        );
        assert!(
            branch_exists(&root, "bitloops/def5678"),
            "branch should still exist"
        );
    }

    // CLI-539
    #[test]
    fn TestRunClean_ForceMode() {
        let repo = setup_clean_test_repo();
        let root = repo.path().to_path_buf();
        let _ = run_git(&root, &["branch", "bitloops/abc1234"]);
        let _ = run_git(&root, &["branch", "bitloops/def5678"]);

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(&root, || run_clean(&mut stdout, true));
        assert!(err.is_ok(), "run_clean returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("Deleted"),
            "expected delete summary, got: {output}"
        );
        assert!(
            !branch_exists(&root, "bitloops/abc1234"),
            "branch should be deleted"
        );
        assert!(
            !branch_exists(&root, "bitloops/def5678"),
            "branch should be deleted"
        );
    }

    // CLI-540
    #[test]
    fn TestRunClean_SessionsBranchPreserved() {
        let repo = setup_clean_test_repo();
        let root = repo.path().to_path_buf();
        let _ = run_git(&root, &["branch", "bitloops/abc1234"]);
        let _ = run_git(&root, &["branch", "bitloops/checkpoints/v1"]);

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(&root, || run_clean(&mut stdout, true));
        assert!(err.is_ok(), "run_clean returned error: {err:?}");

        assert!(
            !branch_exists(&root, "bitloops/abc1234"),
            "shadow branch should be deleted"
        );
        assert!(
            branch_exists(&root, "bitloops/checkpoints/v1"),
            "metadata branch should be preserved"
        );
    }

    // CLI-541
    #[test]
    fn TestRunClean_NotGitRepository() {
        let dir = TempDir::new().expect("temp dir");
        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(dir.path(), || run_clean(&mut stdout, false));
        assert!(err.is_err(), "run_clean should fail outside git repository");
    }

    // CLI-542
    #[test]
    fn TestRunClean_Subdirectory() {
        let repo = setup_clean_test_repo();
        let root = repo.path().to_path_buf();
        let _ = run_git(&root, &["branch", "bitloops/abc1234"]);

        let subdir = root.join("subdir");
        std::fs::create_dir_all(&subdir).expect("subdir");

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(&subdir, || run_clean(&mut stdout, false));
        assert!(
            err.is_ok(),
            "run_clean returned error from subdirectory: {err:?}"
        );

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("bitloops/abc1234"),
            "should list shadow branch from subdirectory call, got: {output}"
        );
    }

    #[test]
    fn TestRunClean_PreviewIncludesOrphanSessionState() {
        let repo = setup_clean_test_repo();
        let root = repo.path().to_path_buf();
        create_orphan_session_state(&root, "orphan-session-123");

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(&root, || run_clean(&mut stdout, false));
        assert!(err.is_ok(), "run_clean returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("Session states (1)"),
            "expected session-state section, got: {output}"
        );
        assert!(
            output.contains("orphan-session-123"),
            "expected orphan session id in output, got: {output}"
        );
    }

    #[test]
    fn TestRunClean_ForceDeletesOrphanSessionState() {
        let repo = setup_clean_test_repo();
        let root = repo.path().to_path_buf();
        create_orphan_session_state(&root, "orphan-session-456");

        let backend = LocalFileBackend::new(&root);
        let session_path = backend.sessions_dir().join("orphan-session-456.json");
        assert!(
            session_path.exists(),
            "expected session file to exist before cleanup"
        );

        let mut stdout = Cursor::new(Vec::new());
        let err = with_cwd(&root, || run_clean(&mut stdout, true));
        assert!(err.is_ok(), "run_clean returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("Session states (1)"),
            "expected deleted session-state section, got: {output}"
        );
        assert!(
            !session_path.exists(),
            "expected orphan session file to be deleted"
        );
    }

    // CLI-543
    #[test]
    fn TestRunCleanWithItems_PartialFailure() {
        let repo = setup_clean_test_repo();
        let root = repo.path().to_path_buf();
        let _ = run_git(&root, &["branch", "bitloops/abc1234"]);

        let mut stdout = Cursor::new(Vec::new());
        let items = vec![
            CleanupItem {
                item_type: CleanupType::ShadowBranch,
                id: "bitloops/abc1234".to_string(),
                reason: "test".to_string(),
            },
            CleanupItem {
                item_type: CleanupType::ShadowBranch,
                id: "bitloops/nonexistent1234567".to_string(),
                reason: "test".to_string(),
            },
        ];

        let err = with_cwd(&root, || run_clean_with_items(&mut stdout, true, &items));
        assert!(
            err.is_err(),
            "run_clean_with_items should return error when some deletions fail"
        );

        let err_str = err.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            err_str.contains("failed to delete"),
            "expected failure message, got: {err_str}"
        );

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("Deleted 1 items"),
            "output should include successful deletions, got: {output}"
        );
        assert!(
            output.contains("Failed to delete 1 items"),
            "output should include failures, got: {output}"
        );
    }

    // CLI-544
    #[test]
    fn TestRunCleanWithItems_AllFailures() {
        let repo = setup_clean_test_repo();
        let root = repo.path().to_path_buf();

        let mut stdout = Cursor::new(Vec::new());
        let items = vec![
            CleanupItem {
                item_type: CleanupType::ShadowBranch,
                id: "bitloops/nonexistent1234567".to_string(),
                reason: "test".to_string(),
            },
            CleanupItem {
                item_type: CleanupType::ShadowBranch,
                id: "bitloops/alsononexistent".to_string(),
                reason: "test".to_string(),
            },
        ];

        let err = with_cwd(&root, || run_clean_with_items(&mut stdout, true, &items));
        assert!(err.is_err(), "should return error when all deletions fail");

        let err_str = err.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            err_str.contains("failed to delete 2 items"),
            "expected failure count in error, got: {err_str}"
        );

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            !output.contains("Deleted"),
            "should not show success section when all deletions fail, got: {output}"
        );
        assert!(
            output.contains("Failed to delete 2 items"),
            "should show failure section, got: {output}"
        );
    }

    // CLI-545
    #[test]
    fn TestRunCleanWithItems_NoItems() {
        let mut stdout = Cursor::new(Vec::new());
        let err = run_clean_with_items(&mut stdout, false, &[]);
        assert!(err.is_ok(), "run_clean_with_items returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("No orphaned items"),
            "expected no orphaned message, got: {output}"
        );
    }

    // CLI-546
    #[test]
    fn TestRunCleanWithItems_MixedTypes_Preview() {
        let mut stdout = Cursor::new(Vec::new());
        let items = vec![
            CleanupItem {
                item_type: CleanupType::ShadowBranch,
                id: "bitloops/abc1234".to_string(),
                reason: "test".to_string(),
            },
            CleanupItem {
                item_type: CleanupType::SessionState,
                id: "session-123".to_string(),
                reason: "no checkpoints".to_string(),
            },
            CleanupItem {
                item_type: CleanupType::Checkpoint,
                id: "checkpoint-abc".to_string(),
                reason: "orphaned".to_string(),
            },
        ];

        let err = run_clean_with_items(&mut stdout, false, &items);
        assert!(err.is_ok(), "run_clean_with_items returned error: {err:?}");

        let output = String::from_utf8(stdout.into_inner()).expect("utf8");
        assert!(
            output.contains("Shadow branches"),
            "missing shadow section: {output}"
        );
        assert!(
            output.contains("Session states"),
            "missing session section: {output}"
        );
        assert!(
            output.contains("Checkpoint metadata"),
            "missing checkpoint section: {output}"
        );
        assert!(
            output.contains("Found 3 orphaned items"),
            "missing total count section: {output}"
        );
    }
}
