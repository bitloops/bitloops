use super::*;

pub(super) async fn run_internal_process(args: InternalDaemonProcessArgs) -> Result<()> {
    let mode: DaemonMode = args.mode.into();
    let daemon_config = resolve_daemon_config(Some(args.config_path.as_path()))?;
    log::info!(
        "daemon process start: mode={} config={} host={:?} port={} service_name={:?}",
        mode,
        daemon_config.config_path.display(),
        args.host,
        args.port,
        args.service_name
    );
    run_server(
        &daemon_config,
        args.server_config(),
        RunServerOptions {
            mode,
            service_name: args.service_name,
            open_browser: false,
            ready_subject: "Bitloops daemon",
            print_banner: false,
            telemetry: args.telemetry,
        },
    )
    .await
}

pub(super) async fn start_foreground(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
    open_browser: bool,
    ready_subject: &str,
    telemetry: Option<bool>,
) -> Result<()> {
    ensure_can_start(daemon_config.config_root.as_path(), false)?;
    run_server(
        daemon_config,
        config,
        RunServerOptions {
            mode: DaemonMode::Foreground,
            service_name: None,
            open_browser,
            ready_subject,
            print_banner: true,
            telemetry,
        },
    )
    .await
}

pub(super) async fn start_detached(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
    telemetry: Option<bool>,
) -> Result<DaemonRuntimeState> {
    ensure_can_start(daemon_config.config_root.as_path(), false)?;
    let args = InternalDaemonProcessArgs::from_server_config(
        daemon_config,
        DaemonMode::Detached,
        None,
        &config,
        telemetry,
    );
    let mut command = build_daemon_spawn_command(&args)?;
    command
        .current_dir(&daemon_config.config_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = command.spawn().with_context(|| {
        format!(
            "spawning detached Bitloops daemon for {}",
            daemon_config.config_path.display()
        )
    })?;
    log::debug!("spawned detached daemon pid={}", child.id());
    wait_until_ready(READY_TIMEOUT).await
}

pub(super) async fn start_service(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
    telemetry: Option<bool>,
) -> Result<DaemonRuntimeState> {
    log::info!(
        "daemon service start requested: config={} host={:?} port={} force_http={} bundle_dir={:?}",
        daemon_config.config_path.display(),
        config.host,
        config.port,
        config.force_http,
        config.bundle_dir
    );
    supervisor_start_repo(daemon_config, config, telemetry).await
}

pub(super) async fn restart(
    config_override: Option<&ResolvedDaemonConfig>,
) -> Result<DaemonRuntimeState> {
    log::info!(
        "daemon restart requested: config_override={}",
        config_override
            .map(|config| config.config_path.display().to_string())
            .unwrap_or_else(|| "<current>".to_string())
    );
    let service = read_service_metadata(Path::new("."))?;
    let runtime = read_runtime_state(Path::new("."))?;

    if service.is_some() {
        let server_config = service
            .as_ref()
            .map(|metadata| metadata.config.clone())
            .context("service metadata missing for Bitloops daemon")?;
        let daemon_config = match config_override {
            Some(daemon_config) => daemon_config.clone(),
            None => {
                let config_path = service
                    .as_ref()
                    .map(|metadata| metadata.config_path.clone())
                    .context("daemon config path missing for Bitloops service metadata")?;
                resolve_daemon_config(Some(config_path.as_path()))?
            }
        };
        return supervisor_restart_repo(&daemon_config, server_config).await;
    }

    let runtime = runtime
        .context("Bitloops daemon is not running. Start it with `bitloops daemon start`.")?;
    let config = DashboardServerConfig {
        host: Some(runtime.host.clone()),
        port: runtime.port,
        no_open: true,
        force_http: runtime.url.starts_with("http://"),
        recheck_local_dashboard_net: false,
        bundle_dir: Some(runtime.bundle_dir.clone()),
    };
    stop().await?;
    let daemon_config = match config_override {
        Some(daemon_config) => daemon_config.clone(),
        None => resolve_daemon_config(Some(runtime.config_path.as_path()))?,
    };
    match runtime.mode {
        DaemonMode::Detached => start_detached(&daemon_config, config, None).await,
        DaemonMode::Foreground => {
            bail!(
                "cannot restart a foreground daemon from another process; run `bitloops daemon start` again"
            )
        }
        DaemonMode::Service => {
            bail!("service-backed daemon should have been handled before local runtime restart")
        }
    }
}

pub(super) async fn stop() -> Result<()> {
    log::info!("daemon stop requested");
    let service = read_service_metadata(Path::new("."))?;
    let runtime = read_runtime_state(Path::new("."))?;

    if let Some(metadata) = service {
        if supervisor_available().unwrap_or(false) {
            if let Err(err) = supervisor_stop_repo().await {
                if runtime.is_some() {
                    log::debug!("supervisor stop fallback to direct runtime stop: {err:#}");
                    stop_service_managed_repo_runtime()?;
                } else {
                    return Err(err);
                }
            }
        } else if runtime.is_some() {
            stop_service_managed_repo_runtime()?;
        }
        let runtime_path = runtime_state_path(Path::new("."));
        if runtime_path.exists() {
            wait_for_runtime_cleanup(&runtime_path, STOP_TIMEOUT)?;
        }
        log::info!("daemon stop completed for service-managed runtime");
        let _ = metadata;
        return Ok(());
    }

    let runtime = runtime
        .context("Bitloops daemon is not running. Start it with `bitloops daemon start`.")?;
    log::info!(
        "daemon stopping process pid={} mode={}",
        runtime.pid,
        runtime.mode
    );
    terminate_process(runtime.pid)?;
    wait_for_runtime_cleanup(&runtime_state_path(Path::new(".")), STOP_TIMEOUT)?;
    log::info!("daemon stop completed");
    Ok(())
}

pub(super) async fn status() -> Result<DaemonStatusReport> {
    let runtime = read_runtime_state(Path::new("."))?;
    let service = read_service_metadata(Path::new("."))?;
    let service_running = service.is_some()
        && read_supervisor_service_metadata()?
            .as_ref()
            .map(|metadata| is_supervisor_service_running(metadata).unwrap_or(false))
            .unwrap_or(false);
    let health = match runtime.as_ref() {
        Some(state) => query_health(state).await.ok(),
        None => None,
    };

    Ok(DaemonStatusReport {
        runtime,
        service,
        service_running,
        health,
    })
}

pub(super) async fn wait_until_ready(timeout: Duration) -> Result<DaemonRuntimeState> {
    let started = Instant::now();
    loop {
        if started.elapsed() > timeout {
            bail!(
                "Bitloops daemon did not become ready within {} seconds",
                timeout.as_secs()
            );
        }

        if let Some(state) = read_runtime_state(Path::new("."))?
            && daemon_http_ready(&state).await
        {
            return Ok(state);
        }

        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}
