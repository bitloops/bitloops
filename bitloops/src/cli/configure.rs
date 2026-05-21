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
        return run_default_config(out).await;
    }

    let Some(path) = args.file.as_deref() else {
        bail!("missing configure mode; pass `--web`, `--file <path>`, or `--default-config`");
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

async fn run_default_config(out: &mut dyn Write) -> Result<()> {
    log::info!("cli configure default-config: installing generated default daemon config");
    let target = crate::config::default_daemon_config_path()?;
    let raw = crate::config::default_daemon_config_toml()?;
    crate::config::validate_daemon_config_text(&raw, &target)
        .with_context(|| format!("validating generated daemon config {}", target.display()))?;

    install_config_atomically(&target, raw.as_bytes())?;
    crate::config::ensure_daemon_store_artifacts(Some(target.as_path()))?;

    writeln!(
        out,
        "Installed Bitloops default daemon config at {}",
        target.display()
    )?;
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
                        default_config: false,
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
                let mut out = Vec::new();
                let result = runtime().block_on(run_with_io(
                    ConfigureArgs {
                        web: false,
                        file: None,
                        default_config: true,
                    },
                    &mut out,
                ));

                result.expect("configure --default-config should write default config");
                let output = String::from_utf8(out).expect("output should be utf-8");
                let default_path =
                    crate::config::default_daemon_config_path().expect("default config path");
                assert!(
                    output.contains(default_path.to_string_lossy().as_ref()),
                    "output should mention installed config path"
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
}
