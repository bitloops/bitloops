#[path = "daemon_config/file.rs"]
mod file;
#[path = "daemon_config/install.rs"]
mod install;
#[path = "daemon_config/plans.rs"]
mod plans;
#[path = "daemon_config/toml.rs"]
mod toml;

#[cfg(test)]
#[path = "daemon_config/tests.rs"]
mod tests;

pub use file::{
    DaemonCliSettings, DaemonTelemetryConsentState, LoadedDaemonSettings,
    bootstrap_default_daemon_environment, default_daemon_config_exists, default_daemon_config_path,
    ensure_daemon_config_exists, ensure_daemon_store_artifacts, load_daemon_settings,
    persist_daemon_cli_settings, persist_dashboard_tls_hint, update_daemon_telemetry_consent,
};
pub(crate) use install::{
    prepare_daemon_embeddings_install, prepare_daemon_inference_install,
    prepare_daemon_platform_embeddings_install,
};
pub(crate) use plans::DaemonEmbeddingsInstallMode;
