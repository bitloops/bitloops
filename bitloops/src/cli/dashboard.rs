use anyhow::Result;
use clap::Args;

#[derive(Args, Debug, Clone, Default)]
pub struct DashboardArgs {}

pub async fn run(_args: DashboardArgs) -> Result<()> {
    crate::cli::daemon::launch_dashboard().await
}

#[cfg(test)]
mod tests {
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn dashboard_cli_no_longer_accepts_server_flags() {
        let parsed = Cli::try_parse_from(["bitloops", "dashboard"])
            .expect("dashboard invocation should parse");

        let Some(Commands::Dashboard(args)) = parsed.command else {
            panic!("expected dashboard command");
        };

        let _ = args;
    }
}
