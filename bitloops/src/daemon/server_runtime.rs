use super::*;

pub(super) struct RunServerOptions<'a> {
    pub mode: DaemonMode,
    pub service_name: Option<String>,
    pub open_browser: bool,
    pub ready_subject: &'a str,
    pub print_banner: bool,
    pub telemetry: Option<bool>,
}

struct DaemonConfigPathOverrideGuard {
    previous: Option<std::ffi::OsString>,
}

impl DaemonConfigPathOverrideGuard {
    fn install(config_path: &Path) -> Self {
        let previous = std::env::var_os(crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE);
        // SAFETY: the daemon process installs this override during startup on a dedicated process
        // path and drops it during shutdown.
        unsafe {
            std::env::set_var(crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE, config_path);
        }
        Self { previous }
    }
}

impl Drop for DaemonConfigPathOverrideGuard {
    fn drop(&mut self) {
        // SAFETY: paired with install() above; the daemon owns this process state for the duration
        // of the server lifetime.
        unsafe {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE, previous);
            } else {
                std::env::remove_var(crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE);
            }
        }
    }
}

pub(super) async fn run_server(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
    options: RunServerOptions<'_>,
) -> Result<()> {
    let _config_override = DaemonConfigPathOverrideGuard::install(&daemon_config.config_path);
    log::debug!(
        "daemon boot: mode={} config={} host={:?} port={}",
        options.mode,
        daemon_config.config_path.display(),
        config.host,
        config.port
    );
    crate::config::update_daemon_telemetry_consent(
        Some(daemon_config.config_path.as_path()),
        crate::cli::telemetry_consent::CURRENT_CLI_VERSION,
        options.telemetry,
    )?;
    crate::telemetry::analytics::start_analytics_spool_worker_once();
    let repo = crate::host::devql::resolve_repo_identity(daemon_config.config_root.as_path())
        .context("resolving repository identity for daemon startup")?;
    let devql_cfg =
        crate::host::devql::DevqlConfig::from_env(daemon_config.config_root.clone(), repo)
            .context("building DevQL config for daemon startup")?;
    let _ = crate::host::devql::ensure_devql_storage_current(&devql_cfg, "Bitloops daemon startup")
        .await?;
    ensure_bound_repo_watchers_for_daemon_startup(daemon_config);
    let _ = crate::daemon::shared_enrichment_coordinator();

    let config_root = daemon_config
        .config_root
        .canonicalize()
        .unwrap_or_else(|_| daemon_config.config_root.clone());
    let service_metadata_path = service_metadata_path(&config_root);
    let current_fingerprint = current_binary_fingerprint()?;
    let on_ready_config = daemon_config.clone();
    let on_shutdown_config = daemon_config.clone();
    let on_ready_service_metadata_path = service_metadata_path.clone();
    let on_ready_service_name = options.service_name.clone();
    let runtime_state_written = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let on_ready_runtime_state_written = std::sync::Arc::clone(&runtime_state_written);
    let ready_hook: DashboardReadyHook = std::sync::Arc::new(move |ready| {
        write_runtime_state(
            &runtime_state_path(&on_ready_config.config_root),
            &DaemonRuntimeState {
                version: 1,
                config_path: on_ready_config.config_path.clone(),
                config_root: on_ready_config.config_root.clone(),
                pid: std::process::id(),
                mode: options.mode,
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
        on_ready_runtime_state_written.store(true, std::sync::atomic::Ordering::SeqCst);

        if matches!(options.mode, DaemonMode::Service)
            && let Ok(Some(mut metadata)) =
                read_service_metadata_for_path(&on_ready_service_metadata_path)
        {
            metadata.last_url = Some(ready.url.clone());
            metadata.last_pid = Some(std::process::id());
            write_service_metadata(&on_ready_service_metadata_path, &metadata)?;
        }

        Ok(())
    });
    let on_shutdown = std::sync::Arc::new(move || {
        stop_bound_repo_watchers_for_daemon_shutdown(&on_shutdown_config);
    });

    let runtime_options = daemon_dashboard_runtime_options(
        &options,
        daemon_config,
        config_root,
        Some(ready_hook),
        Some(on_shutdown),
    );
    let result = api::run_with_options(config, runtime_options).await;

    if runtime_state_written.load(std::sync::atomic::Ordering::SeqCst)
        && let Err(err) = delete_runtime_state()
    {
        log::warn!("failed to clear daemon runtime state on shutdown: {err:#}");
    }

    result
}

fn daemon_dashboard_runtime_options(
    options: &RunServerOptions<'_>,
    daemon_config: &ResolvedDaemonConfig,
    config_root: PathBuf,
    on_ready: Option<DashboardReadyHook>,
    on_shutdown: Option<api::DashboardShutdownHook>,
) -> DashboardRuntimeOptions {
    DashboardRuntimeOptions {
        ready_subject: options.ready_subject.to_string(),
        print_ready_banner: options.print_banner,
        open_browser: options.open_browser,
        bootstrap_devql_schema: false,
        shutdown_message: Some("Bitloops daemon stopped.".to_string()),
        shutdown_delay: Duration::ZERO,
        on_ready,
        on_shutdown,
        config_path: Some(daemon_config.config_path.clone()),
        config_root: Some(config_root),
        repo_registry_path: Some(daemon_config.repo_registry_path.clone()),
    }
}

pub(super) async fn ensure_service_managed_repo_runtime(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
    telemetry: Option<bool>,
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
        telemetry,
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
            wait_for_shutdown_cleanup(
                state.pid,
                &runtime_state_path(Path::new(".")),
                STOP_TIMEOUT,
            )?;
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
    ensure_service_managed_repo_runtime(daemon_config, config, None).await
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
        let current = match current_binary_fingerprint() {
            Ok(fingerprint) => fingerprint,
            Err(err) => {
                log::warn!(
                    "failed to determine current daemon binary fingerprint while checking repo runtime state: {err:#}"
                );
                String::new()
            }
        };
        if !runtime.binary_fingerprint.is_empty()
            && !current.is_empty()
            && runtime.binary_fingerprint != current
            && !matches!(runtime.mode, DaemonMode::Service)
        {
            terminate_process(runtime.pid)?;
            if let Err(err) = delete_runtime_state() {
                log::warn!("failed to clear stale daemon runtime state before restart: {err:#}");
            }
        } else {
            bail!(
                "Bitloops daemon is already running for this repository at {}. Use `bitloops daemon restart` if you need to replace it.",
                runtime.url
            );
        }
    }

    let supervisor_running = match supervisor_available() {
        Ok(running) => running,
        Err(err) => {
            log::warn!(
                "failed to determine daemon supervisor availability while checking repo runtime state: {err:#}"
            );
            false
        }
    };

    if !allow_stopped_service
        && let Some(metadata) = read_service_metadata(repo_root)?
        && metadata.service_name == GLOBAL_SUPERVISOR_SERVICE_NAME
        && supervisor_running
    {
        bail!(
            "Bitloops daemon is already running as an always-on service ({}) for this repository.",
            metadata.service_name
        );
    }

    Ok(())
}

pub(super) fn stop_bound_repo_watchers_for_daemon_shutdown(daemon_config: &ResolvedDaemonConfig) {
    let repo_roots = match bound_repo_roots_for_daemon_config(daemon_config) {
        Ok(repo_roots) => repo_roots,
        Err(err) => {
            log::warn!(
                "failed to resolve bound repo watchers for daemon shutdown (config={}): {err:#}",
                daemon_config.config_path.display()
            );
            return;
        }
    };

    for repo_root in repo_roots {
        if let Err(err) =
            crate::host::devql::watch::stop_watcher(&repo_root, &daemon_config.config_root)
        {
            log::warn!(
                "failed to stop DevQL watcher during daemon shutdown for repo {}: {err:#}",
                repo_root.display()
            );
        }
    }
}

pub(super) fn ensure_bound_repo_watchers_for_daemon_startup(daemon_config: &ResolvedDaemonConfig) {
    ensure_bound_repo_watchers_for_daemon_startup_with(
        daemon_config,
        crate::host::devql::watch::ensure_watcher_running,
    );
}

pub(super) fn ensure_bound_repo_watchers_for_daemon_startup_with<F>(
    daemon_config: &ResolvedDaemonConfig,
    mut ensure_watcher_running: F,
) where
    F: FnMut(&Path, &Path) -> Result<()>,
{
    let repo_roots = match bound_repo_roots_for_daemon_config(daemon_config) {
        Ok(repo_roots) => repo_roots,
        Err(err) => {
            log::warn!(
                "failed to resolve bound repo watchers for daemon startup (config={}): {err:#}",
                daemon_config.config_path.display()
            );
            return;
        }
    };

    for repo_root in repo_roots {
        if !crate::config::settings::is_enabled_for_hooks(&repo_root) {
            continue;
        }
        match crate::config::settings::devql_sync_enabled(&repo_root) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(err) => {
                log::warn!(
                    "failed to inspect DevQL sync settings during daemon startup for repo {}: {err:#}",
                    repo_root.display()
                );
                continue;
            }
        }
        if let Err(err) = ensure_watcher_running(&repo_root, &daemon_config.config_root) {
            log::warn!(
                "failed to start DevQL watcher during daemon startup for repo {}: {err:#}",
                repo_root.display()
            );
        }
    }
}

