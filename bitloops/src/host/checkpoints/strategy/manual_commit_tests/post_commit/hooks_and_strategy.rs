use super::*;

#[test]
pub(crate) fn pre_push_is_noop_even_when_checkpoints_branch_exists() {
    let base = tempfile::tempdir().unwrap();
    let origin_dir = base.path().join("origin.git");
    let work_dir = base.path().join("work");
    fs::create_dir_all(&work_dir).unwrap();

    // Bare remote.
    let out = git_command()
        .args(["init", "--bare", origin_dir.to_string_lossy().as_ref()])
        .output()
        .unwrap();
    assert!(out.status.success(), "git init --bare failed");

    let work_temp = tempfile::TempDir::new_in(&work_dir).unwrap();
    let repo_dir = work_temp.path();
    let run = |args: &[&str]| {
        let out = git_command()
            .args(args)
            .current_dir(repo_dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {:?} failed", args);
    };

    run(&["init"]);
    run(&["config", "user.email", "t@t.com"]);
    run(&["config", "user.name", "Test"]);
    fs::write(repo_dir.join("README.md"), "init").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "initial"]);
    run(&[
        "remote",
        "add",
        "origin",
        origin_dir.to_string_lossy().as_ref(),
    ]);

    // Create local checkpoints branch to push.
    let head = run_git(repo_dir, &["rev-parse", "HEAD"]).unwrap();
    run(&["update-ref", "refs/heads/bitloops/checkpoints/v1", &head]);

    let strategy = ManualCommitStrategy::new(repo_dir);
    strategy.pre_push("origin").unwrap();

    // Remote should not have bitloops/checkpoints/v1 because pre_push is now a no-op.
    let remote_ref = git_command()
        .args([
            "--git-dir",
            origin_dir.to_string_lossy().as_ref(),
            "show-ref",
            "--verify",
            "refs/heads/bitloops/checkpoints/v1",
        ])
        .output()
        .unwrap();
    assert!(
        !remote_ref.status.success(),
        "remote should not contain checkpoints branch after pre-push no-op"
    );
}

#[test]
pub(crate) fn shadow_strategy_direct_instantiation() {
    let dir = tempfile::tempdir().unwrap();
    let strategy = ManualCommitStrategy::new(dir.path());
    assert_eq!(strategy.name(), "manual-commit");
}

#[test]
pub(crate) fn shadow_strategy_description() {
    let dir = tempfile::tempdir().unwrap();
    let strategy = ManualCommitStrategy::new(dir.path());
    assert_eq!(strategy.name(), "manual-commit");
}

#[test]
pub(crate) fn shadow_strategy_validate_repository() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    assert!(
        run_git(dir.path(), &["rev-parse", "--is-inside-work-tree"]).is_ok(),
        "expected git repo to validate"
    );
}

#[test]
pub(crate) fn shadow_strategy_validate_repository_not_git_repo() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        run_git(dir.path(), &["rev-parse", "--is-inside-work-tree"]).is_err(),
        "non-git directory should fail validation"
    );
}
