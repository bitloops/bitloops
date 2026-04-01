use super::*;

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
        Ok(unix_kill_zero_indicates_running(
            result,
            std::io::Error::last_os_error().raw_os_error(),
        ))
    }
}

#[cfg(not(windows))]
fn unix_kill_zero_indicates_running(result: i32, raw_os_error: Option<i32>) -> bool {
    if result == 0 {
        return true;
    }

    matches!(raw_os_error, Some(libc::EPERM))
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

pub(super) fn wait_for_runtime_cleanup(runtime_path: &Path, timeout: Duration) -> Result<()> {
    let started = Instant::now();
    while runtime_path.exists() && started.elapsed() <= timeout {
        std::thread::sleep(Duration::from_millis(100));
    }
    if runtime_path.exists() {
        bail!(
            "Bitloops daemon did not shut down within {} seconds",
            timeout.as_secs()
        );
    }
    Ok(())
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
}
