mod test_command_support;

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn isolated_bitloops_command_routes_test_state_outside_repo() {
    let repo = tempfile::tempdir().expect("temp dir");
    let bin = repo.path().join("bitloops-bin");

    let cmd = test_command_support::new_isolated_bitloops_command(&bin, repo.path(), &["status"]);

    let state_root = PathBuf::from(
        cmd.get_envs()
            .find(|(key, _)| *key == OsStr::new("BITLOOPS_TEST_STATE_DIR_OVERRIDE"))
            .and_then(|(_, value)| value)
            .expect("test state override env should be present"),
    );
    assert_isolated_test_state_path(&state_root, repo.path());
}

#[test]
fn enter_repo_app_env_routes_test_state_outside_repo() {
    let repo = tempfile::tempdir().expect("temp dir");
    let previous = std::env::var_os("BITLOOPS_TEST_STATE_DIR_OVERRIDE");

    {
        let _guard = test_command_support::enter_repo_app_env(repo.path());
        let state_root = PathBuf::from(
            std::env::var_os("BITLOOPS_TEST_STATE_DIR_OVERRIDE")
                .expect("test state override env should be present"),
        );
        assert_isolated_test_state_path(&state_root, repo.path());
    }

    assert_eq!(
        std::env::var_os("BITLOOPS_TEST_STATE_DIR_OVERRIDE"),
        previous
    );
}

#[test]
fn apply_repo_app_paths_routes_test_state_outside_repo() {
    let repo = tempfile::tempdir().expect("temp dir");
    let paths = test_command_support::repo_app_paths(repo.path());
    let mut cmd = Command::new("bitloops");

    test_command_support::apply_repo_app_paths(&mut cmd, &paths);

    let state_root = PathBuf::from(
        cmd.get_envs()
            .find(|(key, _)| *key == OsStr::new("BITLOOPS_TEST_STATE_DIR_OVERRIDE"))
            .and_then(|(_, value)| value)
            .expect("test state override env should be present"),
    );
    assert_isolated_test_state_path(&state_root, repo.path());
}

#[test]
fn repo_app_paths_does_not_mutate_git_exclude() {
    let repo = tempfile::tempdir().expect("temp dir");
    let exclude_path = repo.path().join(".git").join("info").join("exclude");
    fs::create_dir_all(exclude_path.parent().expect("exclude parent")).expect("create .git/info");
    fs::write(&exclude_path, "existing-rule\n").expect("write exclude");

    let _paths = test_command_support::repo_app_paths(repo.path());

    assert_eq!(
        fs::read_to_string(&exclude_path).expect("read exclude"),
        "existing-rule\n"
    );
}

fn assert_isolated_test_state_path(state_root: &Path, repo: &Path) {
    let expected_process_root = std::env::temp_dir()
        .join("bitloops-test-state")
        .join(format!("process-{}", std::process::id()));

    assert!(
        state_root.starts_with(&expected_process_root),
        "test state root {} should be under {}",
        state_root.display(),
        expected_process_root.display()
    );
    assert!(
        !state_root.starts_with(repo),
        "test state root {} should not be under repo {}",
        state_root.display(),
        repo.display()
    );
}