fn bound_repo_roots_for_daemon_config(
    daemon_config: &ResolvedDaemonConfig,
) -> Result<Vec<PathBuf>> {
    let current_binding = crate::devql_transport::daemon_binding_identifier_for_config_path(
        &daemon_config.config_path,
    );
    let mut repo_roots = std::collections::BTreeSet::new();

    if let Some(repo_root) = repo_root_for_current_working_tree(&daemon_config.config_root)?
        && repo_has_local_daemon_binding(&repo_root, &current_binding)
    {
        repo_roots.insert(repo_root);
    }

    let registry =
        crate::devql_transport::load_repo_path_registry(&daemon_config.repo_registry_path)
            .with_context(|| {
                format!(
                    "loading repo path registry {} while resolving bound daemon repos",
                    daemon_config.repo_registry_path.display()
                )
            })?;

    for entry in registry.entries {
        let repo_root = entry
            .repo_root
            .canonicalize()
            .unwrap_or_else(|_| entry.repo_root.clone());
        if repo_has_local_daemon_binding(&repo_root, &current_binding) {
            repo_roots.insert(repo_root);
        }
    }

    Ok(repo_roots.into_iter().collect())
}

fn repo_has_local_daemon_binding(repo_root: &Path, current_binding: &str) -> bool {
    let policy = match crate::config::discover_repo_policy_optional(repo_root) {
        Ok(policy) => policy,
        Err(err) => {
            log::warn!(
                "failed to read repo daemon binding for {} while resolving bound daemon repos: {err:#}",
                repo_root.display()
            );
            return false;
        }
    };
    let Some(config_path) = policy.daemon_config_path.as_deref() else {
        return false;
    };
    crate::devql_transport::daemon_binding_identifier_for_config_path(config_path)
        == current_binding
}

