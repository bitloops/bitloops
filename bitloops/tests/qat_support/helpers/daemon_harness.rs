use bitloops::daemon::DaemonRuntimeState;
use bitloops::host::runtime_store::DaemonSqliteRuntimeStore;
use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Child;

const DAEMON_READY_TIMEOUT: StdDuration = StdDuration::from_secs(60);
const DAEMON_READY_POLL_INTERVAL: StdDuration = StdDuration::from_millis(100);
const DAEMON_STDERR_LOG_FILE: &str = "daemon.stderr.log";

fn daemon_candidate_ports(run_dir: &Path) -> Vec<String> {
    let canonical = run_dir.canonicalize().unwrap_or_else(|_| run_dir.to_path_buf());
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    let seed = hasher.finish();

    let mut ports = Vec::new();
    if let Some(configured_port) = std::env::var("BITLOOPS_QAT_DAEMON_PORT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        ports.push(configured_port);
    }
    #[cfg(not(target_os = "macos"))]
    if !ports.iter().any(|port| port == "0") {
        ports.push("0".to_string());
    }

    for port in (0..8)
        .map(|offset| 32000 + (((seed as u16).wrapping_add((offset * 983) as u16)) % 20000))
        .map(|port| port.to_string())
    {
        if !ports.iter().any(|candidate| candidate == &port) {
            ports.push(port);
        }
    }
    ports
}

fn daemon_runtime_store_candidate_paths(run_dir: &Path) -> Vec<PathBuf> {
    let home_dir = run_dir.join("home");
    vec![
        home_dir
            .join("xdg-state")
            .join("bitloops")
            .join("daemon")
            .join("runtime.sqlite"),
        home_dir
            .join(".local")
            .join("state")
            .join("bitloops")
            .join("daemon")
            .join("runtime.sqlite"),
        home_dir
            .join("Library")
            .join("Application Support")
            .join("bitloops")
            .join("daemon")
            .join("runtime.sqlite"),
    ]
}

fn daemon_start_args(port: &str) -> Vec<String> {
    vec![
        "daemon".to_string(),
        "start".to_string(),
        "--create-default-config".to_string(),
        "--no-telemetry".to_string(),
        "--http".to_string(),
        "--host".to_string(),
        "127.0.0.1".to_string(),
        "--port".to_string(),
        port.to_string(),
    ]
}

fn daemon_stderr_log_path(run_dir: &Path) -> PathBuf {
    run_dir.join(DAEMON_STDERR_LOG_FILE)
}

fn spawn_daemon_process(
    world: &QatWorld,
    port: &str,
    stderr_log_path: &Path,
) -> Result<Child> {
    let stderr_log = File::create(stderr_log_path)
        .with_context(|| format!("creating daemon stderr log {}", stderr_log_path.display()))?;
    let args = daemon_start_args(port);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut command = build_bitloops_command(world, &arg_refs)?;
    command.stdout(Stdio::null()).stderr(Stdio::from(stderr_log));
    command
        .spawn()
        .with_context(|| format!("starting QAT daemon on port candidate {port}"))
}

fn daemon_probe_ready(state: &DaemonRuntimeState) -> bool {
    let Ok(mut stream) = TcpStream::connect((state.host.as_str(), state.port)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(StdDuration::from_millis(750)));
    let _ = stream.set_write_timeout(Some(StdDuration::from_millis(750)));
    let request = format!(
        "GET /devql/sdl HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        state.host
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut response = String::new();
    if stream.read_to_string(&mut response).is_err() {
        return false;
    }

    response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")
}

fn read_daemon_stderr_log(stderr_log_path: &Path) -> String {
    match fs::read_to_string(stderr_log_path) {
        Ok(output) if output.trim().is_empty() => "<no stderr output>".to_string(),
        Ok(output) => output,
        Err(err) => format!(
            "<failed reading stderr log {}: {err}>",
            stderr_log_path.display()
        ),
    }
}

fn read_runtime_state_for_child(
    run_dir: &Path,
    child_pid: u32,
) -> Option<(PathBuf, DaemonRuntimeState)> {
    daemon_runtime_store_candidate_paths(run_dir)
        .into_iter()
        .find_map(|path| {
            if !path.exists() {
                return None;
            }
            let store = DaemonSqliteRuntimeStore::open_at(path.clone()).ok()?;
            let state = store.load_runtime_state().ok().flatten()?;
            if state.pid == child_pid {
                Some((path, state))
            } else {
                None
            }
        })
}

fn wait_for_daemon_ready(
    run_dir: &Path,
    child: &mut Child,
    stderr_log_path: &Path,
) -> Result<(PathBuf, DaemonRuntimeState)> {
    let started = Instant::now();
    let child_pid = child.id();

    loop {
        if let Some((runtime_state_path, state)) = read_runtime_state_for_child(run_dir, child_pid)
            && daemon_probe_ready(&state)
        {
            return Ok((runtime_state_path, state));
        }

        match child
            .try_wait()
            .with_context(|| format!("polling daemon child process {child_pid}"))?
        {
            Some(status) => {
                let stderr = read_daemon_stderr_log(stderr_log_path);
                bail!(
                    "daemon process exited before readiness check succeeded\nchild pid: {child_pid}\nchild status: {status}\nstderr log: {}\nchild stderr:\n{stderr}",
                    stderr_log_path.display()
                );
            }
            None => {}
        }

        if started.elapsed() >= DAEMON_READY_TIMEOUT {
            let stderr = read_daemon_stderr_log(stderr_log_path);
            let runtime_candidates = daemon_runtime_store_candidate_paths(run_dir)
                .into_iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            bail!(
                "daemon did not become ready within {} seconds\nchild pid: {child_pid}\nstderr log: {}\nruntime store candidates:\n{}\nchild stderr:\n{stderr}",
                DAEMON_READY_TIMEOUT.as_secs(),
                stderr_log_path.display(),
                runtime_candidates
            );
        }

        std::thread::sleep(DAEMON_READY_POLL_INTERVAL);
    }
}
