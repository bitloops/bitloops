use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

use anyhow::{Context, Result, bail};
use clap::{ArgGroup, Args};

const CONFIGURATION_ROUTE: &str = "/settings/configuration";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigureWebStartMode {
    ReuseExistingDaemon,
    StartService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigureDaemonStartAction {
    Restart,
    StartDetached,
}

#[cfg(test)]
type ConfigureDaemonStartHook = dyn Fn(
        ConfigureDaemonStartAction,
        &crate::daemon::ResolvedDaemonConfig,
    ) -> Result<crate::daemon::DaemonRuntimeState>
    + 'static;

#[cfg(test)]
thread_local! {
    static CONFIGURE_DAEMON_START_HOOK: RefCell<Option<Rc<ConfigureDaemonStartHook>>> =
        RefCell::new(None);
}

#[derive(Args, Debug, Clone)]
#[command(group(
    ArgGroup::new("mode")
        .required(true)
        .multiple(false)
        .args(["web", "file", "default_config"])
))]
pub struct ConfigureArgs {
    /// Open the daemon configuration dashboard.
    #[arg(long, default_value_t = false)]
    pub web: bool,

    /// Install a complete daemon config.toml file.
    #[arg(long, value_name = "PATH")]
    pub file: Option<PathBuf>,

    /// Install the default daemon config.toml file.
    #[arg(long = "default-config", default_value_t = false)]
    pub default_config: bool,

    /// Install the daemon config without starting or restarting the daemon.
    #[arg(long = "no-start", default_value_t = false, conflicts_with = "web")]
    pub no_start: bool,
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

    if args.default_config {
        return run_default_config(args.no_start, out).await;
    }

    let Some(path) = args.file.as_deref() else {
        bail!("missing configure mode; pass `--web`, `--file <path>`, or `--default-config`");
    };
    run_file(path, args.no_start, out).await
}

async fn run_web(out: &mut dyn Write) -> Result<()> {
    log::info!("cli configure web: bootstrapping default daemon and opening dashboard");
    let config_path = crate::config::bootstrap_default_daemon_environment()?;
    let daemon_config = crate::daemon::resolve_daemon_config(Some(config_path.as_path()))?;
    let existing_url = crate::daemon::daemon_url()?;
    let url = match configure_web_start_mode(existing_url.as_deref()) {
        ConfigureWebStartMode::ReuseExistingDaemon => {
            configuration_dashboard_url(existing_url.as_deref().expect("existing daemon url"))
        }
        ConfigureWebStartMode::StartService => {
            let state = crate::daemon::start_service(
                &daemon_config,
                default_dashboard_server_config(),
                None,
            )
            .await?;
            configuration_dashboard_url(&state.url)
        }
    };

    crate::api::open_in_default_browser(&url)?;
    writeln!(out, "Opened Bitloops daemon configuration at {url}")?;
    Ok(())
}

fn configure_web_start_mode(existing_daemon_url: Option<&str>) -> ConfigureWebStartMode {
    if existing_daemon_url.is_some() {
        ConfigureWebStartMode::ReuseExistingDaemon
    } else {
        ConfigureWebStartMode::StartService
    }
}

async fn run_default_config(no_start: bool, out: &mut dyn Write) -> Result<()> {
    log::info!("cli configure default-config: installing generated default daemon config");
    let runtime = crate::daemon::runtime_state()?;
    if runtime
        .as_ref()
        .is_some_and(|runtime| runtime.mode == crate::daemon::DaemonMode::Foreground)
    {
        bail!(
            "cannot apply daemon config while a foreground daemon is running; stop it and rerun `bitloops configure --default-config`"
        );
    }
    let service = if no_start {
        None
    } else {
        crate::daemon::service_metadata()?
    };

    let target = crate::config::default_daemon_config_path()?;
    let raw = crate::config::default_daemon_config_toml()?;
    crate::config::validate_daemon_config_text(&raw, &target)
        .with_context(|| format!("validating generated daemon config {}", target.display()))?;

    install_config_atomically(&target, raw.as_bytes())?;
    let daemon_config = crate::daemon::resolve_daemon_config(Some(target.as_path()))?;
    if no_start {
        writeln!(
            out,
            "Installed Bitloops default daemon config at {}",
            target.display()
        )?;
        writeln!(
            out,
            "Bitloops daemon was not started or restarted (--no-start)"
        )?;
        return Ok(());
    }

    let state =
        start_configured_daemon_after_install(&daemon_config, runtime.as_ref(), service.as_ref())
            .await?;

    writeln!(
        out,
        "Installed Bitloops default daemon config at {}",
        target.display()
    )?;
    writeln!(out, "Bitloops daemon is running at {}", state.url)?;
    Ok(())
}

