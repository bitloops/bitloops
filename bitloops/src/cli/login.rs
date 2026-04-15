use anyhow::{Result, bail};
use chrono::{Local, TimeZone};
use clap::{Args, Subcommand};

#[derive(Args, Debug, Clone, Default)]
pub struct LoginArgs {
    #[command(subcommand)]
    pub command: Option<LoginCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum LoginCommand {
    /// Show the current login session status.
    Status(LoginStatusArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct LoginStatusArgs {}

pub async fn run(args: LoginArgs) -> Result<()> {
    match args.command {
        Some(LoginCommand::Status(_args)) => run_status().await,
        None => run_login().await,
    }
}

async fn run_login() -> Result<()> {
    match crate::daemon::prepare_workos_device_login().await? {
        crate::daemon::WorkosLoginStart::AlreadyLoggedIn(session) => {
            println!("Already logged in as {}.", session.display_label());
            Ok(())
        }
        crate::daemon::WorkosLoginStart::Pending(start) => {
            println!("Open this URL to sign in:");
            println!(
                "{}",
                start
                    .verification_url_complete
                    .as_deref()
                    .unwrap_or(&start.verification_url)
            );
            println!();
            println!("If prompted, enter this code: {}", start.user_code);

            if let Some(url) = start
                .verification_url_complete
                .as_deref()
                .or(Some(start.verification_url.as_str()))
            {
                if let Err(err) = crate::api::open_in_default_browser(url) {
                    eprintln!("[bitloops] Warning: failed to open browser automatically: {err:#}");
                }
            }

            println!("Waiting for WorkOS login to complete...");
            let session = crate::daemon::complete_workos_device_login(&start).await?;
            println!("Logged in as {}.", session.display_label());
            Ok(())
        }
    }
}

async fn run_status() -> Result<()> {
    let Some(session) = crate::daemon::resolve_workos_session_status().await? else {
        println!("Not logged in.");
        return Ok(());
    };

    println!("Logged in as {}.", session.display_label());
    if let Some(email) = session.user_email.as_deref() {
        println!("Email: {email}");
    }
    if let Some(authentication_method) = session.authentication_method.as_deref() {
        println!("Method: {authentication_method}");
    }
    if let Some(organisation_id) = session.organisation_id.as_deref() {
        println!("Organisation: {organisation_id}");
    }
    if let Some(expires_at_unix) = session.access_token_expires_at_unix {
        let Some(expires_at) = Local.timestamp_opt(expires_at_unix as i64, 0).single() else {
            bail!("stored WorkOS expiry timestamp is invalid");
        };
        println!(
            "Access token expires: {}",
            expires_at.format("%Y-%m-%d %H:%M:%S %:z")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn login_status_subcommand_parses() {
        let parsed =
            Cli::try_parse_from(["bitloops", "login", "status"]).expect("login status parses");

        let Some(Commands::Login(args)) = parsed.command else {
            panic!("expected login command");
        };
        assert!(matches!(
            args.command,
            Some(super::LoginCommand::Status(super::LoginStatusArgs {}))
        ));
    }
}
