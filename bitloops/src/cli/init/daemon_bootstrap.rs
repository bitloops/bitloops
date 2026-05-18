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

#[cfg(not(test))]
fn daemon_server_config_from_status(
    runtime: Option<&crate::daemon::DaemonRuntimeState>,
) -> crate::api::DashboardServerConfig {
    runtime.map_or_else(default_daemon_server_config, |runtime| {
        crate::api::DashboardServerConfig {
            host: Some(runtime.host.clone()),
            port: runtime.port,
            no_open: true,
            force_http: runtime.url.starts_with("http://"),
            recheck_local_dashboard_net: false,
            bundle_dir: Some(runtime.bundle_dir.clone()),
        }
    })
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
    if crate::daemon::runtime_state()?.is_some() {
        return Ok(());
    }

    let config_path = bootstrap_default_daemon_environment()?;
    let daemon_config = crate::daemon::resolve_daemon_config(Some(config_path.as_path()))?;
    let _ =
        crate::daemon::start_detached(&daemon_config, default_daemon_server_config(), telemetry)
            .await?;
    Ok(())
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

        let config = daemon_server_config_from_status(runtime.as_ref());
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
