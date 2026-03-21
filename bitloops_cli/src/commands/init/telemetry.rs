use std::env;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::settings;

use super::agent_selection::can_prompt_interactively;

pub(super) const TELEMETRY_OPTOUT_ENV: &str = "BITLOOPS_TELEMETRY_OPTOUT";

fn telemetry_settings_target(repo_root: &Path) -> PathBuf {
    let local = settings::settings_local_path(repo_root);
    if local.exists() {
        local
    } else {
        settings::settings_path(repo_root)
    }
}

fn persist_telemetry_choice(repo_root: &Path, choice: bool) -> Result<()> {
    let mut merged = settings::load_settings(repo_root).unwrap_or_default();
    merged.telemetry = Some(choice);
    settings::save_settings(&merged, &telemetry_settings_target(repo_root))?;
    Ok(())
}

pub(super) fn prompt_telemetry_consent(
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

pub(super) fn maybe_capture_telemetry_consent(
    repo_root: &Path,
    telemetry_flag: bool,
    allow_prompt: bool,
    out: &mut dyn Write,
) -> Result<()> {
    if !telemetry_flag
        || env::var(TELEMETRY_OPTOUT_ENV)
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
    {
        return persist_telemetry_choice(repo_root, false);
    }

    let existing = settings::load_settings(repo_root).unwrap_or_default();
    if existing.telemetry.is_some() {
        return Ok(());
    }

    if !allow_prompt || !can_prompt_interactively() {
        return Ok(());
    }

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let consent = prompt_telemetry_consent(out, &mut input)?;
    persist_telemetry_choice(repo_root, consent)
}
