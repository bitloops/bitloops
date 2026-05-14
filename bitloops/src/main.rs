use clap::Parser;

pub use bitloops::adapters;
pub use bitloops::api;
pub use bitloops::capability_packs;
pub use bitloops::cli;
pub use bitloops::config;
pub use bitloops::daemon;
pub use bitloops::git;
pub use bitloops::graphql;
pub use bitloops::host;
pub use bitloops::models;
pub use bitloops::storage;
pub use bitloops::telemetry;
pub use bitloops::utils;

#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod test_support;

fn init_standard_logger() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();
}

fn daemon_log_context(command: Option<&cli::Commands>) -> Option<daemon::ProcessLogContext> {
    match command? {
        cli::Commands::DaemonProcess(args) => Some(daemon::ProcessLogContext::daemon(
            match args.mode {
                daemon::DaemonProcessModeArg::Detached => "detached",
                daemon::DaemonProcessModeArg::Service => "service",
            },
            Some(args.config_path.clone()),
            args.service_name.clone(),
        )),
        cli::Commands::DevqlWatcher(args) => Some(daemon::ProcessLogContext::watcher(
            args.daemon_config_root
                .as_deref()
                .map(|config_root| config_root.join(config::BITLOOPS_CONFIG_RELATIVE_PATH))
                .or_else(|| {
                    args.repo_root.as_deref().and_then(|repo_root| {
                        config::resolve_daemon_config_path_for_repo(repo_root).ok()
                    })
                }),
        )),
        cli::Commands::DaemonSupervisor(_) => Some(daemon::ProcessLogContext::supervisor()),
        cli::Commands::CurrentStateWorker(args) => Some(daemon::ProcessLogContext::daemon(
            "current_state_worker",
            Some(args.config_path.clone()),
            None,
        )),
        cli::Commands::Start(args) => Some(daemon::ProcessLogContext::daemon_cli(
            if args.until_stopped {
                "start_service"
            } else if args.detached {
                "start_detached"
            } else {
                "start_foreground"
            },
            args.config.clone(),
        )),
        cli::Commands::Stop(args) => Some(daemon::ProcessLogContext::daemon_cli(
            "stop",
            args.config.clone(),
        )),
        cli::Commands::Restart(args) => Some(daemon::ProcessLogContext::daemon_cli(
            "restart",
            args.config.clone(),
        )),
        cli::Commands::Daemon(args) => match args.command.as_ref()? {
            cli::daemon::DaemonCommand::Start(start) => {
                Some(daemon::ProcessLogContext::daemon_cli(
                    if start.until_stopped {
                        "start_service"
                    } else if start.detached {
                        "start_detached"
                    } else {
                        "start_foreground"
                    },
                    start.config.clone(),
                ))
            }
            cli::daemon::DaemonCommand::Stop(stop) => Some(daemon::ProcessLogContext::daemon_cli(
                "stop",
                stop.config.clone(),
            )),
            cli::daemon::DaemonCommand::Restart(restart) => Some(
                daemon::ProcessLogContext::daemon_cli("restart", restart.config.clone()),
            ),
            _ => None,
        },
        _ => None,
    }
}

fn command_handles_ctrl_c(command: Option<&cli::Commands>) -> bool {
    matches!(
        command,
        Some(cli::Commands::Daemon(cli::daemon::DaemonArgs {
            command: Some(cli::daemon::DaemonCommand::Logs(
                cli::daemon::DaemonLogsArgs { follow: true, .. }
            )),
        }))
    )
}

fn command_requires_persistent_daemon_logging(command: Option<&cli::Commands>) -> bool {
    match command {
        Some(cli::Commands::DevqlWatcher(_))
        | Some(cli::Commands::DaemonProcess(_))
        | Some(cli::Commands::DaemonSupervisor(_))
        | Some(cli::Commands::CurrentStateWorker(_)) => true,
        Some(cli::Commands::Start(args)) => args.detached || args.until_stopped,
        Some(cli::Commands::Daemon(args)) => matches!(
            args.command.as_ref(),
            Some(cli::daemon::DaemonCommand::Start(start)) if start.detached || start.until_stopped
        ),
        _ => false,
    }
}

