use clap::Parser;

pub use bitloops::adapters;
pub use bitloops::api;
pub use bitloops::capability_packs;
pub use bitloops::cli;
pub use bitloops::config;
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

#[tokio::main]
async fn main() {
    // comment to be deleted 1
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cmd = cli::Cli::parse();

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
        ProviderConfig, dashboard_use_bitloops_local, resolve_provider_config,
        resolve_store_backend_config,
    };
    use crate::test_support::process_state::enter_process_state;
    use std::fs;

    fn write_envelope_config(repo_root: &std::path::Path, settings: serde_json::Value) {
        let config_dir = repo_root.join(".bitloops");
        fs::create_dir_all(&config_dir).expect("create config dir");
        fs::write(
            config_dir.join("config.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": "1.0",
                "scope": "project",
                "settings": settings
            }))
            .expect("serialize"),
        )
        .expect("write config");
    }

    #[test]
    fn main_target_resolves_store_backend_config_from_repo_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_envelope_config(
            temp.path(),
            serde_json::json!({
                "stores": {
                    "relational": {
                        "postgres_dsn": "postgres://u:p@localhost:5432/bitloops"
                    }
                }
            }),
        );

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
        write_envelope_config(
            temp.path(),
            serde_json::json!({
                "dashboard": {
                    "use_bitloops_local": true
                }
            }),
        );

        let _guard = enter_process_state(Some(temp.path()), &[]);

        assert!(dashboard_use_bitloops_local());
    }

    #[test]
    fn main_target_provider_config_defaults_without_repo_config() {
        let temp = tempfile::tempdir().expect("temp dir");
        let _guard = enter_process_state(Some(temp.path()), &[]);

        let cfg = resolve_provider_config().expect("provider config");

        assert_eq!(cfg, ProviderConfig::default());
    }
}
