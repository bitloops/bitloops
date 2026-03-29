use crate::test_harness_support::{
    Workspace, prepare_graphql_workspace, seed_production_artefacts, write_rust_static_link_fixture,
};
use bitloops::cli::versioncheck::DISABLE_VERSION_CHECK_ENV;
use bitloops::host::devql::watch::DISABLE_WATCHER_AUTOSTART_ENV;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub struct SeededGraphqlWorkspace {
    pub workspace: Workspace,
    _daemon: DaemonGuard,
}

struct DaemonGuard {
    workdir: PathBuf,
    _home: TempDir,
    xdg_config_home: PathBuf,
}

impl DaemonGuard {
    fn start(workdir: &Path) -> Self {
        let home = TempDir::new().expect("create isolated home for daemon");
        let xdg_config_home = home.path().join("xdg");
        fs::create_dir_all(&xdg_config_home).expect("create isolated daemon xdg config home");

        let status = daemon_command(workdir, home.path(), &xdg_config_home)
            .args([
                "daemon",
                "start",
                "-d",
                "--http",
                "--host",
                "127.0.0.1",
                "--port",
                "0",
            ])
            .status()
            .expect("start GraphQL test daemon");
        assert!(status.success(), "daemon start should succeed");

        Self {
            workdir: workdir.to_path_buf(),
            _home: home,
            xdg_config_home,
        }
    }

    fn stop(&self) {
        let _ = daemon_command(&self.workdir, self._home.path(), &self.xdg_config_home)
            .args(["daemon", "stop"])
            .status();
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

fn daemon_command(workdir: &Path, home: &Path, xdg_config_home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bitloops"));
    command
        .current_dir(workdir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", xdg_config_home)
        .env(DISABLE_WATCHER_AUTOSTART_ENV, "1")
        .env(DISABLE_VERSION_CHECK_ENV, "1")
        .env_remove("BITLOOPS_DEVQL_PG_DSN")
        .env_remove("BITLOOPS_DEVQL_CH_URL")
        .env_remove("BITLOOPS_DEVQL_CH_DATABASE")
        .env_remove("BITLOOPS_DEVQL_CH_USER")
        .env_remove("BITLOOPS_DEVQL_CH_PASSWORD");
    command
}

pub fn seeded_rust_graphql_workspace(name: &str) -> SeededGraphqlWorkspace {
    let workspace = Workspace::new(name);
    write_rust_static_link_fixture(&workspace);
    prepare_graphql_workspace(&workspace);
    seed_production_artefacts(&workspace, "C0");
    let daemon = DaemonGuard::start(workspace.repo_dir());

    SeededGraphqlWorkspace {
        workspace,
        _daemon: daemon,
    }
}

pub fn run_query_json(seeded: &SeededGraphqlWorkspace, args: &[&str]) -> Value {
    serde_json::from_str(&run_bitloops_with_daemon_home_or_panic(
        seeded.workspace.repo_dir(),
        args,
        seeded._daemon._home.path(),
        &seeded._daemon.xdg_config_home,
    ))
    .expect("bitloops output should be valid JSON")
}

fn run_bitloops_with_daemon_home_or_panic(
    workdir: &Path,
    args: &[&str],
    home: &Path,
    xdg_config_home: &Path,
) -> String {
    let output = daemon_command(workdir, home, xdg_config_home)
        .args(args)
        .output()
        .expect("execute bitloops command");
    if !output.status.success() {
        panic!(
            "bitloops command failed in {}: {:?}\nstdout:\n{}\nstderr:\n{}",
            workdir.display(),
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout).expect("stdout should be valid utf-8")
}

pub fn extract_connection_nodes(payload: &Value) -> Vec<Value> {
    let connection = payload
        .get("repo")
        .and_then(|repo| repo.get("artefacts"))
        .unwrap_or_else(|| &payload["artefacts"]);
    connection["edges"]
        .as_array()
        .expect("artefact connection edges")
        .iter()
        .map(|edge| edge["node"].clone())
        .collect()
}