fn repo_root_for_current_working_tree(cwd: &Path) -> Result<Option<PathBuf>> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .with_context(|| format!("resolving git repo root from {}", cwd.display()))?;
    if !output.status.success() {
        return Ok(None);
    }

    let repo_root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if repo_root.is_empty() {
        return Ok(None);
    }

    let repo_root = PathBuf::from(repo_root);
    Ok(Some(repo_root.canonicalize().unwrap_or(repo_root)))
}

#[cfg(test)]
mod tests {
    use super::{DashboardRuntimeOptions, RunServerOptions, daemon_dashboard_runtime_options};
    use crate::daemon::{DaemonMode, ResolvedDaemonConfig};
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn daemon_runtime_options_skip_shutdown_delay() {
        let options = daemon_dashboard_runtime_options(
            &RunServerOptions {
                mode: DaemonMode::Detached,
                service_name: None,
                open_browser: false,
                ready_subject: "Bitloops daemon",
                print_banner: false,
                telemetry: None,
            },
            &ResolvedDaemonConfig {
                config_path: PathBuf::from("/tmp/.bitloops.toml"),
                config_root: PathBuf::from("/tmp"),
                relational_db_path: PathBuf::from("/tmp/stores/daemon.sqlite"),
                events_db_path: PathBuf::from("/tmp/stores/daemon.duckdb"),
                blob_store_path: PathBuf::from("/tmp/blob-store"),
                repo_registry_path: PathBuf::from("/tmp/repo-registry.json"),
            },
            PathBuf::from("/tmp"),
            None,
            None,
        );

        assert_eq!(options.shutdown_delay, Duration::ZERO);
        assert_eq!(
            options.shutdown_message.as_deref(),
            Some("Bitloops daemon stopped.")
        );
    }

    #[test]
    fn dashboard_runtime_options_default_shutdown_delay_remains_five_seconds() {
        assert_eq!(
            DashboardRuntimeOptions::default().shutdown_delay,
            Duration::from_secs(5)
        );
    }
}
