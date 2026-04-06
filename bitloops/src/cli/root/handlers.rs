use anyhow::{Context, Result};
use std::env;
#[cfg(test)]
use std::io::BufRead;
use std::io::{self, Write};

use crate::cli::{clean, doctor, enable, reset, resume};
use crate::config::settings;

use super::args::{CleanArgs, DisableArgs, DoctorArgs, HelpArgs, ResetArgs, ResumeArgs};
use super::completion;
use super::help::write_help;
use super::settings::load_settings_once;
use super::version;

pub fn run_clean_command(args: &CleanArgs) -> Result<()> {
    let mut out = io::stdout();
    clean::run_clean(&mut out, args.force)
}

pub fn run_disable_command(args: &DisableArgs) -> Result<()> {
    let cwd = env::current_dir().context("getting current directory")?;
    let mut out = io::stdout();
    enable::run_disable(&cwd, &mut out, args.project)
}

pub fn run_doctor_command(args: &DoctorArgs) -> Result<()> {
    doctor::run_doctor(args.force)
}

pub fn run_help_command(args: &HelpArgs) -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    write_help(&mut out, &args.command, args.tree)
}

pub fn run_root_default_help() -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    write_help(&mut out, &[], false)
}

pub fn run_reset_command(args: &ResetArgs) -> Result<()> {
    let cwd = env::current_dir().context("getting current directory")?;
    let repo_root = enable::find_repo_root(&cwd)?;
    let strategy_name = load_settings_once(&repo_root)
        .map(|settings| settings.strategy)
        .unwrap_or_else(|| settings::DEFAULT_STRATEGY.to_string());

    let config = reset::ResetConfig {
        repo_root,
        force: args.force,
        session_id: args.session.clone(),
        strategy_name,
    };
    reset::run_reset_cmd(&config)
}

pub fn run_resume_command(args: &ResumeArgs) -> Result<()> {
    let cwd = env::current_dir().context("getting current directory")?;
    let repo_root = enable::find_repo_root(&cwd)?;
    let outcome = resume::run_resume(&repo_root, &args.branch, args.force)?;
    if !outcome.message.is_empty() {
        println!("{}", outcome.message);
    }
    Ok(())
}

pub fn run_send_analytics_command(
    args: &crate::telemetry::analytics::SendAnalyticsArgs,
) -> Result<()> {
    crate::telemetry::analytics::send_event(&args.payload);
    Ok(())
}

pub fn run_completion_command(args: &super::args::CompletionArgs) -> Result<()> {
    completion::run_completion_command(args)
}

pub fn run_version_command(check_for_updates: bool) -> Result<()> {
    version::run_version_command(check_for_updates)
}

#[cfg(test)]
pub(crate) fn run_curl_bash_post_install_command_with_io(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<()> {
    if let Err(err) = enable::run_post_install_shell_completion_with_io(out, input) {
        writeln!(out, "Note: Shell completion setup skipped: {err}")?;
    }
    Ok(())
}

pub fn run_curl_bash_post_install_command() -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if let Err(err) = enable::run_post_install_shell_completion(&mut out) {
        writeln!(out, "Note: Shell completion setup skipped: {err}")?;
    }
    Ok(())
}