async fn run_file(path: &Path, no_start: bool, out: &mut dyn Write) -> Result<()> {
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
    let service = if no_start {
        None
    } else {
        crate::daemon::service_metadata()?
    };

    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading daemon config {}", path.display()))?;
    crate::config::validate_daemon_config_text(&raw, path)
        .with_context(|| format!("validating daemon config {}", path.display()))?;

    let target = crate::config::default_daemon_config_path()?;
    install_config_atomically(&target, raw.as_bytes())?;

    let daemon_config = crate::daemon::resolve_daemon_config(Some(target.as_path()))?;
    if no_start {
        writeln!(
            out,
            "Installed Bitloops daemon config at {}",
            target.display()
        )?;
        writeln!(
            out,
            "Bitloops daemon was not started or restarted (--no-start)"
        )?;
        return Ok(());
    }

    let state =
        start_configured_daemon_after_install(&daemon_config, runtime.as_ref(), service.as_ref())
            .await?;

    writeln!(
        out,
        "Installed Bitloops daemon config at {}",
        target.display()
    )?;
    writeln!(out, "Bitloops daemon is running at {}", state.url)?;
    Ok(())
}

async fn start_configured_daemon_after_install(
    daemon_config: &crate::daemon::ResolvedDaemonConfig,
    runtime: Option<&crate::daemon::DaemonRuntimeState>,
    service: Option<&crate::daemon::DaemonServiceMetadata>,
) -> Result<crate::daemon::DaemonRuntimeState> {
    let action = configure_daemon_start_action(runtime, service);

    #[cfg(test)]
    if let Some(result) = maybe_run_configure_daemon_start_hook(action, daemon_config) {
        return result;
    }

    match action {
        ConfigureDaemonStartAction::Restart => crate::daemon::restart(Some(daemon_config)).await,
        ConfigureDaemonStartAction::StartDetached => {
            crate::daemon::start_detached(daemon_config, default_dashboard_server_config(), None)
                .await
        }
    }
}

fn configure_daemon_start_action(
    runtime: Option<&crate::daemon::DaemonRuntimeState>,
    service: Option<&crate::daemon::DaemonServiceMetadata>,
) -> ConfigureDaemonStartAction {
    if runtime.is_some() || service.is_some() {
        ConfigureDaemonStartAction::Restart
    } else {
        ConfigureDaemonStartAction::StartDetached
    }
}

#[cfg(test)]
fn maybe_run_configure_daemon_start_hook(
    action: ConfigureDaemonStartAction,
    daemon_config: &crate::daemon::ResolvedDaemonConfig,
) -> Option<Result<crate::daemon::DaemonRuntimeState>> {
    CONFIGURE_DAEMON_START_HOOK.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|hook| hook(action, daemon_config))
    })
}

