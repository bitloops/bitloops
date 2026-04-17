use super::*;

const SUPERVISOR_READY_REQUEST_TIMEOUT: Duration = Duration::from_millis(500);

pub(super) async fn ensure_supervisor_available() -> Result<SupervisorRuntimeState> {
    let metadata = install_or_update_supervisor_service()?;
    let current = match current_binary_fingerprint() {
        Ok(fingerprint) => fingerprint,
        Err(err) => {
            log::warn!(
                "failed to determine current daemon binary fingerprint while checking supervisor runtime: {err:#}"
            );
            String::new()
        }
    };
    if let Some(runtime) = read_supervisor_runtime_state()?
        && supervisor_http_ready(&runtime).await
        && (runtime.binary_fingerprint == current || current.is_empty())
    {
        return Ok(runtime);
    }

    start_configured_supervisor_service(&metadata)?;
    wait_until_supervisor_ready(READY_TIMEOUT).await
}

pub(super) fn supervisor_available() -> Result<bool> {
    Ok(read_supervisor_runtime_state()?.is_some())
}

pub(super) async fn wait_until_supervisor_ready(
    timeout: Duration,
) -> Result<SupervisorRuntimeState> {
    let started = Instant::now();
    loop {
        if started.elapsed() > timeout {
            bail!(
                "Bitloops daemon supervisor did not become ready within {} seconds",
                timeout.as_secs()
            );
        }

        if let Some(runtime) = read_supervisor_runtime_state()?
            && supervisor_http_ready(&runtime).await
        {
            return Ok(runtime);
        }

        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

async fn supervisor_http_ready(runtime: &SupervisorRuntimeState) -> bool {
    reqwest::Client::builder()
        .timeout(SUPERVISOR_READY_REQUEST_TIMEOUT)
        .build()
        .expect("supervisor readiness client should build")
        .get(format!(
            "{}/health",
            runtime.control_url.trim_end_matches('/')
        ))
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

pub(super) async fn supervisor_start_repo(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
    telemetry: Option<bool>,
) -> Result<DaemonRuntimeState> {
    let runtime = ensure_supervisor_available().await?;
    let response = reqwest::Client::new()
        .post(format!(
            "{}/daemon/start",
            runtime.control_url.trim_end_matches('/')
        ))
        .json(&SupervisorStartRequest {
            config_path: daemon_config.config_path.clone(),
            config,
            telemetry,
        })
        .send()
        .await
        .context("sending start request to Bitloops daemon supervisor")?;
    decode_supervisor_response(response).await
}

pub(super) async fn supervisor_stop_repo() -> Result<()> {
    let runtime =
        read_supervisor_runtime_state()?.context("Bitloops daemon supervisor is not running")?;
    let response = reqwest::Client::new()
        .post(format!(
            "{}/daemon/stop",
            runtime.control_url.trim_end_matches('/')
        ))
        .json(&SupervisorStopRequest {})
        .send()
        .await
        .context("sending stop request to Bitloops daemon supervisor")?;
    decode_supervisor_response::<SupervisorHealthResponse>(response)
        .await
        .map(|_| ())
}

pub(super) async fn supervisor_restart_repo(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
) -> Result<DaemonRuntimeState> {
    let runtime = ensure_supervisor_available().await?;
    let response = reqwest::Client::new()
        .post(format!(
            "{}/daemon/restart",
            runtime.control_url.trim_end_matches('/')
        ))
        .json(&SupervisorStartRequest {
            config_path: daemon_config.config_path.clone(),
            config,
            telemetry: None,
        })
        .send()
        .await
        .context("sending restart request to Bitloops daemon supervisor")?;
    decode_supervisor_response(response).await
}

async fn decode_supervisor_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!(
            "Bitloops daemon supervisor returned HTTP {}{}",
            status,
            if body.trim().is_empty() {
                "".to_string()
            } else {
                format!(": {}", body.trim())
            }
        );
    }

    response
        .json::<T>()
        .await
        .context("decoding Bitloops daemon supervisor response")
}

#[cfg(test)]
mod tests {
    use super::SUPERVISOR_READY_REQUEST_TIMEOUT;
    use std::time::Duration;

    #[test]
    fn supervisor_readiness_probe_timeout_is_short() {
        assert_eq!(SUPERVISOR_READY_REQUEST_TIMEOUT, Duration::from_millis(500));
    }
}
