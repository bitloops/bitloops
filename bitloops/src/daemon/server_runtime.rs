use super::*;

pub(super) async fn run_server(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
    mode: DaemonMode,
    service_name: Option<String>,
    open_browser: bool,
    ready_subject: &str,
    print_banner: bool,
) -> Result<()> {
    let config_root = daemon_config
        .config_root
        .canonicalize()
        .unwrap_or_else(|_| daemon_config.config_root.clone());
    let runtime_path = runtime_state_path(&config_root);
    let service_metadata_path = service_metadata_path(&config_root);
    let current_fingerprint = current_binary_fingerprint()?;
    let on_ready_config = daemon_config.clone();
    let on_ready_runtime_path = runtime_path.clone();
    let on_ready_service_metadata_path = service_metadata_path.clone();
    let on_ready_service_name = service_name.clone();
    let ready_hook: DashboardReadyHook = std::sync::Arc::new(move |ready| {
        write_runtime_state(
            &on_ready_runtime_path,
            &DaemonRuntimeState {
                version: 1,
                config_path: on_ready_config.config_path.clone(),
                config_root: on_ready_config.config_root.clone(),
                pid: std::process::id(),
                mode,
                service_name: on_ready_service_name.clone(),
                url: ready.url.clone(),
                host: ready.host.clone(),
                port: ready.port,
                bundle_dir: ready.bundle_dir.clone(),
                relational_db_path: on_ready_config.relational_db_path.clone(),
                events_db_path: on_ready_config.events_db_path.clone(),
                blob_store_path: on_ready_config.blob_store_path.clone(),
                repo_registry_path: on_ready_config.repo_registry_path.clone(),
                binary_fingerprint: current_fingerprint.clone(),
                updated_at_unix: unix_timestamp_now(),
            },
        )?;

        if matches!(mode, DaemonMode::Service)
            && let Ok(Some(mut metadata)) =
                read_service_metadata_for_path(&on_ready_service_metadata_path)
        {
            metadata.last_url = Some(ready.url.clone());
            metadata.last_pid = Some(std::process::id());
            write_json(&on_ready_service_metadata_path, &metadata)?;
        }

        Ok(())
    });
    let on_shutdown_runtime_path = runtime_path.clone();
    let on_shutdown = std::sync::Arc::new(move || {
        let _ = fs::remove_file(&on_shutdown_runtime_path);
    });

    api::run_with_options(
        config,
        DashboardRuntimeOptions {
            ready_subject: ready_subject.to_string(),
            print_ready_banner: print_banner,
            open_browser,
            shutdown_message: Some("Bitloops daemon stopped.".to_string()),
            on_ready: Some(ready_hook),
            on_shutdown: Some(on_shutdown),
            config_root: Some(config_root),
            repo_registry_path: Some(daemon_config.repo_registry_path.clone()),
        },
    )
    .await
}

pub(super) async fn ensure_service_managed_repo_runtime(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
) -> Result<DaemonRuntimeState> {
    let config_root = daemon_config
        .config_root
        .canonicalize()
        .unwrap_or_else(|_| daemon_config.config_root.clone());

    if let Some(runtime) = read_runtime_state(&config_root)? {
        if runtime.mode == DaemonMode::Service
            && runtime.service_name.as_deref() == Some(GLOBAL_SUPERVISOR_SERVICE_NAME)
        {
            install_or_update_repo_service_binding(daemon_config, config)?;
            if daemon_http_ready(&runtime).await {
                return Ok(runtime);
            }
            return wait_until_ready(READY_TIMEOUT).await;
        }

        bail!(
            "Bitloops daemon is already running at {}. Use `bitloops daemon restart` if you need to replace it.",
            runtime.url
        );
    }

    let binding = install_or_update_repo_service_binding(daemon_config, config.clone())?;
    let args = InternalDaemonProcessArgs::from_server_config(
        daemon_config,
        DaemonMode::Service,
        Some(binding.service_name.clone()),
        &config,
    );
    let mut command = build_daemon_spawn_command(&args)?;
    command
        .current_dir(&config_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    command.spawn().with_context(|| {
        format!(
            "spawning service-managed Bitloops daemon for {}",
            daemon_config.config_path.display()
        )
    })?;

    wait_until_ready(READY_TIMEOUT).await
}

pub(super) fn stop_service_managed_repo_runtime() -> Result<()> {
    let runtime = read_runtime_state(Path::new("."))?;
    let binding = read_service_metadata(Path::new("."))?;

    match runtime {
        Some(state)
            if state.mode == DaemonMode::Service
                && state.service_name.as_deref() == Some(GLOBAL_SUPERVISOR_SERVICE_NAME) =>
        {
            terminate_process(state.pid)?;
            wait_for_runtime_cleanup(&runtime_state_path(Path::new(".")), STOP_TIMEOUT)?;
            Ok(())
        }
        Some(state) => bail!(
            "Bitloops daemon is running in {} mode. Use `bitloops daemon stop` from that mode instead.",
            state.mode
        ),
        None if binding.is_some() => Ok(()),
        None => bail!("Bitloops daemon is not running. Start it with `bitloops daemon start`."),
    }
}

pub(super) async fn restart_service_managed_repo_runtime(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
) -> Result<DaemonRuntimeState> {
    if read_runtime_state(daemon_config.config_root.as_path())?.is_some() {
        stop_service_managed_repo_runtime()?;
    }
    ensure_service_managed_repo_runtime(daemon_config, config).await
}

pub(super) fn install_or_update_repo_service_binding(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
) -> Result<DaemonServiceMetadata> {
    let existing = read_service_metadata(daemon_config.config_root.as_path())?;
    let metadata = DaemonServiceMetadata {
        version: 1,
        config_path: daemon_config.config_path.clone(),
        config_root: daemon_config.config_root.clone(),
        manager: current_service_manager(),
        service_name: GLOBAL_SUPERVISOR_SERVICE_NAME.to_string(),
        service_file: None,
        config,
        last_url: existing.as_ref().and_then(|value| value.last_url.clone()),
        last_pid: existing.as_ref().and_then(|value| value.last_pid),
    };
    write_service_metadata(
        &service_metadata_path(daemon_config.config_root.as_path()),
        &metadata,
    )?;
    Ok(metadata)
}

pub(super) fn ensure_can_start(repo_root: &Path, allow_stopped_service: bool) -> Result<()> {
    if let Some(runtime) = read_runtime_state(repo_root)? {
        let current = current_binary_fingerprint().unwrap_or_default();
        if !runtime.binary_fingerprint.is_empty()
            && !current.is_empty()
            && runtime.binary_fingerprint != current
            && !matches!(runtime.mode, DaemonMode::Service)
        {
            terminate_process(runtime.pid)?;
            let _ = fs::remove_file(runtime_state_path(repo_root));
        } else {
            bail!(
                "Bitloops daemon is already running for this repository at {}. Use `bitloops daemon restart` if you need to replace it.",
                runtime.url
            );
        }
    }

    if !allow_stopped_service
        && let Some(metadata) = read_service_metadata(repo_root)?
        && metadata.service_name == GLOBAL_SUPERVISOR_SERVICE_NAME
        && supervisor_available().unwrap_or(false)
    {
        bail!(
            "Bitloops daemon is already running as an always-on service ({}) for this repository.",
            metadata.service_name
        );
    }

    Ok(())
}
