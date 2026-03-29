use crate::test_harness_support::{
    Workspace, apply_repo_app_env, prepare_graphql_workspace, seed_production_artefacts,
    with_repo_app_env, write_rust_static_link_fixture,
};
use bitloops::cli::versioncheck::DISABLE_VERSION_CHECK_ENV;
use bitloops::daemon::DaemonRuntimeState;
use bitloops::host::devql::watch::DISABLE_WATCHER_AUTOSTART_ENV;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub struct SeededGraphqlWorkspace {
    pub workspace: Workspace,
    _daemon: DaemonGuard,
}

struct DaemonGuard {
    child: Child,
}

impl DaemonGuard {
    fn start(workdir: &Path) -> Self {
        let mut last_error = None;
        for port in candidate_ports(workdir) {
            let child = daemon_command(workdir)
                .args([
                    "daemon",
                    "start",
                    "--http",
                    "--host",
                    "127.0.0.1",
                    "--port",
                    &port.to_string(),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start GraphQL test daemon");

            let mut guard = Self { child };
            match wait_until_ready(workdir, &mut guard.child) {
                Ok(()) => return guard,
                Err(err) => {
                    last_error = Some(format!("port {port}: {err}"));
                    let _ = guard.child.kill();
                    let _ = guard.child.wait();
                }
            }
        }

        panic!(
            "start GraphQL test daemon failed: {}",
            last_error.unwrap_or_else(|| "no candidate ports attempted".to_string())
        );
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

fn candidate_ports(workdir: &Path) -> Vec<u16> {
    let canonical = workdir.canonicalize().unwrap_or_else(|_| workdir.to_path_buf());
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    let seed = hasher.finish();

    (0..8)
        .map(|offset| 32000 + (((seed as u16).wrapping_add((offset * 983) as u16)) % 20000))
        .collect()
}

fn read_child_stderr(child: &mut Child) -> String {
    let Some(mut stderr) = child.stderr.take() else {
        return "<stderr unavailable>".to_string();
    };

    let mut output = String::new();
    match stderr.read_to_string(&mut output) {
        Ok(_) if output.trim().is_empty() => "<no stderr output>".to_string(),
        Ok(_) => output,
        Err(err) => format!("<failed reading stderr: {err}>"),
    }
}

fn wait_until_ready(workdir: &Path, child: &mut Child) -> Result<(), String> {
    let runtime_path = with_repo_app_env(workdir, || {
        bitloops::daemon::runtime_state_path(Path::new("."))
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime for GraphQL daemon guard");
    runtime.block_on(async move {
        let client = reqwest::Client::new();
        for _ in 0..600 {
            if let Ok(bytes) = std::fs::read(&runtime_path)
                && let Ok(state) = serde_json::from_slice::<DaemonRuntimeState>(&bytes)
            {
                let url = format!("{}/devql/sdl", state.url.trim_end_matches('/'));
                if let Ok(response) = client.get(&url).send().await
                    && response.status().is_success()
                {
                    return Ok(());
                }
            }

            match child.try_wait() {
                Ok(Some(status)) => {
                    let stderr = read_child_stderr(child);
                    return Err(format!(
                        "daemon process exited before readiness check succeeded using runtime state {}\nchild status: {status}\nchild stderr:\n{stderr}",
                        runtime_path.display()
                    ));
                }
                Ok(None) => {}
                Err(err) => {
                    return Err(format!(
                        "failed to inspect daemon process status while waiting for {}: {err}",
                        runtime_path.display()
                    ));
                }
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let (child_status, child_stderr) = match child.try_wait() {
            Ok(Some(status)) => (status.to_string(), read_child_stderr(child)),
            Ok(None) => (
                "still running".to_string(),
                "<child still running; stderr cannot be drained without stopping it>".to_string(),
            ),
            Err(err) => (
                format!("<failed to inspect status: {err}>"),
                "<stderr unavailable>".to_string(),
            ),
        };
        Err(format!(
            "daemon server did not become ready using runtime state {}\nchild status: {child_status}\nchild stderr:\n{child_stderr}",
            runtime_path.display()
        ))
    })
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
