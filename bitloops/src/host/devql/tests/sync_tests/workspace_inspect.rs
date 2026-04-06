use std::fs;
use tempfile::tempdir;

use super::fixtures::{seed_full_sync_repo, seed_workspace_repo};

#[test]
fn workspace_state_inspect_workspace_reads_head_tree() {
    let repo = seed_workspace_repo();

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect clean workspace");

    let head_sha = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", "HEAD"],
    )
    .expect("resolve HEAD");
    let head_blob = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", "HEAD:src/lib.rs"],
    )
    .expect("resolve HEAD blob");
    let head_tree_sha = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", "HEAD^{tree}"],
    )
    .expect("resolve HEAD tree");
    let active_branch = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["branch", "--show-current"],
    )
    .expect("resolve active branch");

    assert_eq!(state.head_commit_sha.as_deref(), Some(head_sha.as_str()));
    assert_eq!(state.head_tree_sha.as_deref(), Some(head_tree_sha.as_str()));
    assert_eq!(state.active_branch.as_deref(), Some(active_branch.as_str()));
    assert_eq!(state.head_tree.len(), 2);
    assert_eq!(state.head_tree.get("src/lib.rs"), Some(&head_blob));
    assert!(state.head_tree.contains_key("README.md"));
    assert!(state.staged_changes.is_empty());
    assert!(state.dirty_files.is_empty());
    assert!(state.untracked_files.is_empty());
}

#[test]
fn workspace_state_reports_dirty_files() {
    let repo = seed_workspace_repo();
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n",
    )
    .expect("rewrite rust source");

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect dirty workspace");

    assert!(state.staged_changes.is_empty());
    assert_eq!(state.dirty_files, vec!["src/lib.rs".to_string()]);
    assert!(state.untracked_files.is_empty());
    assert!(state.head_tree.contains_key("src/lib.rs"));
}

#[test]
fn workspace_state_staged_changes_report_index_diffs() {
    let repo = seed_workspace_repo();
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hey {name}\")\n}\n",
    )
    .expect("rewrite rust source");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "src/lib.rs"]);

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect staged workspace");

    let index_blob = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["rev-parse", ":src/lib.rs"],
    )
    .expect("resolve index blob");
    let staged = state
        .staged_changes
        .get("src/lib.rs")
        .expect("expected staged rust file");
    assert_eq!(
        staged,
        &crate::host::devql::sync::workspace_state::StagedChange::Modified(index_blob)
    );
    assert_eq!(state.staged_changes.len(), 1);
    assert!(state.dirty_files.is_empty());
    assert!(state.untracked_files.is_empty());
}

#[test]
fn workspace_state_reports_staged_deletes() {
    let repo = seed_workspace_repo();
    crate::test_support::git_fixtures::git_ok(repo.path(), &["rm", "src/lib.rs"]);

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect staged delete workspace");

    let staged = state
        .staged_changes
        .get("src/lib.rs")
        .expect("expected staged delete");
    assert_eq!(
        staged,
        &crate::host::devql::sync::workspace_state::StagedChange::Deleted
    );
    assert_eq!(state.staged_changes.len(), 1);
    assert!(state.dirty_files.is_empty());
    assert!(state.untracked_files.is_empty());
}

#[test]
fn workspace_state_reports_untracked_files() {
    let repo = seed_workspace_repo();
    fs::write(
        repo.path().join("src/new_file.rs"),
        "pub fn created() -> i32 {\n    7\n}\n",
    )
    .expect("write untracked rust source");

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect workspace with untracked file");

    assert!(state.staged_changes.is_empty());
    assert!(state.dirty_files.is_empty());
    assert_eq!(state.untracked_files, vec!["src/new_file.rs".to_string()]);
    assert!(!state.head_tree.contains_key("src/new_file.rs"));
}

#[test]
fn workspace_state_filter_limits_results_to_requested_paths() {
    let repo = seed_full_sync_repo();
    let requested_paths = std::collections::HashSet::from(["src/lib.rs".to_string()]);

    let state = crate::host::devql::sync::workspace_state::inspect_workspace_for_paths(
        repo.path(),
        Some(&requested_paths),
    )
    .expect("inspect filtered workspace");

    assert_eq!(state.head_tree.len(), 1);
    assert!(state.head_tree.contains_key("src/lib.rs"));
    assert!(
        state.staged_changes.keys().all(|path| path == "src/lib.rs"),
        "filtered staged changes should only include requested paths"
    );
    assert!(
        state.dirty_files.iter().all(|path| path == "src/lib.rs"),
        "filtered dirty files should only include requested paths"
    );
    assert!(
        state
            .untracked_files
            .iter()
            .all(|path| path == "src/lib.rs"),
        "filtered untracked files should only include requested paths"
    );
}

#[test]
fn workspace_state_unborn_head_reports_raw_workspace_state() {
    let repo = tempdir().expect("temp dir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    fs::create_dir_all(repo.path().join("src")).expect("create src dir");
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn draft() -> bool {\n    true\n}\n",
    )
    .expect("write rust source");
    crate::test_support::git_fixtures::git_ok(repo.path(), &["add", "src/lib.rs"]);

    let state = crate::host::devql::sync::workspace_state::inspect_workspace(repo.path())
        .expect("inspect unborn HEAD");

    let active_branch = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo.path(),
        &["branch", "--show-current"],
    )
    .expect("resolve active branch");

    assert_eq!(state.head_commit_sha, None);
    assert_eq!(state.head_tree_sha, None);
    assert_eq!(state.active_branch.as_deref(), Some(active_branch.as_str()));
    assert!(state.head_tree.is_empty());
    assert_eq!(state.staged_changes.len(), 1);
    assert_eq!(
        state
            .staged_changes
            .get("src/lib.rs")
            .expect("expected staged rust file"),
        &crate::host::devql::sync::workspace_state::StagedChange::Added(
            crate::host::checkpoints::strategy::manual_commit::run_git(
                repo.path(),
                &["rev-parse", ":src/lib.rs"],
            )
            .expect("resolve staged blob"),
        )
    );
    assert!(state.dirty_files.is_empty());
    assert!(state.untracked_files.is_empty());
}
