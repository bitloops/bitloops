use anyhow::Result;
use std::env;
use std::io::{self, Write};

use crate::cli::versioncheck;
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, bitloops_wordmark, color_hex_if_enabled};

use super::build::{build_commit, build_date, build_target, build_version};

const VERSION_DIVIDER: &str = "───────────────────";

fn pretty_version(version: &str) -> String {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return "unknown".to_string();
    }
    if trimmed == "dev" {
        return format!("v{}", env!("CARGO_PKG_VERSION"));
    }
    if trimmed.starts_with('v') {
        return trimmed.to_string();
    }
    format!("v{trimmed}")
}

fn short_commit(commit: &str) -> String {
    let trimmed = commit.trim();
    if trimmed.is_empty() {
        return "unknown".to_string();
    }

    trimmed.chars().take(7).collect()
}

fn version_for_update_check() -> &'static str {
    let version = build_version();
    if version == "dev" {
        env!("CARGO_PKG_VERSION")
    } else {
        version
    }
}

pub(crate) fn write_version(
    w: &mut dyn Write,
    version: &str,
    commit: &str,
    target: &str,
    built: &str,
) -> Result<()> {
    writeln!(w)?;
    writeln!(
        w,
        "{}",
        color_hex_if_enabled(&bitloops_wordmark(), BITLOOPS_PURPLE_HEX)
    )?;
    writeln!(w, "Bitloops CLI {}", pretty_version(version))?;
    writeln!(w, "{VERSION_DIVIDER}")?;
    writeln!(w, "commit: {}", short_commit(commit))?;
    writeln!(w, "target: {}", target.trim())?;
    writeln!(w, "built: {}", built.trim())?;
    writeln!(w)?;
    Ok(())
}

pub fn run_version_command(check_for_updates: bool) -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    write_version(
        &mut out,
        build_version(),
        build_commit(),
        build_target(),
        build_date(),
    )?;

    if check_for_updates {
        versioncheck::check_now(&mut out, version_for_update_check());
    } else {
        writeln!(
            out,
            "Run `bitloops --version --check` to check for updates."
        )?;
    }

    Ok(())
}
