use crate::test_harness_support::{
    Workspace, prepare_graphql_workspace, run_bitloops_or_panic, seed_production_artefacts,
    write_rust_static_link_fixture,
};
use bitloops::cli::versioncheck::DISABLE_VERSION_CHECK_ENV;
use bitloops::host::devql::watch::DISABLE_WATCHER_AUTOSTART_ENV;
use serde_json::Value;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub struct SeededGraphqlWorkspace {
    pub workspace: Workspace,
    pub repo_name: String,
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
        let port = pick_port().to_string();

        let status = daemon_command(workdir, home.path(), &xdg_config_home)
            .args([
                "daemon",
                "start",
                "-d",
                "--http",
                "--host",
                "127.0.0.1",
                "--port",
                port.as_str(),
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

fn pick_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral daemon port");
    let port = listener
        .local_addr()
        .expect("read daemon listener addr")
        .port();
    drop(listener);
    port
}

pub fn seeded_rust_graphql_workspace(name: &str) -> SeededGraphqlWorkspace {
    let workspace = Workspace::new(name);
    write_rust_static_link_fixture(&workspace);
    prepare_graphql_workspace(&workspace);
    seed_production_artefacts(&workspace, "C0");
    let daemon = DaemonGuard::start(workspace.repo_dir());

    let repo_name = workspace
        .repo_dir()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("workspace repo dir should have a UTF-8 file name")
        .to_string();

    SeededGraphqlWorkspace {
        workspace,
        repo_name,
        _daemon: daemon,
    }
}

pub fn run_query_json(workspace: &Workspace, args: &[&str]) -> Value {
    serde_json::from_str(&run_bitloops_or_panic(workspace.repo_dir(), args))
        .expect("bitloops output should be valid JSON")
}

pub fn extract_connection_nodes(payload: &Value) -> Vec<Value> {
    payload["repo"]["artefacts"]["edges"]
        .as_array()
        .expect("artefact connection edges")
        .iter()
        .map(|edge| edge["node"].clone())
        .collect()
}
