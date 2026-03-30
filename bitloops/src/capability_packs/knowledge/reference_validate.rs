use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::host::checkpoints::strategy::manual_commit::run_git;

pub(crate) fn is_valid_artefact_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('-').collect();
    if parts.len() != 5 {
        return false;
    }

    let expected_lengths = [8, 4, 4, 4, 12];
    parts
        .iter()
        .zip(expected_lengths.iter())
        .all(|(part, &len)| {
            part.len() == len
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
        })
}

pub fn resolve_commit_sha(repo_root: &Path, rev: &str) -> Result<String> {
    let trimmed = rev.trim();
    if trimmed.is_empty() {
        bail!("commit sha must not be empty");
    }

    let resolved = run_git(
        repo_root,
        &["rev-parse", "--verify", &format!("{trimmed}^{{commit}}")],
    )
    .with_context(|| format!("validating commit `{trimmed}`"))?;
    Ok(resolved.trim().to_string())
}
