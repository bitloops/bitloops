use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

pub const DEFAULT_DASHBOARD_PORT: u16 = crate::server::dashboard::DEFAULT_DASHBOARD_PORT;

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

pub async fn run(args: DashboardArgs) -> Result<()> {
    crate::server::dashboard::run(crate::server::dashboard::DashboardServerConfig {
        host: args.host,
        port: args.port,
        no_open: args.no_open,
        bundle_dir: args.bundle_dir,
    })
    .await
}
