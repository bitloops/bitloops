use super::*;

pub(super) async fn run_internal_supervisor(_args: InternalDaemonSupervisorArgs) -> Result<()> {
    let result = async {
        let control_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("binding Bitloops daemon supervisor control listener")?;
        let control_addr = control_listener
            .local_addr()
            .context("reading Bitloops daemon supervisor listener address")?;
        let control_url = format!("http://127.0.0.1:{}", control_addr.port());
        let fingerprint = current_binary_fingerprint()?;
        log::info!("daemon supervisor started: control_url={}", control_url);

        write_supervisor_runtime_state(&SupervisorRuntimeState {
            version: 1,
            pid: std::process::id(),
            control_url: control_url.clone(),
            binary_fingerprint: fingerprint,
            updated_at_unix: unix_timestamp_now(),
        })?;

        let app = Router::new()
            .route("/health", get(supervisor_health))
            .route("/daemon/start", post(handle_supervisor_start_repo))
            .route("/daemon/stop", post(handle_supervisor_stop_repo))
            .route("/daemon/restart", post(handle_supervisor_restart_repo))
            .with_state(SupervisorAppState {
                operation_lock: Arc::new(Mutex::new(())),
            });
        let child_reaper = spawn_supervisor_child_reaper();

        let result = axum::serve(control_listener, app)
            .with_graceful_shutdown(wait_for_shutdown_signal())
            .await;
        if let Some(reaper) = child_reaper {
            reaper.abort();
        }

        if let Err(err) = delete_supervisor_runtime_state() {
            log::warn!("failed to clear daemon supervisor runtime state on shutdown: {err:#}");
        }
        result.context("running Bitloops daemon supervisor")
    }
    .await;
    if let Err(err) = &result {
        log::error!("daemon supervisor failed: {err:#}");
    }
    result
}

#[cfg(unix)]
fn spawn_supervisor_child_reaper() -> Option<tokio::task::JoinHandle<()>> {
    Some(tokio::spawn(async {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigchld = match signal(SignalKind::child()) {
            Ok(signal) => signal,
            Err(err) => {
                log::warn!("failed to install SIGCHLD handler for daemon supervisor: {err:#}");
                return;
            }
        };
        while sigchld.recv().await.is_some() {
            match reap_terminated_child_processes() {
                Ok(reaped) if !reaped.is_empty() => {
                    log_reaped_child_processes(&reaped);
                }
                Ok(_) => {}
                Err(err) => {
                    log::warn!("failed to reap daemon supervisor child processes: {err:#}");
                }
            }
        }
    }))
}

#[cfg(not(unix))]
fn spawn_supervisor_child_reaper() -> Option<tokio::task::JoinHandle<()>> {
    None
}

async fn supervisor_health() -> Json<SupervisorHealthResponse> {
    Json(SupervisorHealthResponse {
        status: "ok".to_string(),
    })
}

async fn handle_supervisor_start_repo(
    State(state): State<SupervisorAppState>,
    Json(request): Json<SupervisorStartRequest>,
) -> Result<Json<DaemonRuntimeState>, (axum::http::StatusCode, String)> {
    let _guard = state.operation_lock.lock().await;
    log::info!(
        "supervisor start request: config={} host={:?} port={} force_http={} bundle_dir={:?}",
        request.config_path.display(),
        request.config.host,
        request.config.port,
        request.config.force_http,
        request.config.bundle_dir
    );
    let daemon_config =
        resolve_daemon_config(Some(request.config_path.as_path())).map_err(supervisor_api_error)?;
    ensure_service_managed_repo_runtime(&daemon_config, request.config, request.telemetry)
        .await
        .map(Json)
        .map_err(supervisor_api_error)
}

async fn handle_supervisor_stop_repo(
    State(state): State<SupervisorAppState>,
    Json(_request): Json<SupervisorStopRequest>,
) -> Result<Json<SupervisorHealthResponse>, (axum::http::StatusCode, String)> {
    let _guard = state.operation_lock.lock().await;
    log::info!("supervisor stop request received");
    stop_service_managed_repo_runtime().map_err(supervisor_api_error)?;
    Ok(Json(SupervisorHealthResponse {
        status: "ok".to_string(),
    }))
}

async fn handle_supervisor_restart_repo(
    State(state): State<SupervisorAppState>,
    Json(request): Json<SupervisorStartRequest>,
) -> Result<Json<DaemonRuntimeState>, (axum::http::StatusCode, String)> {
    let _guard = state.operation_lock.lock().await;
    log::info!(
        "supervisor restart request: config={} host={:?} port={} force_http={} bundle_dir={:?}",
        request.config_path.display(),
        request.config.host,
        request.config.port,
        request.config.force_http,
        request.config.bundle_dir
    );
    let daemon_config =
        resolve_daemon_config(Some(request.config_path.as_path())).map_err(supervisor_api_error)?;
    restart_service_managed_repo_runtime(&daemon_config, request.config)
        .await
        .map(Json)
        .map_err(supervisor_api_error)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownTrigger {
    CtrlC,
    Sigterm,
}

impl ShutdownTrigger {
    fn label(self) -> &'static str {
        match self {
            ShutdownTrigger::CtrlC => "ctrl-c",
            ShutdownTrigger::Sigterm => "sigterm",
        }
    }
}

