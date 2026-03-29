use crate::test_harness_support::{
    Workspace, apply_repo_app_env, prepare_graphql_workspace, seed_production_artefacts,
    write_rust_static_link_fixture,
};
use bitloops::cli::versioncheck::DISABLE_VERSION_CHECK_ENV;
use bitloops::host::devql::watch::DISABLE_WATCHER_AUTOSTART_ENV;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct SeededGraphqlWorkspace {
    pub workspace: Workspace,
    _daemon: DaemonGuard,
}

struct DaemonGuard {
    workdir: PathBuf,
}

impl DaemonGuard {
    fn start(workdir: &Path) -> Self {
        let status = daemon_command(workdir)
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
        }
    }

    fn stop(&self) {
        let _ = daemon_command(&self.workdir)
            .args(["daemon", "stop"])
            .status();
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

fn daemon_command(workdir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bitloops"));
    command.current_dir(workdir);
    apply_repo_app_env(&mut command, workdir);
    command
        .env(DISABLE_WATCHER_AUTOSTART_ENV, "1")
        .env(DISABLE_VERSION_CHECK_ENV, "1");
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
    serde_json::from_str(&run_bitloops_or_panic(seeded.workspace.repo_dir(), args))
        .expect("bitloops output should be valid JSON")
}

fn run_bitloops_or_panic(workdir: &Path, args: &[&str]) -> String {
    let output = daemon_command(workdir)
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
