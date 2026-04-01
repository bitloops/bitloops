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
        cli::Commands::DaemonSupervisor(_) => Some(daemon::ProcessLogContext::supervisor()),
        cli::Commands::Start(args) if !args.detached && !args.until_stopped => Some(
            daemon::ProcessLogContext::daemon("foreground", args.config.clone(), None),
        ),
        cli::Commands::Restart(args) => Some(daemon::ProcessLogContext::daemon(
            "foreground",
            args.config.clone(),
            None,
        )),
        cli::Commands::Daemon(args) => match args.command.as_ref()? {
            cli::daemon::DaemonCommand::Start(start) if !start.detached && !start.until_stopped => {
                Some(daemon::ProcessLogContext::daemon(
                    "foreground",
                    start.config.clone(),
                    None,
                ))
            }
            cli::daemon::DaemonCommand::Restart(restart) => Some(
                daemon::ProcessLogContext::daemon("foreground", restart.config.clone(), None),
            ),
            _ => None,
        },
        _ => None,
    }
}

#[tokio::main]
async fn main() {
    let cmd = cli::Cli::parse();
    if let Some(context) = daemon_log_context(cmd.command.as_ref()) {
        if let Err(err) = daemon::init_process_logger(context) {
            eprintln!("[bitloops] Warning: failed to initialize daemon logger: {err:#}");
            init_standard_logger();
        }
    } else {
        init_standard_logger();
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
    use crate::test_support::process_state::enter_process_state;
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

        let xdg_config_home = temp.path().join(".config").display().to_string();
        let app_data = temp
            .path()
            .join("AppData")
            .join("Roaming")
            .display()
            .to_string();
        let home = temp.path().display().to_string();
        let env_vars = vec![
            ("HOME", Some(home.as_str())),
            ("XDG_CONFIG_HOME", Some(xdg_config_home.as_str())),
            ("APPDATA", Some(app_data.as_str())),
        ];
        let _guard = enter_process_state(Some(temp.path()), &env_vars);
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

        let xdg_config_home = temp.path().join(".config").display().to_string();
        let app_data = temp
            .path()
            .join("AppData")
            .join("Roaming")
            .display()
            .to_string();
        let home = temp.path().display().to_string();
        let env_vars = vec![
            ("HOME", Some(home.as_str())),
            ("XDG_CONFIG_HOME", Some(xdg_config_home.as_str())),
            ("APPDATA", Some(app_data.as_str())),
        ];
        let _guard = enter_process_state(Some(temp.path()), &env_vars);

        assert_eq!(
            resolve_dashboard_config().local_dashboard,
            Some(DashboardLocalDashboardConfig { tls: Some(true) })
        );
    }

    #[test]
    fn main_target_provider_config_defaults_without_repo_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        let _guard = enter_process_state(Some(temp.path()), &[]);

        let cfg = resolve_provider_config().expect("provider config");

        assert_eq!(cfg, ProviderConfig::default());
    }
}