#[tokio::main]
async fn main() {
    let cmd = cli::Cli::parse();
    if let Some(context) = daemon_log_context(cmd.command.as_ref()) {
        let require_log_file = command_requires_persistent_daemon_logging(cmd.command.as_ref());
        if let Err(err) = daemon::init_process_logger(context, require_log_file) {
            if require_log_file {
                eprintln!("Error: failed to initialize required daemon logger: {err:#}");
                std::process::exit(1);
            }
            eprintln!("[bitloops] Warning: failed to initialize daemon logger: {err:#}");
            init_standard_logger();
        }
    } else {
        init_standard_logger();
    }

    if command_handles_ctrl_c(cmd.command.as_ref()) {
        if let Err(e) = cli::run(cmd).await {
            if e.downcast_ref::<cli::SilentError>().is_none() {
                eprintln!("Error: {e:#}");
            }
            std::process::exit(1);
        }
        return;
    }

    tokio::select! {
        result = cli::run(cmd) => {
            if let Err(e) = result {
                // SilentError: command already printed the error, skip duplicate output
                if e.downcast_ref::<cli::SilentError>().is_none() {
                    eprintln!("Error: {e:#}");
                }
                std::process::exit(1);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            // Interrupted — exit cleanly
        }
    }
}

#[cfg(test)]
mod tests {
    use super::config::{
        DashboardLocalDashboardConfig, ProviderConfig, resolve_dashboard_config,
        resolve_provider_config, resolve_store_backend_config,
    };
    use super::daemon_log_context;
    use crate::cli::{Cli, Commands};
    use crate::test_support::process_state::enter_process_state;
    use clap::Parser;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn daemon_config_root(home_root: &Path) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            home_root.join("Library").join("Application Support")
        }
        #[cfg(target_os = "windows")]
        {
            home_root.join("AppData").join("Roaming")
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            home_root.join(".config")
        }
    }

    fn write_daemon_config(home_root: &Path, toml: &str) {
        let config_path = daemon_config_root(home_root)
            .join("bitloops")
            .join("config.toml");
        let config_dir = config_path.parent().expect("config dir");
        fs::create_dir_all(config_dir).expect("create config dir");
        fs::write(config_path, toml).expect("write config");
    }

    fn enter_isolated_platform_dirs(
        home_root: &Path,
    ) -> crate::test_support::process_state::ProcessStateGuard {
        let xdg_config_home = home_root.join(".config").display().to_string();
        let app_data = home_root
            .join("AppData")
            .join("Roaming")
            .display()
            .to_string();
        let home = home_root.display().to_string();
        let env_vars = vec![
            ("HOME", Some(home.as_str())),
            ("XDG_CONFIG_HOME", Some(xdg_config_home.as_str())),
            ("APPDATA", Some(app_data.as_str())),
        ];
        enter_process_state(Some(home_root), &env_vars)
    }

    #[test]
    fn main_target_resolves_store_backend_config_from_daemon_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_daemon_config(
            temp.path(),
            r#"
[stores.relational]
postgres_dsn = "postgres://u:p@localhost:5432/bitloops"
"#,
        );

        let _guard = enter_isolated_platform_dirs(temp.path());
        let cfg = resolve_store_backend_config().expect("backend config");

        assert_eq!(
            cfg.relational.postgres_dsn.as_deref(),
            Some("postgres://u:p@localhost:5432/bitloops")
        );
    }

    #[test]
    fn main_target_dashboard_local_dashboard_reads_daemon_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_daemon_config(
            temp.path(),
            r#"
[dashboard.local_dashboard]
tls = true
"#,
        );

        let _guard = enter_isolated_platform_dirs(temp.path());

        assert_eq!(
            resolve_dashboard_config().local_dashboard,
            Some(DashboardLocalDashboardConfig { tls: Some(true) })
        );
    }

    #[test]
    fn main_target_provider_config_defaults_without_repo_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        let _guard = enter_isolated_platform_dirs(temp.path());

        let cfg = resolve_provider_config().expect("provider config");

        assert_eq!(cfg, ProviderConfig::default());
    }

    #[test]
    fn main_target_marks_daemon_logs_follow_as_ctrl_c_owner() {
        let parsed = Cli::try_parse_from(["bitloops", "daemon", "logs", "--follow"])
            .expect("daemon logs follow should parse");

        assert!(super::command_handles_ctrl_c(parsed.command.as_ref()));
    }

    #[test]
    fn main_target_keeps_standard_ctrl_c_handling_for_other_commands() {
        let parsed = Cli::try_parse_from(["bitloops", "daemon", "logs", "--path"])
            .expect("daemon logs path should parse");
        assert!(!super::command_handles_ctrl_c(parsed.command.as_ref()));

        let parsed = Cli::try_parse_from(["bitloops", "daemon", "status"])
            .expect("daemon status should parse");
        assert!(!super::command_handles_ctrl_c(parsed.command.as_ref()));
    }

    #[test]
    fn daemon_log_context_uses_daemon_cli_for_stop_command() {
        let parsed = Cli::parse_from(["bitloops", "daemon", "stop"]);
        let Some(context) = daemon_log_context(parsed.command.as_ref()) else {
            panic!("expected daemon log context");
        };
        let command = serde_json::to_value(&context).expect("serialize process log context");
        assert_eq!(command["process"], "daemon_cli");
        assert_eq!(command["mode"], "stop");
    }

    #[test]
    fn daemon_log_context_uses_daemon_cli_for_detached_start_command() {
        let parsed = Cli::parse_from(["bitloops", "daemon", "start", "-d"]);
        let Some(Commands::Daemon(_)) = parsed.command.as_ref() else {
            panic!("expected daemon command");
        };
        let Some(context) = daemon_log_context(parsed.command.as_ref()) else {
            panic!("expected daemon log context");
        };
        let command = serde_json::to_value(&context).expect("serialize process log context");
        assert_eq!(command["process"], "daemon_cli");
        assert_eq!(command["mode"], "start_detached");
    }

    #[test]
    fn daemon_log_context_uses_watcher_process_for_internal_watcher_command() {
        let parsed = Cli::parse_from([
            "bitloops",
            "__devql-watcher",
            "--repo-root",
            "/tmp/repo",
            "--daemon-config-root",
            "/tmp/config-root",
        ]);
        let Some(context) = daemon_log_context(parsed.command.as_ref()) else {
            panic!("expected watcher log context");
        };
        let command = serde_json::to_value(&context).expect("serialize process log context");
        assert_eq!(command["process"], "watcher");
        assert_eq!(command["mode"], "watcher");
        assert_eq!(command["config_path"], "/tmp/config-root/config.toml");
    }

    #[test]
    fn daemon_logger_init_policy_requires_persistent_logging_for_internal_background_commands() {
        let parsed = Cli::try_parse_from([
            "bitloops",
            "__devql-watcher",
            "--repo-root",
            "/tmp/repo",
            "--daemon-config-root",
            "/tmp/config-root",
        ])
        .expect("internal watcher should parse");
        assert!(super::command_requires_persistent_daemon_logging(
            parsed.command.as_ref()
        ));

        let parsed = Cli::try_parse_from([
            "bitloops",
            "__daemon-process",
            "--config-path",
            "/tmp/bitloops.toml",
            "--mode",
            "detached",
        ])
        .expect("internal detached daemon process should parse");
        assert!(super::command_requires_persistent_daemon_logging(
            parsed.command.as_ref()
        ));

        let parsed = Cli::try_parse_from([
            "bitloops",
            "__daemon-process",
            "--config-path",
            "/tmp/bitloops.toml",
            "--mode",
            "service",
        ])
        .expect("internal service daemon process should parse");
        assert!(super::command_requires_persistent_daemon_logging(
            parsed.command.as_ref()
        ));

        let parsed = Cli::try_parse_from(["bitloops", "__daemon-supervisor"])
            .expect("internal daemon supervisor should parse");
        assert!(super::command_requires_persistent_daemon_logging(
            parsed.command.as_ref()
        ));
    }

    #[test]
    fn daemon_logger_init_policy_requires_persistent_logging_for_detached_and_service_starts() {
        let parsed = Cli::try_parse_from(["bitloops", "daemon", "start", "-d"])
            .expect("daemon detached start should parse");
        assert!(super::command_requires_persistent_daemon_logging(
            parsed.command.as_ref()
        ));

        let parsed = Cli::try_parse_from(["bitloops", "daemon", "start", "--until-stopped"])
            .expect("daemon service start should parse");
        assert!(super::command_requires_persistent_daemon_logging(
            parsed.command.as_ref()
        ));

        let parsed = Cli::try_parse_from(["bitloops", "start", "-d"])
            .expect("root detached start should parse");
        assert!(super::command_requires_persistent_daemon_logging(
            parsed.command.as_ref()
        ));

        let parsed = Cli::try_parse_from(["bitloops", "start", "--until-stopped"])
            .expect("root service start should parse");
        assert!(super::command_requires_persistent_daemon_logging(
            parsed.command.as_ref()
        ));
    }

    #[test]
    fn daemon_logger_init_policy_allows_fallback_for_foreground_and_non_daemon_commands() {
        let parsed = Cli::try_parse_from(["bitloops", "daemon", "start"])
            .expect("daemon foreground start should parse");
        assert!(!super::command_requires_persistent_daemon_logging(
            parsed.command.as_ref()
        ));

        let parsed = Cli::try_parse_from(["bitloops", "daemon", "status"])
            .expect("daemon status should parse");
        assert!(!super::command_requires_persistent_daemon_logging(
            parsed.command.as_ref()
        ));

        let parsed =
            Cli::try_parse_from(["bitloops", "version"]).expect("version command should parse");
        assert!(!super::command_requires_persistent_daemon_logging(
            parsed.command.as_ref()
        ));
    }
}