#[cfg(test)]
fn with_configure_daemon_start_hook<T>(
    hook: impl Fn(
        ConfigureDaemonStartAction,
        &crate::daemon::ResolvedDaemonConfig,
    ) -> Result<crate::daemon::DaemonRuntimeState>
    + 'static,
    f: impl FnOnce() -> T,
) -> T {
    CONFIGURE_DAEMON_START_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "configure daemon start hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });
    let result = f();
    CONFIGURE_DAEMON_START_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
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
    use crate::utils::platform_dirs::{
        TestPlatformDirOverrides, bitloops_data_dir, with_test_platform_dir_overrides,
    };
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

    fn fake_daemon_state(
        daemon_config: &crate::daemon::ResolvedDaemonConfig,
        mode: crate::daemon::DaemonMode,
    ) -> crate::daemon::DaemonRuntimeState {
        crate::daemon::DaemonRuntimeState {
            version: 1,
            config_path: daemon_config.config_path.clone(),
            config_root: daemon_config.config_root.clone(),
            pid: std::process::id(),
            mode,
            service_name: None,
            url: "http://127.0.0.1:5667".to_string(),
            host: "127.0.0.1".to_string(),
            port: 5667,
            bundle_dir: daemon_config.config_root.join("bundle"),
            relational_db_path: daemon_config.relational_db_path.clone(),
            events_db_path: daemon_config.events_db_path.clone(),
            blob_store_path: daemon_config.blob_store_path.clone(),
            repo_registry_path: daemon_config.repo_registry_path.clone(),
            binary_fingerprint: crate::daemon::current_binary_fingerprint().unwrap_or_default(),
            updated_at_unix: 0,
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
    fn configure_web_start_mode_uses_always_on_service_for_fresh_daemon() {
        assert_eq!(
            configure_web_start_mode(None),
            ConfigureWebStartMode::StartService
        );
    }

    #[test]
    fn configure_daemon_start_action_restarts_existing_daemon_and_starts_fresh_detached() {
        let temp = TempDir::new().expect("temp dir");
        let daemon_config = crate::daemon::ResolvedDaemonConfig {
            config_path: temp.path().join("config.toml"),
            config_root: temp.path().to_path_buf(),
            relational_db_path: temp.path().join("relational.db"),
            events_db_path: temp.path().join("events.duckdb"),
            blob_store_path: temp.path().join("blob"),
            repo_registry_path: temp.path().join("repo-registry.json"),
        };
        let runtime = fake_daemon_state(&daemon_config, crate::daemon::DaemonMode::Detached);

        assert_eq!(
            configure_daemon_start_action(Some(&runtime), None),
            ConfigureDaemonStartAction::Restart
        );
        assert_eq!(
            configure_daemon_start_action(None, None),
            ConfigureDaemonStartAction::StartDetached
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
                        default_config: false,
                        no_start: false,
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

    #[test]
    fn configure_default_config_writes_expected_daemon_config() {
        let temp = TempDir::new().expect("temp dir");

        with_process_state(None, &[], || {
            with_test_platform_dir_overrides(app_dir_overrides(&temp), || {
                let start_actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::<
                    ConfigureDaemonStartAction,
                >::new(
                )));
                let start_actions_for_hook = std::rc::Rc::clone(&start_actions);
                let mut out = Vec::new();
                let result = with_configure_daemon_start_hook(
                    move |action, daemon_config| {
                        start_actions_for_hook.borrow_mut().push(action);
                        Ok(fake_daemon_state(
                            daemon_config,
                            crate::daemon::DaemonMode::Detached,
                        ))
                    },
                    || {
                        runtime().block_on(run_with_io(
                            ConfigureArgs {
                                web: false,
                                file: None,
                                default_config: true,
                                no_start: false,
                            },
                            &mut out,
                        ))
                    },
                );

                result.expect("configure --default-config should write default config");
                let output = String::from_utf8(out).expect("output should be utf-8");
                let default_path =
                    crate::config::default_daemon_config_path().expect("default config path");
                assert!(
                    output.contains(default_path.to_string_lossy().as_ref()),
                    "output should mention installed config path"
                );
                assert!(
                    output.contains("Bitloops daemon is running at http://127.0.0.1:5667"),
                    "output should mention started daemon"
                );
                assert_eq!(
                    start_actions.borrow().as_slice(),
                    &[ConfigureDaemonStartAction::StartDetached]
                );

                let content =
                    fs::read_to_string(&default_path).expect("default config should exist");
                crate::config::validate_daemon_config_text(&content, &default_path)
                    .expect("default daemon config should validate");

                let default_root = Path::new(".");
                for path in [
                    crate::utils::paths::default_relational_db_path(default_root),
                    crate::utils::paths::default_events_db_path(default_root),
                    crate::utils::paths::default_blob_store_path(default_root),
                ] {
                    assert!(
                        content.contains(path.to_string_lossy().as_ref()),
                        "default config should contain dynamic path {}",
                        path.display()
                    );
                }

                let data_dir = bitloops_data_dir().expect("data dir");
                let platform_embeddings_binary_name = if cfg!(windows) {
                    "bitloops-platform-embeddings.exe"
                } else {
                    "bitloops-platform-embeddings"
                };
                let bitloops_inference_binary_name = if cfg!(windows) {
                    "bitloops-inference.exe"
                } else {
                    "bitloops-inference"
                };
                for path in [
                    data_dir
                        .join("tools")
                        .join("bitloops-platform-embeddings")
                        .join(platform_embeddings_binary_name),
                    data_dir
                        .join("tools")
                        .join("bitloops-inference")
                        .join(bitloops_inference_binary_name),
                ] {
                    assert!(
                        content.contains(path.to_string_lossy().as_ref()),
                        "default config should contain managed tool path {}",
                        path.display()
                    );
                }

                for expected in [
                    "[runtime]",
                    "local_dev = false",
                    concat!("cli_version = \"", env!("CARGO_PKG_VERSION"), "\""),
                    "[telemetry]",
                    "enabled = true",
                    "[inference.runtimes.bitloops_platform_embeddings]",
                    "args = [\"--api-key-env\", \"BITLOOPS_PLATFORM_GATEWAY_TOKEN\"]",
                    "[inference.runtimes.bitloops_inference]",
                    "[inference.profiles.platform_code]",
                    "driver = \"bitloops_embeddings_ipc\"",
                    "model = \"bge-m3\"",
                    "[inference.profiles.guidance_llm]",
                    "model = \"ministral-3-3b-instruct\"",
                    "api_key = \"${BITLOOPS_PLATFORM_GATEWAY_TOKEN}\"",
                    "max_output_tokens = 4096",
                    "[inference.profiles.summary_llm]",
                    "max_output_tokens = 200",
                    "[context_guidance.inference]",
                    "guidance_generation = \"guidance_llm\"",
                    "[semantic_clones]",
                    "summary_mode = \"auto\"",
                    "[semantic_clones.inference]",
                    "summary_generation = \"summary_llm\"",
                    "[inference.profiles.architecture_fact_synthesis_codex]",
                    "task = \"structured_generation\"",
                    "runtime = \"codex\"",
                    "driver = \"codex_exec\"",
                    "thinking_level = \"low\"",
                    "[inference.profiles.architecture_role_adjudication_codex]",
                    "max_output_tokens = 1024",
                    "[architecture.inference]",
                    "fact_synthesis = \"architecture_fact_synthesis_codex\"",
                    "role_adjudication = \"architecture_role_adjudication_codex\"",
                    "[inference.runtimes.codex]",
                    "--ask-for-approval",
                    "never",
                    "request_timeout_secs = 600",
                ] {
                    assert!(
                        content.contains(expected),
                        "default config should contain {expected:?}"
                    );
                }
            })
        });
    }

    #[test]
    fn configure_default_config_no_start_writes_config_without_starting_daemon() {
        let temp = TempDir::new().expect("temp dir");

        with_process_state(None, &[], || {
            with_test_platform_dir_overrides(app_dir_overrides(&temp), || {
                let start_actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::<
                    ConfigureDaemonStartAction,
                >::new(
                )));
                let start_actions_for_hook = std::rc::Rc::clone(&start_actions);
                let mut out = Vec::new();
                let result = with_configure_daemon_start_hook(
                    move |action, daemon_config| {
                        start_actions_for_hook.borrow_mut().push(action);
                        Ok(fake_daemon_state(
                            daemon_config,
                            crate::daemon::DaemonMode::Detached,
                        ))
                    },
                    || {
                        runtime().block_on(run_with_io(
                            ConfigureArgs {
                                web: false,
                                file: None,
                                default_config: true,
                                no_start: true,
                            },
                            &mut out,
                        ))
                    },
                );

                result.expect("configure --default-config --no-start should write default config");
                let output = String::from_utf8(out).expect("output should be utf-8");
                let default_path =
                    crate::config::default_daemon_config_path().expect("default config path");
                assert!(
                    output.contains(default_path.to_string_lossy().as_ref()),
                    "output should mention installed config path"
                );
                assert!(
                    !output.contains("Bitloops daemon is running"),
                    "output should not claim the daemon is running"
                );
                assert!(
                    start_actions.borrow().is_empty(),
                    "configure --no-start should not start or restart the daemon"
                );
                assert!(default_path.exists(), "default config should exist");
            })
        });
    }

    #[test]
    fn configure_file_no_start_writes_config_without_starting_daemon() {
        let temp = TempDir::new().expect("temp dir");

        with_process_state(None, &[], || {
            with_test_platform_dir_overrides(app_dir_overrides(&temp), || {
                let source = temp.path().join("source-config.toml");
                let raw = crate::config::default_daemon_config_toml()
                    .expect("default daemon config should render");
                fs::write(&source, raw).expect("write source config");

                let start_actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::<
                    ConfigureDaemonStartAction,
                >::new(
                )));
                let start_actions_for_hook = std::rc::Rc::clone(&start_actions);
                let mut out = Vec::new();
                let result = with_configure_daemon_start_hook(
                    move |action, daemon_config| {
                        start_actions_for_hook.borrow_mut().push(action);
                        Ok(fake_daemon_state(
                            daemon_config,
                            crate::daemon::DaemonMode::Detached,
                        ))
                    },
                    || {
                        runtime().block_on(run_with_io(
                            ConfigureArgs {
                                web: false,
                                file: Some(source.clone()),
                                default_config: false,
                                no_start: true,
                            },
                            &mut out,
                        ))
                    },
                );

                result.expect("configure --file --no-start should write config");
                let output = String::from_utf8(out).expect("output should be utf-8");
                let default_path =
                    crate::config::default_daemon_config_path().expect("default config path");
                assert!(
                    output.contains(default_path.to_string_lossy().as_ref()),
                    "output should mention installed config path"
                );
                assert!(
                    !output.contains("Bitloops daemon is running"),
                    "output should not claim the daemon is running"
                );
                assert!(
                    start_actions.borrow().is_empty(),
                    "configure --no-start should not start or restart the daemon"
                );
                assert!(default_path.exists(), "installed config should exist");
            })
        });
    }
}
