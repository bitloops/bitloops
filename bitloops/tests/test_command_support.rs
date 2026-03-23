use std::fs;
use std::path::Path;
use std::process::Command;

use bitloops::cli::versioncheck::DISABLE_VERSION_CHECK_ENV;
use bitloops::host::devql::watch::DISABLE_WATCHER_AUTOSTART_ENV;
use tempfile::TempDir;

pub fn new_isolated_bitloops_command(
    bin_path: &Path,
    repo: &Path,
    args: &[&str],
) -> (Command, TempDir) {
    let isolated_home = tempfile::tempdir().expect("create isolated home for test command");
    let xdg_config_home = isolated_home.path().join("xdg");
    fs::create_dir_all(&xdg_config_home).expect("create isolated xdg config home");

    let mut cmd = Command::new(bin_path);
    cmd.args(args)
        .current_dir(repo)
        .env("HOME", isolated_home.path())
        .env("USERPROFILE", isolated_home.path())
        .env("XDG_CONFIG_HOME", &xdg_config_home)
        .env(DISABLE_WATCHER_AUTOSTART_ENV, "1")
        .env(DISABLE_VERSION_CHECK_ENV, "1")
        .env_remove("BITLOOPS_DEVQL_PG_DSN")
        .env_remove("BITLOOPS_DEVQL_CH_URL")
        .env_remove("BITLOOPS_DEVQL_CH_DATABASE")
        .env_remove("BITLOOPS_DEVQL_CH_USER")
        .env_remove("BITLOOPS_DEVQL_CH_PASSWORD");

    (cmd, isolated_home)
}
