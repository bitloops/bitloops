mod test_command_support;

use std::ffi::OsStr;
use std::process::Command;

#[test]
fn isolated_bitloops_command_routes_test_state_under_repo() {
    let repo = tempfile::tempdir().expect("temp dir");
    let bin = repo.path().join("bitloops-bin");

    let cmd = test_command_support::new_isolated_bitloops_command(&bin, repo.path(), &["status"]);

    let expected_state_root = repo.path().join(".bitloops-test-state");
    assert_eq!(
        cmd.get_envs()
            .find(|(key, _)| *key == OsStr::new("BITLOOPS_TEST_STATE_DIR_OVERRIDE"))
            .and_then(|(_, value)| value)
            .expect("test state override env should be present"),
        expected_state_root.as_os_str()
    );
}

#[test]
fn enter_repo_app_env_routes_test_state_under_repo() {
    let repo = tempfile::tempdir().expect("temp dir");
    let previous = std::env::var_os("BITLOOPS_TEST_STATE_DIR_OVERRIDE");

    {
        let _guard = test_command_support::enter_repo_app_env(repo.path());
        assert_eq!(
            std::env::var_os("BITLOOPS_TEST_STATE_DIR_OVERRIDE")
                .expect("test state override env should be present"),
            repo.path().join(".bitloops-test-state").into_os_string()
        );
    }

    assert_eq!(
        std::env::var_os("BITLOOPS_TEST_STATE_DIR_OVERRIDE"),
        previous
    );
}

#[test]
fn apply_repo_app_paths_routes_test_state_under_repo() {
    let repo = tempfile::tempdir().expect("temp dir");
    let paths = test_command_support::repo_app_paths(repo.path());
    let mut cmd = Command::new("bitloops");

    test_command_support::apply_repo_app_paths(&mut cmd, &paths);

    assert_eq!(
        cmd.get_envs()
            .find(|(key, _)| *key == OsStr::new("BITLOOPS_TEST_STATE_DIR_OVERRIDE"))
            .and_then(|(_, value)| value)
            .expect("test state override env should be present"),
        repo.path().join(".bitloops-test-state").as_os_str()
    );
}
