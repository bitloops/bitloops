use super::types::INTERNAL_DAEMON_COMMAND_NAME;
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChildTerminationRecord {
    pub(super) pid: u32,
    pub(super) outcome: ChildTerminationOutcome,
}

impl ChildTerminationRecord {
    pub(super) fn summary(&self) -> String {
        self.outcome.summary()
    }

    pub(super) fn is_expected_shutdown(&self) -> bool {
        match self.outcome {
            ChildTerminationOutcome::Exited { code } => code == 0,
            ChildTerminationOutcome::Signaled { signal, .. } => {
                signal == expected_shutdown_signal_term()
                    || signal == expected_shutdown_signal_int()
            }
            ChildTerminationOutcome::Stopped { .. }
            | ChildTerminationOutcome::Continued
            | ChildTerminationOutcome::Unknown { .. } => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChildTerminationOutcome {
    Exited { code: i32 },
    Signaled { signal: i32, core_dumped: bool },
    Stopped { signal: i32 },
    Continued,
    Unknown { raw_status: i32 },
}

impl ChildTerminationOutcome {
    fn summary(&self) -> String {
        match self {
            ChildTerminationOutcome::Exited { code } => format!("exited with code {code}"),
            ChildTerminationOutcome::Signaled {
                signal,
                core_dumped,
            } => {
                let signal_name = signal_name(*signal)
                    .map(|name| format!(" ({name})"))
                    .unwrap_or_default();
                if *core_dumped {
                    format!("signal {signal}{signal_name}, core dumped")
                } else {
                    format!("signal {signal}{signal_name}")
                }
            }
            ChildTerminationOutcome::Stopped { signal } => {
                let signal_name = signal_name(*signal)
                    .map(|name| format!(" ({name})"))
                    .unwrap_or_default();
                format!("stopped by signal {signal}{signal_name}")
            }
            ChildTerminationOutcome::Continued => "continued".to_string(),
            ChildTerminationOutcome::Unknown { raw_status } => {
                format!("unknown wait status {raw_status}")
            }
        }
    }
}

#[cfg(unix)]
fn signal_name(signal: i32) -> Option<&'static str> {
    match signal {
        libc::SIGTERM => Some("SIGTERM"),
        libc::SIGINT => Some("SIGINT"),
        libc::SIGKILL => Some("SIGKILL"),
        libc::SIGABRT => Some("SIGABRT"),
        libc::SIGSEGV => Some("SIGSEGV"),
        libc::SIGBUS => Some("SIGBUS"),
        libc::SIGILL => Some("SIGILL"),
        libc::SIGQUIT => Some("SIGQUIT"),
        libc::SIGTRAP => Some("SIGTRAP"),
        _ => None,
    }
}

#[cfg(not(unix))]
fn signal_name(_signal: i32) -> Option<&'static str> {
    None
}

fn expected_shutdown_signal_term() -> i32 {
    #[cfg(unix)]
    {
        libc::SIGTERM
    }
    #[cfg(not(unix))]
    {
        15
    }
}

fn expected_shutdown_signal_int() -> i32 {
    #[cfg(unix)]
    {
        libc::SIGINT
    }
    #[cfg(not(unix))]
    {
        2
    }
}

pub(super) fn current_binary_fingerprint() -> Result<String> {
    let current_exe = env::current_exe().context("resolving Bitloops executable path")?;
    let bytes = fs::read(&current_exe)
        .with_context(|| format!("reading Bitloops executable {}", current_exe.display()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub(super) fn build_daemon_spawn_command(args: &InternalDaemonProcessArgs) -> Result<Command> {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    let executable = env::current_exe().context("resolving Bitloops executable for daemon")?;
    let mut command = Command::new(executable);
    command.args(args.argv());
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    Ok(command)
}

pub(super) fn process_is_running(pid: u32) -> Result<bool> {
    #[cfg(windows)]
    {
        Ok(Command::new("cmd")
            .args([
                "/C",
                &format!("tasklist /FI \"PID eq {pid}\" | findstr {pid}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false))
    }

    #[cfg(not(windows))]
    {
        if pid > i32::MAX as u32 {
            return Ok(false);
        }

        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        let visible = unix_kill_zero_indicates_running(
            result,
            std::io::Error::last_os_error().raw_os_error(),
        );
        if !visible {
            return Ok(false);
        }

        Ok(!unix_process_is_zombie(pid))
    }
}

pub(super) fn running_internal_daemon_process_pids_for_config(
    config_path: &Path,
) -> Result<Vec<u32>> {
    #[cfg(unix)]
    {
        let output = Command::new("ps")
            .args(["-axo", "pid=,command="])
            .stdin(Stdio::null())
            .output()
            .context("listing Bitloops daemon processes")?;
        if !output.status.success() {
            return Ok(Vec::new());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_internal_daemon_process_pids(
            &stdout,
            std::process::id(),
            Some(config_path),
        ))
    }

    #[cfg(not(unix))]
    {
        let _ = config_path;
        Ok(Vec::new())
    }
}

pub(super) fn parse_internal_daemon_process_pids(
    ps_output: &str,
    current_pid: u32,
    config_path: Option<&Path>,
) -> Vec<u32> {
    ps_output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let (pid_raw, command) = trimmed.split_once(char::is_whitespace)?;
            let pid = pid_raw.parse::<u32>().ok()?;
            if pid == current_pid || !command.contains(INTERNAL_DAEMON_COMMAND_NAME) {
                return None;
            }
            if let Some(config_path) = config_path
                && !command_matches_config_path(command, config_path)
            {
                return None;
            }
            Some(pid)
        })
        .collect()
}

fn command_matches_config_path(command: &str, config_path: &Path) -> bool {
    let expected = config_path.to_string_lossy();
    command_contains_exact_flag_value(command, "--config-path ", &expected)
        || command_contains_exact_flag_value(command, "--config-path=", &expected)
}

fn command_contains_exact_flag_value(command: &str, prefix: &str, expected: &str) -> bool {
    let needle = format!("{prefix}{expected}");
    let mut search_from = 0;
    while let Some(offset) = command[search_from..].find(&needle) {
        let end = search_from + offset + needle.len();
        if command[end..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
        {
            return true;
        }
        search_from = end;
    }
    false
}

#[cfg(not(windows))]
fn unix_kill_zero_indicates_running(result: i32, raw_os_error: Option<i32>) -> bool {
    if result == 0 {
        return true;
    }

    matches!(raw_os_error, Some(libc::EPERM))
}

#[cfg(not(windows))]
fn unix_process_is_zombie(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    String::from_utf8_lossy(&output.stdout)
        .trim_start()
        .starts_with('Z')
}

pub(super) fn terminate_process(pid: u32) -> Result<()> {
    #[cfg(windows)]
    {
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `taskkill` for Bitloops daemon")?;
        if !status.success() {
            bail!("failed to stop Bitloops daemon process {pid}");
        }
    }

    #[cfg(not(windows))]
    {
        let status = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `kill -TERM` for Bitloops daemon")?;
        if !status.success() {
            bail!("failed to stop Bitloops daemon process {pid}");
        }
    }

    Ok(())
}

pub(super) fn terminate_process_and_wait_for_shutdown_cleanup(
    pid: u32,
    timeout: Duration,
    runtime_clean_exit_grace: Duration,
    force_kill_timeout: Duration,
) -> Result<()> {
    terminate_process(pid)?;
    wait_for_shutdown_cleanup_with_force_kill(
        pid,
        timeout,
        runtime_clean_exit_grace,
        force_kill_timeout,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShutdownCleanupStatus {
    process_exited: bool,
    runtime_cleaned: bool,
}

impl ShutdownCleanupStatus {
    fn complete(self) -> bool {
        self.process_exited && self.runtime_cleaned
    }
}

fn force_kill_process(pid: u32) -> Result<()> {
    if !process_is_running(pid)? {
        return Ok(());
    }

    #[cfg(windows)]
    {
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `taskkill /F` for Bitloops daemon")?;
        if !status.success() {
            bail!("failed to force-stop Bitloops daemon process {pid}");
        }
    }

    #[cfg(not(windows))]
    {
        let status = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `kill -KILL` for Bitloops daemon")?;
        if !status.success() {
            bail!("failed to force-stop Bitloops daemon process {pid}");
        }
    }

    Ok(())
}

#[cfg(unix)]
pub(super) fn reap_terminated_child_process(pid: u32, timeout: Duration) -> Result<bool> {
    if pid > i32::MAX as u32 {
        return Ok(false);
    }

    let deadline = Instant::now() + timeout;
    loop {
        let mut status = 0;
        let result = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
        if result == pid as libc::pid_t {
            return Ok(true);
        }
        if result == 0 {
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(25));
            continue;
        }

        let raw_os_error = std::io::Error::last_os_error().raw_os_error();
        match raw_os_error {
            Some(libc::ECHILD) => return Ok(false),
            Some(libc::EINTR) => continue,
            _ => {
                return Err(std::io::Error::last_os_error())
                    .context("waiting for Bitloops daemon child process");
            }
        }
    }
}

#[cfg(not(unix))]
pub(super) fn reap_terminated_child_process(_pid: u32, _timeout: Duration) -> Result<bool> {
    Ok(false)
}

#[cfg(unix)]
pub(super) fn reap_terminated_child_processes() -> Result<Vec<ChildTerminationRecord>> {
    let mut reaped = Vec::new();
    loop {
        let mut status = 0;
        let result = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if result > 0 {
            reaped.push(ChildTerminationRecord {
                pid: result as u32,
                outcome: decode_child_termination_status(status),
            });
            continue;
        }
        if result == 0 {
            return Ok(reaped);
        }

        let raw_os_error = std::io::Error::last_os_error().raw_os_error();
        return match raw_os_error {
            Some(libc::ECHILD) => Ok(reaped),
            Some(libc::EINTR) => continue,
            _ => Err(std::io::Error::last_os_error())
                .context("reaping Bitloops daemon child processes"),
        };
    }
}

#[cfg(unix)]
fn decode_child_termination_status(status: i32) -> ChildTerminationOutcome {
    if libc::WIFEXITED(status) {
        return ChildTerminationOutcome::Exited {
            code: libc::WEXITSTATUS(status),
        };
    }
    if libc::WIFSIGNALED(status) {
        return ChildTerminationOutcome::Signaled {
            signal: libc::WTERMSIG(status),
            core_dumped: libc::WCOREDUMP(status),
        };
    }
    if libc::WIFSTOPPED(status) {
        return ChildTerminationOutcome::Stopped {
            signal: libc::WSTOPSIG(status),
        };
    }
    if libc::WIFCONTINUED(status) {
        return ChildTerminationOutcome::Continued;
    }
    ChildTerminationOutcome::Unknown { raw_status: status }
}

#[cfg(not(unix))]
pub(super) fn reap_terminated_child_processes() -> Result<Vec<ChildTerminationRecord>> {
    Ok(Vec::new())
}

pub(super) fn wait_for_shutdown_cleanup(pid: u32, timeout: Duration) -> Result<()> {
    wait_for_shutdown_cleanup_with_force_kill(
        pid,
        timeout,
        STOP_RUNTIME_CLEAN_EXIT_GRACE,
        FORCE_KILL_TIMEOUT,
    )
}

fn process_has_exited_or_was_reaped(pid: u32) -> Result<bool> {
    #[cfg(unix)]
    if reap_terminated_child_process(pid, Duration::ZERO)? {
        return Ok(true);
    }

    process_is_running(pid).map(|running| !running)
}

fn wait_for_shutdown_cleanup_with_force_kill(
    pid: u32,
    timeout: Duration,
    runtime_clean_exit_grace: Duration,
    force_kill_timeout: Duration,
) -> Result<()> {
    let graceful_started = Instant::now();
    let mut runtime_cleaned_since = None::<Instant>;
    let mut force_kill_started = None::<Instant>;

    loop {
        let status = shutdown_cleanup_status(pid)?;
        if status.complete() {
            return Ok(());
        }

        if let Some(force_started) = force_kill_started {
            if status.process_exited {
                cleanup_stale_runtime_state_after_forced_shutdown(status);
                return Ok(());
            }
            if force_started.elapsed() > force_kill_timeout {
                bail!(
                    "Bitloops daemon did not shut down after forced termination (process_exited={}, runtime_cleaned={})",
                    status.process_exited,
                    status.runtime_cleaned
                );
            }
        } else {
            if status.runtime_cleaned && !status.process_exited {
                let cleaned_since = runtime_cleaned_since.get_or_insert_with(Instant::now);
                if cleaned_since.elapsed() >= runtime_clean_exit_grace {
                    log::warn!(
                        "daemon process {} remained alive for {:?} after runtime cleanup; forcing termination",
                        pid,
                        runtime_clean_exit_grace
                    );
                    force_kill_process(pid)?;
                    force_kill_started = Some(Instant::now());
                    continue;
                }
            } else {
                runtime_cleaned_since = None;
            }

            if graceful_started.elapsed() > timeout {
                if !status.process_exited {
                    log::warn!(
                        "daemon process {} exceeded graceful shutdown timeout of {:?}; forcing termination (runtime_cleaned={})",
                        pid,
                        timeout,
                        status.runtime_cleaned
                    );
                    force_kill_process(pid)?;
                    force_kill_started = Some(Instant::now());
                    continue;
                }

                cleanup_stale_runtime_state_after_forced_shutdown(status);
                return Ok(());
            }
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn shutdown_cleanup_status(pid: u32) -> Result<ShutdownCleanupStatus> {
    Ok(ShutdownCleanupStatus {
        process_exited: process_has_exited_or_was_reaped(pid)?,
        runtime_cleaned: read_runtime_state(Path::new("."))?.is_none(),
    })
}

fn cleanup_stale_runtime_state_after_forced_shutdown(status: ShutdownCleanupStatus) {
    if !status.runtime_cleaned
        && let Err(err) = delete_runtime_state()
    {
        log::warn!("failed to clear stale daemon runtime state after forced shutdown: {err:#}");
    }
}

pub(super) async fn daemon_http_ready(state: &DaemonRuntimeState) -> bool {
    let client = match daemon_http_client(&state.url) {
        Ok(client) => client,
        Err(_) => return false,
    };
    let url = format!("{}/devql/sdl", state.url.trim_end_matches('/'));
    client
        .get(url)
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

pub(super) fn daemon_http_client(url: &str) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if should_accept_invalid_daemon_certs(url) {
        builder = builder.danger_accept_invalid_certs(true);
    }
    builder
        .build()
        .context("building Bitloops daemon HTTP client")
}

pub(super) async fn query_health(state: &DaemonRuntimeState) -> Result<DaemonHealthSummary> {
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct HealthEnvelope {
        health: HealthPayload,
    }

    #[derive(Debug, Deserialize)]
    struct HealthPayload {
        relational: Option<HealthBackend>,
        events: Option<HealthBackend>,
        blob: Option<HealthBackend>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct HealthBackend {
        backend: Option<String>,
        connected: Option<bool>,
    }

    let payload: HealthEnvelope = execute_graphql(
        &state.config_root,
        r#"{ health { relational { backend connected } events { backend connected } blob { backend connected } } }"#,
        json!({}),
    )
    .await?;

    Ok(DaemonHealthSummary {
        relational_backend: payload
            .health
            .relational
            .as_ref()
            .and_then(|value| value.backend.clone()),
        relational_connected: payload.health.relational.and_then(|value| value.connected),
        events_backend: payload
            .health
            .events
            .as_ref()
            .and_then(|value| value.backend.clone()),
        events_connected: payload.health.events.and_then(|value| value.connected),
        blob_backend: payload
            .health
            .blob
            .as_ref()
            .and_then(|value| value.backend.clone()),
        blob_connected: payload.health.blob.and_then(|value| value.connected),
    })
}

fn should_accept_invalid_daemon_certs(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }

    matches!(
        parsed.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("::1") | Some("[::1]")
    )
}

#[cfg(test)]
mod tests {
    use super::should_accept_invalid_daemon_certs;
    #[cfg(not(windows))]
    use super::unix_kill_zero_indicates_running;
    #[cfg(unix)]
    use super::{
        process_is_running, reap_terminated_child_process,
        terminate_process_and_wait_for_shutdown_cleanup, wait_for_shutdown_cleanup,
    };
    #[cfg(unix)]
    use crate::test_support::process_state::enter_process_state;
    #[cfg(unix)]
    use std::process::Command;
    #[cfg(unix)]
    use std::time::{Duration, Instant};
    #[cfg(unix)]
    use tempfile::TempDir;

    #[test]
    fn daemon_http_client_only_relaxes_loopback_https_urls() {
        assert!(should_accept_invalid_daemon_certs("https://localhost:5667"));
        assert!(should_accept_invalid_daemon_certs("https://127.0.0.1:5667"));
        assert!(should_accept_invalid_daemon_certs("https://[::1]:5667"));
        assert!(!should_accept_invalid_daemon_certs("http://127.0.0.1:5667"));
        assert!(!should_accept_invalid_daemon_certs(
            "https://dev.internal:5667"
        ));
        assert!(!should_accept_invalid_daemon_certs("not-a-url"));
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_pid_probe_treats_permission_denied_as_running() {
        assert!(unix_kill_zero_indicates_running(-1, Some(libc::EPERM)));
        assert!(!unix_kill_zero_indicates_running(-1, Some(libc::ESRCH)));
        assert!(!unix_kill_zero_indicates_running(-1, Some(libc::EINVAL)));
        assert!(unix_kill_zero_indicates_running(0, None));
    }

    #[cfg(unix)]
    #[test]
    fn process_liveness_treats_zombie_child_as_exited() {
        let child = Command::new("sh")
            .args(["-c", "exit 0"])
            .spawn()
            .expect("spawn short-lived child");
        let pid = child.id();
        drop(child);

        let deadline = Instant::now() + Duration::from_secs(2);
        while process_is_running(pid).expect("inspect child process state before reap") {
            assert!(
                Instant::now() < deadline,
                "expected exited zombie child to be treated as no longer running"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            reap_terminated_child_process(pid, Duration::from_secs(1))
                .expect("reap child process by pid"),
            "expected exited child to be reaped"
        );
        assert!(
            !process_is_running(pid).expect("inspect child process state after reap"),
            "expected reaped child to disappear from process table"
        );
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_cleanup_waits_for_child_process_exit_even_without_runtime_state() {
        let cwd = TempDir::new().expect("temp cwd");
        let state_root = TempDir::new().expect("temp state root");
        let state_root_str = state_root.path().to_string_lossy().to_string();
        let _guard = enter_process_state(
            Some(cwd.path()),
            &[(
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_root_str.as_str()),
            )],
        );
        let child = Command::new("sh")
            .args(["-c", "sleep 0.05"])
            .spawn()
            .expect("spawn short-lived child");
        let pid = child.id();
        drop(child);

        wait_for_shutdown_cleanup(pid, Duration::from_secs(1))
            .expect("wait for child shutdown cleanup");
        assert!(
            !process_is_running(pid).expect("inspect child process state after shutdown wait"),
            "expected shutdown cleanup to wait until the child is gone"
        );
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_cleanup_force_kills_lingering_process_once_runtime_is_cleaned() {
        let cwd = TempDir::new().expect("temp cwd");
        let state_root = TempDir::new().expect("temp state root");
        let state_root_str = state_root.path().to_string_lossy().to_string();
        let _guard = enter_process_state(
            Some(cwd.path()),
            &[(
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_root_str.as_str()),
            )],
        );
        let child = Command::new("sh")
            .args(["-c", "trap '' TERM; exec sleep 60"])
            .spawn()
            .expect("spawn TERM-ignoring child");
        let pid = child.id();
        drop(child);

        let started = Instant::now();
        terminate_process_and_wait_for_shutdown_cleanup(
            pid,
            Duration::from_secs(2),
            Duration::from_millis(100),
            Duration::from_secs(1),
        )
        .expect("force-kill lingering child during shutdown cleanup");

        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "expected forced shutdown cleanup to finish early, elapsed={:?}",
            started.elapsed()
        );
        assert!(
            !process_is_running(pid).expect("inspect child process state after forced shutdown"),
            "expected forced shutdown cleanup to terminate lingering child"
        );
    }
}
