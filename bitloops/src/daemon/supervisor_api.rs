use super::*;

pub(super) async fn run_internal_supervisor(_args: InternalDaemonSupervisorArgs) -> Result<()> {
    let control_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding Bitloops daemon supervisor control listener")?;
    let control_addr = control_listener
        .local_addr()
        .context("reading Bitloops daemon supervisor listener address")?;
    let control_url = format!("http://127.0.0.1:{}", control_addr.port());
    let runtime_path = supervisor_runtime_state_path()?;
    let fingerprint = current_binary_fingerprint()?;
    log::info!("daemon supervisor started: control_url={}", control_url);

    write_json(
        &runtime_path,
        &SupervisorRuntimeState {
            version: 1,
            pid: std::process::id(),
            control_url: control_url.clone(),
            binary_fingerprint: fingerprint,
            updated_at_unix: unix_timestamp_now(),
        },
    )?;

    let app = Router::new()
        .route("/health", get(supervisor_health))
        .route("/daemon/start", post(handle_supervisor_start_repo))
        .route("/daemon/stop", post(handle_supervisor_stop_repo))
        .route("/daemon/restart", post(handle_supervisor_restart_repo))
        .with_state(SupervisorAppState {
            operation_lock: Arc::new(Mutex::new(())),
        });

    let result = axum::serve(control_listener, app)
        .with_graceful_shutdown(wait_for_shutdown_signal())
        .await;

    let _ = fs::remove_file(runtime_path);
    result.context("running Bitloops daemon supervisor")
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

fn supervisor_api_error(err: anyhow::Error) -> (axum::http::StatusCode, String) {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        err.to_string(),
    )
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = signal(SignalKind::terminate()).ok();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = async {
                if let Some(signal) = terminate.as_mut() {
                    signal.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
