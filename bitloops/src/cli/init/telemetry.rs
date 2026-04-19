use std::env;
use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::Result;

use crate::config::{load_daemon_settings, persist_daemon_cli_settings};

use super::agent_selection::can_prompt_interactively;

pub(crate) const TELEMETRY_OPTOUT_ENV: &str = "BITLOOPS_TELEMETRY_OPTOUT";

fn persist_telemetry_choice(choice: bool) -> Result<()> {
    let loaded = load_daemon_settings(None)?;
    let mut cli = loaded.cli;
    cli.telemetry = Some(choice);
    persist_daemon_cli_settings(&cli)?;
    Ok(())
}

pub(crate) fn prompt_telemetry_consent(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<bool> {
    writeln!(out)?;
    writeln!(out, "Help improve Bitloops CLI?")?;
    writeln!(
        out,
        "Share anonymous usage data. No code or personal info collected."
    )?;

    loop {
        write!(out, "Enable anonymous telemetry? [Y/n]: ")?;
        out.flush()?;

        let mut line = String::new();
        input.read_line(&mut line)?;
        let answer = line.trim().to_ascii_lowercase();
        match answer.as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => {
                writeln!(out, "Please answer yes or no.")?;
            }
        }
    }
}

pub(crate) fn maybe_capture_telemetry_consent(
    repo_root: &Path,
    telemetry_flag: bool,
    allow_prompt: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let _ = repo_root;
    if !telemetry_flag
        || env::var(TELEMETRY_OPTOUT_ENV)
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
    {
        return persist_telemetry_choice(false);
    }

    if load_daemon_settings(None)?.cli.telemetry.is_some() {
        return Ok(());
    }

    if !allow_prompt || !can_prompt_interactively() {
        return Ok(());
    }

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let consent = prompt_telemetry_consent(out, &mut input)?;
    persist_telemetry_choice(consent)
}
