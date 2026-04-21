use anyhow::{Result, bail};
use chrono::{Local, TimeZone};
use clap::{Args, Subcommand};

#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
use std::rc::Rc;

#[cfg(test)]
type EnsureLoggedInHook = dyn Fn() -> Result<crate::daemon::WorkosSessionDetails> + 'static;

#[cfg(test)]
thread_local! {
    static ENSURE_LOGGED_IN_HOOK: RefCell<Option<Rc<EnsureLoggedInHook>>> = RefCell::new(None);
}

#[derive(Args, Debug, Clone, Default)]
pub struct LoginArgs {
    #[command(subcommand)]
    pub command: Option<LoginCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum LoginCommand {
    /// Show the current login session status.
    Status(LoginStatusArgs),
    /// Print the current platform bearer token, refreshing it if needed.
    Token(LoginTokenArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct LoginStatusArgs {}

#[derive(Args, Debug, Clone, Default)]
pub struct LoginTokenArgs {}

pub async fn run(args: LoginArgs) -> Result<()> {
    match args.command {
        Some(LoginCommand::Status(_args)) => run_status().await,
        Some(LoginCommand::Token(_args)) => run_token(),
        None => {
            ensure_logged_in().await?;
            Ok(())
        }
    }
}

pub(crate) async fn ensure_logged_in() -> Result<crate::daemon::WorkosSessionDetails> {
    #[cfg(test)]
    if let Some(hook) = ENSURE_LOGGED_IN_HOOK.with(|cell| cell.borrow().clone()) {
        return hook();
    }

    match crate::daemon::prepare_workos_device_login().await? {
        crate::daemon::WorkosLoginStart::AlreadyLoggedIn(session) => {
            println!("Already signed in as {}.", session.display_label());
            Ok(session)
        }
        crate::daemon::WorkosLoginStart::Pending(start) => {
            println!();
            println!("Sign in to Bitloops");
            println!();
            println!("Open the following URL in your browser:");
            println!(
                "{}",
                start
                    .verification_url_complete
                    .as_deref()
                    .unwrap_or(&start.verification_url)
            );
            println!();
            println!("Verification Code: {}", start.user_code);

            if let Some(url) = start
                .verification_url_complete
                .as_deref()
                .or(Some(start.verification_url.as_str()))
                && let Err(err) = crate::api::open_in_default_browser(url)
            {
                eprintln!("[bitloops] Warning: failed to open browser automatically: {err:#}");
            }

            println!();
            println!("Waiting for authentication…");
            let session = crate::daemon::complete_workos_device_login(&start).await?;
            println!("🔒 Signed in as {}", session.display_label());
            println!();
            Ok(session)
        }
    }
}

#[cfg(test)]
pub(crate) fn with_ensure_logged_in_hook<T>(
    hook: impl Fn() -> Result<crate::daemon::WorkosSessionDetails> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    ENSURE_LOGGED_IN_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "ensure_logged_in hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });
    let result = f();
    ENSURE_LOGGED_IN_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}

async fn run_status() -> Result<()> {
    let Some(session) = crate::daemon::resolve_workos_session_status().await? else {
        println!("Not signed in.");
        return Ok(());
    };

    for line in render_status_lines(&session)? {
        println!("{line}");
    }
    Ok(())
}

fn run_token() -> Result<()> {
    let Some(token) = crate::daemon::platform_gateway_bearer_token()? else {
        bail!("not signed in or no platform bearer token is available; run `bitloops login` first");
    };
    println!("{token}");
    Ok(())
}

fn render_status_lines(session: &crate::daemon::WorkosSessionDetails) -> Result<Vec<String>> {
    let mut lines = vec![format!("Signed in as {}.", session.display_label())];
    if let Some(email) = session.user_email.as_deref() {
        lines.push(format!("Email: {email}"));
    }
    if let Some(authentication_method) = session.authentication_method.as_deref() {
        lines.push(format!("Method: {authentication_method}"));
    }
    if let Some(organisation_id) = session.organisation_id.as_deref() {
        lines.push(format!("Organisation: {organisation_id}"));
    }
    if let Some(expires_at_unix) = session.access_token_expires_at_unix {
        let Some(expires_at) = Local.timestamp_opt(expires_at_unix as i64, 0).single() else {
            bail!("stored WorkOS expiry timestamp is invalid");
        };
        lines.push(format!(
            "Access token expires: {}",
            expires_at.format("%Y-%m-%d %H:%M:%S %:z")
        ));
    }
    Ok(lines)
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

    #[test]
    fn login_token_subcommand_parses() {
        let parsed =
            Cli::try_parse_from(["bitloops", "login", "token"]).expect("login token parses");

        let Some(Commands::Login(args)) = parsed.command else {
            panic!("expected login command");
        };
        assert!(matches!(
            args.command,
            Some(super::LoginCommand::Token(super::LoginTokenArgs {}))
        ));
    }

    #[test]
    fn login_status_does_not_render_the_access_token() {
        let lines = super::render_status_lines(&crate::daemon::WorkosSessionDetails {
            client_id: "client_test".to_string(),
            user_id: Some("user_123".to_string()),
            user_email: Some("cli@example.com".to_string()),
            user_first_name: Some("CLI".to_string()),
            user_last_name: Some("User".to_string()),
            organisation_id: Some("org_123".to_string()),
            authentication_method: Some("GoogleOAuth".to_string()),
            access_token_expires_at_unix: None,
            authenticated_at_unix: 0,
            updated_at_unix: 0,
        })
        .expect("status lines");

        assert_eq!(lines[0], "Signed in as CLI User.");
        assert!(lines.contains(&"Email: cli@example.com".to_string()));
        assert!(lines.contains(&"Method: GoogleOAuth".to_string()));
        assert!(lines.contains(&"Organisation: org_123".to_string()));
        assert!(!lines.iter().any(|line| line.starts_with("Access token:")));
    }
}
