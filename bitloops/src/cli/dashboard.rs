use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

pub const DEFAULT_DASHBOARD_PORT: u16 = crate::api::DEFAULT_DASHBOARD_PORT;

#[derive(Args, Debug, Clone)]
pub struct DashboardArgs {
    /// Hostname to bind the dashboard server to.
    #[arg(long)]
    pub host: Option<String>,

    /// Port to bind the dashboard server to.
    #[arg(long, default_value_t = DEFAULT_DASHBOARD_PORT)]
    pub port: u16,

    /// Do not open the dashboard URL in the default browser.
    #[arg(long, default_value_t = false)]
    pub no_open: bool,

    /// Path to the dashboard bundle directory (contains index.html).
    #[arg(long = "bundle-dir", alias = "bundle", value_name = "PATH")]
    pub bundle_dir: Option<PathBuf>,
}

fn build_server_config(args: DashboardArgs) -> crate::api::DashboardServerConfig {
    crate::api::DashboardServerConfig {
        host: args.host,
        port: args.port,
        no_open: args.no_open,
        bundle_dir: args.bundle_dir,
    }
}

pub async fn run(args: DashboardArgs) -> Result<()> {
    crate::api::run(build_server_config(args)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn dashboard_cli_maps_bundle_alias_into_server_config() {
        let parsed = Cli::try_parse_from([
            "bitloops",
            "dashboard",
            "--host",
            "0.0.0.0",
            "--port",
            "6100",
            "--no-open",
            "--bundle",
            "/tmp/custom-bundle",
        ])
        .expect("dashboard invocation should parse");

        let Some(Commands::Dashboard(args)) = parsed.command else {
            panic!("expected dashboard command");
        };

        let config = build_server_config(args);
        assert_eq!(config.host.as_deref(), Some("0.0.0.0"));
        assert_eq!(config.port, 6100);
        assert!(config.no_open);
        assert_eq!(config.bundle_dir, Some(PathBuf::from("/tmp/custom-bundle")));
    }

    #[test]
    fn dashboard_cli_defaults_track_server_defaults() {
        let parsed = Cli::try_parse_from(["bitloops", "dashboard"])
            .expect("dashboard invocation should parse");

        let Some(Commands::Dashboard(args)) = parsed.command else {
            panic!("expected dashboard command");
        };

        assert_eq!(args.port, DEFAULT_DASHBOARD_PORT);
        assert_eq!(DEFAULT_DASHBOARD_PORT, crate::api::DEFAULT_DASHBOARD_PORT);
        assert!(args.host.is_none());
        assert!(args.bundle_dir.is_none());
        assert!(!args.no_open);
    }
}
