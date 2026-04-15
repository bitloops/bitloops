use anyhow::Result;
use clap::Args;

#[derive(Args, Debug, Clone, Default)]
pub struct LogoutArgs {}

pub async fn run(_args: LogoutArgs) -> Result<()> {
    if crate::daemon::logout_workos_session().await? {
        println!("Logged out.");
    } else {
        println!("Not logged in.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn logout_command_parses() {
        let parsed = Cli::try_parse_from(["bitloops", "logout"]).expect("logout parses");

        let Some(Commands::Logout(_args)) = parsed.command else {
            panic!("expected logout command");
        };
    }
}
