use clap::Parser;

pub use bitloops_cli::adapters;
pub use bitloops_cli::app;
pub use bitloops_cli::capability_packs;
pub use bitloops_cli::config;
pub use bitloops_cli::host;
pub use bitloops_cli::git;
pub use bitloops_cli::models;
pub use bitloops_cli::read;
pub use bitloops_cli::repository;
pub use bitloops_cli::storage;
pub use bitloops_cli::telemetry;
pub use bitloops_cli::utils;
mod branding;
mod commands;
mod server;

#[cfg(test)]
pub(crate) mod test_support;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cmd = commands::Cli::parse();

    tokio::select! {
        result = commands::run(cmd) => {
            if let Err(e) = result {
                // SilentError: command already printed the error, skip duplicate output
                if e.downcast_ref::<commands::SilentError>().is_none() {
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
        DashboardFileConfig, ProviderConfig, dashboard_use_bitloops_local, resolve_provider_config,
        resolve_store_backend_config,
    };
    use crate::test_support::process_state::enter_process_state;
    use std::fs;

    #[test]
    fn main_target_resolves_store_backend_config_from_repo_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config_dir = temp.path().join(".bitloops");
        fs::create_dir_all(&config_dir).expect("create config dir");
        fs::write(
            config_dir.join("config.json"),
            serde_json::json!({
                "stores": {
                    "relational": {
                        "provider": "postgres",
                        "postgres_dsn": "postgres://u:p@localhost:5432/bitloops"
                    }
                }
            })
            .to_string(),
        )
        .expect("write repo config");

        let _guard = enter_process_state(Some(temp.path()), &[]);
        let cfg = resolve_store_backend_config().expect("backend config");

        assert_eq!(
            cfg.relational.postgres_dsn.as_deref(),
            Some("postgres://u:p@localhost:5432/bitloops")
        );
    }

    #[test]
    fn main_target_dashboard_use_bitloops_local_reads_repo_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config_dir = temp.path().join(".bitloops");
        fs::create_dir_all(&config_dir).expect("create config dir");
        fs::write(
            config_dir.join("config.json"),
            serde_json::json!({
                "dashboard": {
                    "use_bitloops_local": true
                }
            })
            .to_string(),
        )
        .expect("write repo config");

        let _guard = enter_process_state(Some(temp.path()), &[]);

        assert!(dashboard_use_bitloops_local());
        assert_eq!(DashboardFileConfig::load().use_bitloops_local, Some(true));
    }

    #[test]
    fn main_target_provider_config_defaults_without_repo_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        let _guard = enter_process_state(Some(temp.path()), &[]);

        let cfg = resolve_provider_config().expect("provider config");

        assert_eq!(cfg, ProviderConfig::default());
    }
}