fn supervisor_api_error(err: anyhow::Error) -> (axum::http::StatusCode, String) {
    log::error!("daemon supervisor request failed: {err:#}");
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        err.to_string(),
    )
}

async fn wait_for_shutdown_signal() {
    fn log_shutdown_signal(trigger: ShutdownTrigger) {
        log::info!(
            "daemon supervisor shutdown signal received: {}",
            trigger.label()
        );
    }

    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(signal) => Some(signal),
            Err(err) => {
                log::warn!("failed to install SIGTERM handler for daemon supervisor: {err:#}");
                None
            }
        };
        let trigger = select_unix_shutdown_trigger(tokio::signal::ctrl_c(), async {
            if let Some(signal) = terminate.as_mut() {
                signal.recv().await;
            } else {
                std::future::pending::<()>().await;
            }
        })
        .await;
        log_shutdown_signal(trigger);
    }

    #[cfg(not(unix))]
    {
        if let Err(err) = tokio::signal::ctrl_c().await {
            log::warn!("failed to install Ctrl-C handler for daemon supervisor: {err:#}");
            std::future::pending::<()>().await;
        }
        log_shutdown_signal(ShutdownTrigger::CtrlC);
    }
}

#[cfg(unix)]
async fn select_unix_shutdown_trigger<CtrlC, Sigterm>(
    ctrl_c: CtrlC,
    sigterm: Sigterm,
) -> ShutdownTrigger
where
    CtrlC: std::future::Future<Output = std::io::Result<()>>,
    Sigterm: std::future::Future<Output = ()>,
{
    tokio::select! {
        trigger = async {
            match ctrl_c.await {
                Ok(()) => ShutdownTrigger::CtrlC,
                Err(err) => {
                    log::warn!("failed to install Ctrl-C handler for daemon supervisor: {err:#}");
                    std::future::pending::<ShutdownTrigger>().await
                }
            }
        } => trigger,
        _ = sigterm => ShutdownTrigger::Sigterm,
    }
}

fn log_reaped_child_processes(reaped: &[ChildTerminationRecord]) {
    for child in reaped {
        if child.is_expected_shutdown() {
            log::info!(
                "daemon supervisor reaped child process: pid={} outcome={}",
                child.pid,
                child.summary()
            );
        } else {
            log::warn!(
                "daemon supervisor reaped child process: pid={} outcome={}",
                child.pid,
                child.summary()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::DashboardServerConfig;
    use crate::test_support::log_capture::{capture_logs, capture_logs_async};
    #[cfg(unix)]
    use std::io;

    #[tokio::test]
    async fn supervisor_api_logs_terminal_handoff_failure() {
        let state = SupervisorAppState {
            operation_lock: Arc::new(Mutex::new(())),
        };
        let request = SupervisorStartRequest {
            config_path: std::path::PathBuf::from("/tmp/missing-bitloops-config.toml"),
            config: DashboardServerConfig {
                host: None,
                port: crate::api::DEFAULT_DASHBOARD_PORT,
                no_open: true,
                force_http: false,
                recheck_local_dashboard_net: false,
                bundle_dir: None,
            },
            telemetry: None,
        };

        let (result, logs) =
            capture_logs_async(handle_supervisor_start_repo(State(state), Json(request))).await;

        assert!(
            result.is_err(),
            "missing config should fail supervisor start request"
        );
        assert!(
            logs.iter().any(|entry| entry.level == log::Level::Error
                && entry.message.contains("daemon supervisor request failed")),
            "expected supervisor API to log terminal handoff failure, got logs: {logs:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn supervisor_logs_reaped_sigterm_child_details() {
        let (_, logs) = capture_logs(|| {
            log_reaped_child_processes(&[ChildTerminationRecord {
                pid: 42,
                outcome: ChildTerminationOutcome::Signaled {
                    signal: libc::SIGTERM,
                    core_dumped: false,
                },
            }]);
        });

        assert!(
            logs.iter().any(|entry| entry.level == log::Level::Info
                && entry.message.contains("pid=42")
                && entry.message.contains("signal 15 (SIGTERM)")),
            "expected child reap diagnostic log entry, got logs: {logs:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn supervisor_shutdown_prefers_sigterm_when_ctrl_c_handler_fails() {
        let trigger = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            select_unix_shutdown_trigger(
                async { Err(io::Error::other("ctrl-c unavailable")) },
                async {},
            ),
        )
        .await
        .expect("shutdown trigger selection should not hang");

        assert_eq!(trigger, ShutdownTrigger::Sigterm);
    }
}
