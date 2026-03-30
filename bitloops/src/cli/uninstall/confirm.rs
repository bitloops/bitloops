use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::Result;

use super::targets::{ALL_TARGETS, UninstallTarget};

pub(super) fn confirm_uninstall(
    out: &mut dyn Write,
    targets: &BTreeSet<UninstallTarget>,
    hook_repo_roots: &[PathBuf],
    legacy_repo_roots: &[PathBuf],
) -> Result<bool> {
    writeln!(out)?;
    writeln!(out, "This will remove the following Bitloops artefacts:")?;
    for target in ALL_TARGETS
        .iter()
        .copied()
        .filter(|target| targets.contains(target))
    {
        writeln!(
            out,
            "  - {}",
            target.summary(hook_repo_roots.len(), legacy_repo_roots.len())
        )?;
    }
    write!(out, "\nContinue? [y/N]: ")?;
    out.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
