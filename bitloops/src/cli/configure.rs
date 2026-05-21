use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{ArgGroup, Args};

const CONFIGURATION_ROUTE: &str = "/settings/configuration";

#[derive(Args, Debug, Clone)]
#[command(group(
    ArgGroup::new("mode")
        .required(true)
        .multiple(false)
        .args(["web", "file"])
))]
pub struct ConfigureArgs {
    /// Open the daemon configuration dashboard.
    #[arg(long, default_value_t = false)]
    pub web: bool,

    /// Install a complete daemon config.toml file.
    #[arg(long, value_name = "PATH")]
    pub file: Option<PathBuf>,
}

pub async fn run(args: ConfigureArgs) -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    run_with_io(args, &mut out).await
}

pub(crate) async fn run_with_io(args: ConfigureArgs, out: &mut dyn Write) -> Result<()> {
    if args.web {
        return run_web(out).await;
    }

    let Some(path) = args.file.as_deref() else {
        bail!("missing configure mode; pass `--web` or `--file <path>`");
    };
    run_file(path, out).await
}

async fn run_web(out: &mut dyn Write) -> Result<()> {
    log::info!("cli configure web: bootstrapping default daemon and opening dashboard");
    let config_path = crate::config::bootstrap_default_daemon_environment()?;
    let daemon_config = crate::daemon::resolve_daemon_config(Some(config_path.as_path()))?;
    let url = if let Some(url) = crate::daemon::daemon_url()? {
        configuration_dashboard_url(&url)
    } else {
        let server_config = default_dashboard_server_config();
        let state = if crate::daemon::service_metadata()?.is_some() {
            crate::daemon::start_service(&daemon_config, server_config, None).await?
        } else {
            crate::daemon::start_detached(&daemon_config, server_config, None).await?
        };
        configuration_dashboard_url(&state.url)
    };

    crate::api::open_in_default_browser(&url)?;
    writeln!(out, "Opened Bitloops daemon configuration at {url}")?;
    Ok(())
}

async fn run_file(path: &Path, out: &mut dyn Write) -> Result<()> {
    log::info!("cli configure file: source={}", path.display());
    let runtime = crate::daemon::runtime_state()?;
    if runtime
        .as_ref()
        .is_some_and(|runtime| runtime.mode == crate::daemon::DaemonMode::Foreground)
    {
        bail!(
            "cannot apply daemon config while a foreground daemon is running; stop it and rerun `bitloops configure --file {}`",
            path.display()
        );
    }

    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading daemon config {}", path.display()))?;
    crate::config::validate_daemon_config_text(&raw, path)
        .with_context(|| format!("validating daemon config {}", path.display()))?;

    let target = crate::config::default_daemon_config_path()?;
    install_config_atomically(&target, raw.as_bytes())?;
    crate::config::ensure_daemon_store_artifacts(Some(target.as_path()))?;

    let daemon_config = crate::daemon::resolve_daemon_config(Some(target.as_path()))?;
    let service = crate::daemon::service_metadata()?;
    let state = if runtime.is_some() || service.is_some() {
        crate::daemon::restart(Some(&daemon_config)).await?
    } else {
        crate::daemon::start_detached(&daemon_config, default_dashboard_server_config(), None)
            .await?
    };

    writeln!(
        out,
        "Installed Bitloops daemon config at {}",
        target.display()
    )?;
    writeln!(out, "Bitloops daemon is running at {}", state.url)?;
    Ok(())
}

fn default_dashboard_server_config() -> crate::api::DashboardServerConfig {
    crate::api::DashboardServerConfig {
        host: None,
        port: crate::api::DEFAULT_DASHBOARD_PORT,
        no_open: true,
        force_http: false,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
    }
}

fn configuration_dashboard_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with(CONFIGURATION_ROUTE) {
        trimmed.to_string()
    } else {
        format!("{trimmed}{CONFIGURATION_ROUTE}")
    }
}

fn install_config_atomically(target: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating daemon config directory {}", parent.display()))?;
    }
    let tmp = target.with_extension(format!("toml.tmp-{}", std::process::id()));
    fs::write(&tmp, bytes)
        .with_context(|| format!("writing temporary daemon config {}", tmp.display()))?;
    fs::rename(&tmp, target)
        .with_context(|| format!("installing daemon config {}", target.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::process_state::with_process_state;
    use crate::utils::platform_dirs::{TestPlatformDirOverrides, with_test_platform_dir_overrides};
    use tempfile::TempDir;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    fn app_dir_overrides(temp: &TempDir) -> TestPlatformDirOverrides {
        TestPlatformDirOverrides {
            config_root: Some(temp.path().join("config-root")),
            data_root: Some(temp.path().join("data-root")),
            cache_root: Some(temp.path().join("cache-root")),
            state_root: Some(temp.path().join("state-root")),
        }
    }

    #[test]
    fn configuration_dashboard_url_targets_configuration_route() {
        assert_eq!(
            configuration_dashboard_url("http://127.0.0.1:5667"),
            "http://127.0.0.1:5667/settings/configuration"
        );
        assert_eq!(
            configuration_dashboard_url("http://127.0.0.1:5667/settings/configuration"),
            "http://127.0.0.1:5667/settings/configuration"
        );
    }

    #[test]
    fn configure_file_rejects_invalid_full_daemon_config_before_installing() {
        let temp = TempDir::new().expect("temp dir");
        let invalid = temp.path().join("invalid.toml");
        fs::write(&invalid, "[runtime\nlocal_dev = true\n").expect("write invalid config");

        with_process_state(None, &[], || {
            with_test_platform_dir_overrides(app_dir_overrides(&temp), || {
                let mut out = Vec::new();
                let result = runtime().block_on(run_with_io(
                    ConfigureArgs {
                        web: false,
                        file: Some(invalid.clone()),
                    },
                    &mut out,
                ));

                assert!(result.is_err());
                let default_path =
                    crate::config::default_daemon_config_path().expect("default config path");
                assert!(!default_path.exists());
            })
        });
    }
}
