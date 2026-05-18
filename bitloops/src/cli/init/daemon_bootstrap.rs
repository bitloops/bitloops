use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

use crate::config::bootstrap_default_daemon_environment;

#[cfg(test)]
type InstallDefaultDaemonHook = dyn Fn(bool) -> Result<()> + 'static;

#[cfg(test)]
type EnableDefaultDaemonServiceHook = dyn Fn(bool) -> Result<()> + 'static;

#[cfg(test)]
thread_local! {
    static INSTALL_DEFAULT_DAEMON_HOOK: RefCell<Option<Rc<InstallDefaultDaemonHook>>> =
        RefCell::new(None);
    static ENABLE_DEFAULT_DAEMON_SERVICE_HOOK: RefCell<Option<Rc<EnableDefaultDaemonServiceHook>>> =
        RefCell::new(None);
}

fn default_daemon_server_config() -> crate::api::DashboardServerConfig {
    crate::api::DashboardServerConfig {
        host: None,
        port: crate::api::DEFAULT_DASHBOARD_PORT,
        no_open: true,
        force_http: false,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefaultDaemonBootstrapMode {
    AlreadyRunning,
    ServiceManaged,
    Detached,
}

fn choose_default_daemon_bootstrap_mode(
    runtime: Option<&crate::daemon::DaemonRuntimeState>,
    service: Option<&crate::daemon::DaemonServiceMetadata>,
) -> DefaultDaemonBootstrapMode {
    if runtime.is_some() {
        DefaultDaemonBootstrapMode::AlreadyRunning
    } else if service.is_some() {
        DefaultDaemonBootstrapMode::ServiceManaged
    } else {
        DefaultDaemonBootstrapMode::Detached
    }
}

fn daemon_server_config_from_status(
    runtime: Option<&crate::daemon::DaemonRuntimeState>,
    service: Option<&crate::daemon::DaemonServiceMetadata>,
) -> crate::api::DashboardServerConfig {
    if let Some(runtime) = runtime {
        return crate::api::DashboardServerConfig {
            host: Some(runtime.host.clone()),
            port: runtime.port,
            no_open: true,
            force_http: runtime.url.starts_with("http://"),
            recheck_local_dashboard_net: false,
            bundle_dir: Some(runtime.bundle_dir.clone()),
        };
    }

    service
        .map(|metadata| metadata.config.clone())
        .unwrap_or_else(default_daemon_server_config)
}

pub(crate) async fn maybe_install_default_daemon(
    install_default_daemon: bool,
    telemetry: Option<bool>,
) -> Result<()> {
    #[cfg(test)]
    if let Some(result) = maybe_run_install_default_daemon_hook(install_default_daemon) {
        return result;
    }

    if !install_default_daemon {
        return Ok(());
    }

    let _guard = DefaultDaemonBootstrapLock::acquire()?;
    let runtime = crate::daemon::runtime_state()?;
    let service = crate::daemon::service_metadata()?;
    let bootstrap_mode = choose_default_daemon_bootstrap_mode(runtime.as_ref(), service.as_ref());

    match bootstrap_mode {
        DefaultDaemonBootstrapMode::AlreadyRunning => Ok(()),
        DefaultDaemonBootstrapMode::ServiceManaged | DefaultDaemonBootstrapMode::Detached => {
            let config_path = bootstrap_default_daemon_environment()?;
            let daemon_config = crate::daemon::resolve_daemon_config(Some(config_path.as_path()))?;
            let config = daemon_server_config_from_status(runtime.as_ref(), service.as_ref());
            match bootstrap_mode {
                DefaultDaemonBootstrapMode::ServiceManaged => {
                    let _ = crate::daemon::start_service(&daemon_config, config, telemetry).await?;
                }
                DefaultDaemonBootstrapMode::Detached => {
                    let _ =
                        crate::daemon::start_detached(&daemon_config, config, telemetry).await?;
                }
                DefaultDaemonBootstrapMode::AlreadyRunning => {}
            }
            Ok(())
        }
    }
}

struct DefaultDaemonBootstrapLock {
    #[allow(dead_code)]
    file: std::fs::File,
    path: PathBuf,
}

impl DefaultDaemonBootstrapLock {
    fn acquire() -> Result<Self> {
        let config_path = crate::config::default_daemon_config_path()?;
        if let Some(parent) = config_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(anyhow::Error::from)?;
        }
        let lock_path = config_path.with_file_name("daemon-bootstrap.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(anyhow::Error::from)?;
        lock_default_daemon_bootstrap_file(&file)?;
        Ok(Self {
            file,
            path: lock_path,
        })
    }
}

impl Drop for DefaultDaemonBootstrapLock {
    fn drop(&mut self) {
        if let Err(err) = unlock_default_daemon_bootstrap_file(&self.file) {
            log::warn!(
                "failed to release default daemon bootstrap lock {}: {err:#}",
                self.path.display()
            );
        }
    }
}

fn lock_default_daemon_bootstrap_file(file: &std::fs::File) -> Result<()> {
    fs2::FileExt::lock_exclusive(file).context("acquiring default daemon bootstrap lock")
}

fn unlock_default_daemon_bootstrap_file(file: &std::fs::File) -> Result<()> {
    fs2::FileExt::unlock(file).context("releasing default daemon bootstrap lock")
}

pub(crate) async fn maybe_enable_default_daemon_service(
    enable_default_daemon_service: bool,
    _daemon_config_path: &Path,
    _telemetry: Option<bool>,
) -> Result<()> {
    if !enable_default_daemon_service {
        return Ok(());
    }

    #[cfg(test)]
    {
        if let Some(result) =
            maybe_run_enable_default_daemon_service_hook(enable_default_daemon_service)
        {
            return result;
        }

        Ok(())
    }

    #[cfg(not(test))]
    {
        let runtime = crate::daemon::runtime_state()?;
        let service = crate::daemon::service_metadata()?;
        let already_service_managed = runtime
            .as_ref()
            .is_some_and(|runtime| runtime.mode == crate::daemon::DaemonMode::Service)
            || service.is_some();
        if already_service_managed {
            return Ok(());
        }

        let config = daemon_server_config_from_status(runtime.as_ref(), service.as_ref());
        if runtime.is_some() {
            crate::daemon::stop().await?;
        }

        let daemon_config = crate::daemon::resolve_daemon_config(Some(_daemon_config_path))?;
        let _ = crate::daemon::start_service(&daemon_config, config, _telemetry).await?;
        Ok(())
    }
}

#[cfg(test)]
fn maybe_run_install_default_daemon_hook(install_default_daemon: bool) -> Option<Result<()>> {
    INSTALL_DEFAULT_DAEMON_HOOK.with(|cell: &RefCell<Option<Rc<InstallDefaultDaemonHook>>>| {
        cell.borrow()
            .as_ref()
            .map(|hook| hook(install_default_daemon))
    })
}

#[cfg(test)]
pub(crate) fn with_install_default_daemon_hook<T>(
    hook: impl Fn(bool) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    INSTALL_DEFAULT_DAEMON_HOOK.with(|cell: &RefCell<Option<Rc<InstallDefaultDaemonHook>>>| {
        assert!(
            cell.borrow().is_none(),
            "install default daemon hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });
    let result = f();
    INSTALL_DEFAULT_DAEMON_HOOK.with(|cell: &RefCell<Option<Rc<InstallDefaultDaemonHook>>>| {
        *cell.borrow_mut() = None;
    });
    result
}

#[cfg(test)]
fn maybe_run_enable_default_daemon_service_hook(
    enable_default_daemon_service: bool,
) -> Option<Result<()>> {
    ENABLE_DEFAULT_DAEMON_SERVICE_HOOK.with(
        |cell: &RefCell<Option<Rc<EnableDefaultDaemonServiceHook>>>| {
            cell.borrow()
                .as_ref()
                .map(|hook| hook(enable_default_daemon_service))
        },
    )
}

#[cfg(test)]
pub(crate) fn with_enable_default_daemon_service_hook<T>(
    hook: impl Fn(bool) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    ENABLE_DEFAULT_DAEMON_SERVICE_HOOK.with(
        |cell: &RefCell<Option<Rc<EnableDefaultDaemonServiceHook>>>| {
            assert!(
                cell.borrow().is_none(),
                "enable default daemon service hook already installed"
            );
            *cell.borrow_mut() = Some(Rc::new(hook));
        },
    );
    let result = f();
    ENABLE_DEFAULT_DAEMON_SERVICE_HOOK.with(
        |cell: &RefCell<Option<Rc<EnableDefaultDaemonServiceHook>>>| {
            *cell.borrow_mut() = None;
        },
    );
    result
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    fn dashboard_config(port: u16, force_http: bool) -> crate::api::DashboardServerConfig {
        crate::api::DashboardServerConfig {
            host: Some("127.0.0.1".to_string()),
            port,
            no_open: true,
            force_http,
            recheck_local_dashboard_net: false,
            bundle_dir: Some(PathBuf::from("/tmp/bitloops-dashboard-service")),
        }
    }

    fn daemon_runtime_state() -> crate::daemon::DaemonRuntimeState {
        crate::daemon::DaemonRuntimeState {
            version: 1,
            config_path: PathBuf::from("/tmp/bitloops/config.toml"),
            config_root: PathBuf::from("/tmp/bitloops"),
            pid: 12345,
            mode: crate::daemon::DaemonMode::Service,
            service_name: Some("com.bitloops.daemon".to_string()),
            url: "http://127.0.0.1:5667".to_string(),
            host: "127.0.0.1".to_string(),
            port: 5667,
            bundle_dir: PathBuf::from("/tmp/bitloops-dashboard-runtime"),
            relational_db_path: PathBuf::from("/tmp/bitloops/relational.db"),
            events_db_path: PathBuf::from("/tmp/bitloops/events.db"),
            blob_store_path: PathBuf::from("/tmp/bitloops/blob-store"),
            repo_registry_path: PathBuf::from("/tmp/bitloops/repos.json"),
            binary_fingerprint: "test-fingerprint".to_string(),
            updated_at_unix: 1,
        }
    }

    fn daemon_service_metadata() -> crate::daemon::DaemonServiceMetadata {
        crate::daemon::DaemonServiceMetadata {
            version: 1,
            config_path: PathBuf::from("/tmp/bitloops/config.toml"),
            config_root: PathBuf::from("/tmp/bitloops"),
            manager: crate::daemon::ServiceManagerKind::Launchd,
            service_name: "com.bitloops.daemon".to_string(),
            service_file: Some(PathBuf::from("/tmp/com.bitloops.daemon.plist")),
            config: dashboard_config(5777, false),
            last_url: Some("http://127.0.0.1:5777".to_string()),
            last_pid: None,
        }
    }

    #[test]
    fn default_daemon_bootstrap_mode_uses_service_when_metadata_exists_without_runtime() {
        assert_eq!(
            super::choose_default_daemon_bootstrap_mode(None, None),
            super::DefaultDaemonBootstrapMode::Detached
        );

        let service = daemon_service_metadata();
        assert_eq!(
            super::choose_default_daemon_bootstrap_mode(None, Some(&service)),
            super::DefaultDaemonBootstrapMode::ServiceManaged
        );

        let runtime = daemon_runtime_state();
        assert_eq!(
            super::choose_default_daemon_bootstrap_mode(Some(&runtime), Some(&service)),
            super::DefaultDaemonBootstrapMode::AlreadyRunning
        );
    }

    #[test]
    fn daemon_server_config_prefers_runtime_then_service_then_defaults() {
        let runtime = daemon_runtime_state();
        let service = daemon_service_metadata();

        let from_runtime = super::daemon_server_config_from_status(Some(&runtime), Some(&service));
        assert_eq!(from_runtime.port, 5667);
        assert_eq!(from_runtime.bundle_dir, Some(runtime.bundle_dir.clone()));
        assert!(from_runtime.force_http);

        let from_service = super::daemon_server_config_from_status(None, Some(&service));
        assert_eq!(from_service.port, 5777);
        assert_eq!(from_service.bundle_dir, service.config.bundle_dir.clone());
        assert!(!from_service.force_http);

        let defaulted = super::daemon_server_config_from_status(None, None);
        assert_eq!(defaulted.port, crate::api::DEFAULT_DASHBOARD_PORT);
        assert!(defaulted.bundle_dir.is_none());
    }

    #[test]
    fn default_daemon_bootstrap_mode_matches_second_repo_stale_service_state() {
        let service = daemon_service_metadata();

        let runtime_absent = None;
        let service_present = Some(&service);

        assert_eq!(
            super::choose_default_daemon_bootstrap_mode(runtime_absent, service_present),
            super::DefaultDaemonBootstrapMode::ServiceManaged,
            "when the supervisor service is installed but no daemon runtime is attached, init bootstrap must use the service path rather than detached start"
        );
    }
}
