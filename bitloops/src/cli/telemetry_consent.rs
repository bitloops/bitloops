use std::env;
use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::json;

#[cfg(not(test))]
use std::io::IsTerminal;
#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

pub(crate) const CURRENT_CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const NON_INTERACTIVE_TELEMETRY_ERROR: &str = "Telemetry consent is required in non-interactive mode. Re-run with `--telemetry`, `--telemetry=false`, or `--no-telemetry`.";

const UPDATE_CLI_TELEMETRY_MUTATION: &str = r#"
    mutation UpdateCliTelemetryConsent($cliVersion: String!, $telemetry: Boolean) {
      updateCliTelemetryConsent(cliVersion: $cliVersion, telemetry: $telemetry) {
        telemetry
        needsPrompt
      }
    }
"#;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TelemetryConsentResult {
    pub telemetry: Option<bool>,
    pub needs_prompt: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCliTelemetryConsentMutationData {
    update_cli_telemetry_consent: TelemetryConsentResult,
}

#[cfg(test)]
type GlobalGraphqlExecutorHook =
    dyn Fn(&Path, &str, &serde_json::Value) -> Result<serde_json::Value> + 'static;

#[cfg(test)]
thread_local! {
    static GLOBAL_GRAPHQL_EXECUTOR_HOOK: RefCell<Option<Rc<GlobalGraphqlExecutorHook>>> =
        RefCell::new(None);
    static TEST_TTY_OVERRIDE: RefCell<Option<bool>> = const { RefCell::new(None) };
    static TEST_ASSUME_DAEMON_RUNNING_OVERRIDE: RefCell<Option<bool>> = const { RefCell::new(None) };
}

pub(crate) fn can_prompt_interactively() -> bool {
    #[cfg(test)]
    if let Some(value) = test_tty_override() {
        return value;
    }
    if let Ok(value) = env::var("BITLOOPS_TEST_TTY") {
        return value == "1";
    }

    #[cfg(test)]
    {
        false
    }

    #[cfg(not(test))]
    {
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }
}

pub(crate) fn telemetry_flag_choice(telemetry: Option<bool>, no_telemetry: bool) -> Option<bool> {
    if no_telemetry { Some(false) } else { telemetry }
}

pub(crate) fn prompt_default_config_setup(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<bool> {
    prompt_yes_no(
        out,
        input,
        &[],
        "No global Bitloops daemon config was found. Set up the default configuration? [Y/n] ",
    )
}

pub(crate) fn prompt_telemetry_consent(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<bool> {
    prompt_yes_no(
        out,
        input,
        &[
            "Help us improve Bitloops",
            "Share anonymous usage data to help us make Bitloops better. No code or personal information is collected.",
        ],
        "Enable anonymous telemetry? [Y/n] ",
    )
}

pub(crate) async fn update_cli_telemetry_consent_via_daemon(
    runtime_root: &Path,
    telemetry: Option<bool>,
) -> Result<TelemetryConsentResult> {
    let data: UpdateCliTelemetryConsentMutationData = execute_global_graphql(
        runtime_root,
        UPDATE_CLI_TELEMETRY_MUTATION,
        json!({
            "cliVersion": CURRENT_CLI_VERSION,
            "telemetry": telemetry,
        }),
    )
    .await?;

    Ok(data.update_cli_telemetry_consent)
}

pub(crate) async fn ensure_existing_config_telemetry_consent(
    runtime_root: &Path,
    explicit_choice: Option<bool>,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<()> {
    let state = update_cli_telemetry_consent_via_daemon(runtime_root, explicit_choice).await?;
    if !state.needs_prompt {
        return Ok(());
    }

    if !can_prompt_interactively() {
        bail!(NON_INTERACTIVE_TELEMETRY_ERROR);
    }

    let choice = prompt_telemetry_consent(out, input)?;
    let persisted = update_cli_telemetry_consent_via_daemon(runtime_root, Some(choice)).await?;
    if persisted.needs_prompt {
        bail!("failed to persist telemetry consent");
    }

    Ok(())
}

pub(crate) async fn ensure_default_daemon_running() -> Result<()> {
    #[cfg(test)]
    match test_assume_daemon_running_override() {
        Some(true) => return Ok(()),
        Some(false) => {}
        None => {
            if env::var("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING")
                .ok()
                .is_some_and(|value| !value.trim().is_empty() && value.trim() != "0")
            {
                return Ok(());
            }
        }
    }

    let status = crate::daemon::status().await?;
    if status.runtime.is_some() {
        return Ok(());
    }

    if crate::config::default_daemon_config_exists()? {
        bail!("Bitloops daemon is not running. Start it with `bitloops start`.")
    }

    bail!(
        "Bitloops daemon has not been bootstrapped yet. Run `bitloops start --create-default-config` or `bitloops init --install-default-daemon`."
    )
}

fn prompt_yes_no(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    lines: &[&str],
    prompt: &str,
) -> Result<bool> {
    if !lines.is_empty() {
        writeln!(out)?;
        for line in lines {
            writeln!(out, "{line}")?;
        }
    }

    loop {
        write!(out, "{prompt}")?;
        out.flush()?;

        let mut line = String::new();
        input.read_line(&mut line)?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(out, "Please answer yes or no.")?,
        }
    }
}

async fn execute_global_graphql<T: for<'de> Deserialize<'de>>(
    runtime_root: &Path,
    query: &str,
    variables: serde_json::Value,
) -> Result<T> {
    #[cfg(test)]
    if let Some(data) = maybe_execute_global_graphql_via_hook(runtime_root, query, &variables) {
        return Ok(serde_json::from_value(data?)?);
    }

    crate::daemon::execute_repo_graphql(runtime_root, query, variables).await
}

#[cfg(test)]
pub(crate) fn with_global_graphql_executor_hook<T>(
    hook: impl Fn(&Path, &str, &serde_json::Value) -> Result<serde_json::Value> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    GLOBAL_GRAPHQL_EXECUTOR_HOOK.with(|cell: &RefCell<Option<Rc<GlobalGraphqlExecutorHook>>>| {
        assert!(
            cell.borrow().is_none(),
            "global graphql executor hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });
    let result = f();
    GLOBAL_GRAPHQL_EXECUTOR_HOOK.with(|cell: &RefCell<Option<Rc<GlobalGraphqlExecutorHook>>>| {
        *cell.borrow_mut() = None;
    });
    result
}

#[cfg(test)]
fn maybe_execute_global_graphql_via_hook(
    runtime_root: &Path,
    query: &str,
    variables: &serde_json::Value,
) -> Option<Result<serde_json::Value>> {
    GLOBAL_GRAPHQL_EXECUTOR_HOOK.with(|hook: &RefCell<Option<Rc<GlobalGraphqlExecutorHook>>>| {
        hook.borrow()
            .as_ref()
            .map(|hook| hook(runtime_root, query, variables))
    })
}

#[cfg(test)]
pub(crate) fn test_tty_override() -> Option<bool> {
    TEST_TTY_OVERRIDE.with(|cell| *cell.borrow())
}

#[cfg(test)]
fn test_assume_daemon_running_override() -> Option<bool> {
    TEST_ASSUME_DAEMON_RUNNING_OVERRIDE.with(|cell| *cell.borrow())
}

#[cfg(test)]
pub(crate) fn with_test_tty_override<T>(value: bool, f: impl FnOnce() -> T) -> T {
    TEST_TTY_OVERRIDE.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "test tty override already installed"
        );
        *cell.borrow_mut() = Some(value);
    });
    let result = f();
    TEST_TTY_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}

#[cfg(test)]
pub(crate) fn with_test_assume_daemon_running<T>(value: bool, f: impl FnOnce() -> T) -> T {
    TEST_ASSUME_DAEMON_RUNNING_OVERRIDE.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "test daemon-running override already installed"
        );
        *cell.borrow_mut() = Some(value);
    });
    let result = f();
    TEST_ASSUME_DAEMON_RUNNING_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}
