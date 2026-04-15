use std::collections::BTreeSet;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::Result;

use super::targets::{ALL_TARGETS, UninstallTarget};

pub(super) fn confirm_uninstall(
    out: &mut dyn Write,
    targets: &BTreeSet<UninstallTarget>,
    agent_project_roots: &[PathBuf],
    hook_repo_roots: &[PathBuf],
    repo_data_roots: &[PathBuf],
) -> Result<bool> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    confirm_uninstall_with_input(
        out,
        targets,
        agent_project_roots,
        hook_repo_roots,
        repo_data_roots,
        &mut input,
    )
}

fn confirm_uninstall_with_input(
    out: &mut dyn Write,
    targets: &BTreeSet<UninstallTarget>,
    agent_project_roots: &[PathBuf],
    hook_repo_roots: &[PathBuf],
    repo_data_roots: &[PathBuf],
    input: &mut dyn BufRead,
) -> Result<bool> {
    writeln!(out)?;
    writeln!(out, "This will remove the following Bitloops artefacts:")?;
    for target in ALL_TARGETS
        .iter()
        .copied()
        .filter(|target| targets.contains(target))
    {
        let summary = match target {
            UninstallTarget::AgentHooks => {
                format!(
                    "Agent hooks in {} Bitloops project(s)",
                    agent_project_roots.len()
                )
            }
            _ => target.summary(hook_repo_roots.len(), repo_data_roots.len()),
        };
        writeln!(out, "  - {summary}")?;
    }
    write!(out, "\nContinue? [y/N]: ")?;
    out.flush()?;

    let mut line = String::new();
    input.read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn targets(values: &[UninstallTarget]) -> BTreeSet<UninstallTarget> {
        values.iter().copied().collect()
    }

    #[test]
    fn confirm_uninstall_accepts_yes_variants() {
        for response in ["y\n", "YeS\n"] {
            let mut out = Vec::new();
            let mut input = Cursor::new(response.as_bytes());

            let confirmed = confirm_uninstall_with_input(
                &mut out,
                &targets(&[UninstallTarget::AgentHooks]),
                &[],
                &[],
                &[],
                &mut input,
            )
            .expect("confirmation should succeed");

            assert!(confirmed, "response {response:?} should confirm");
        }
    }

    #[test]
    fn confirm_uninstall_defaults_to_no_and_lists_selected_targets() {
        let mut out = Vec::new();
        let mut input = Cursor::new(b"\n".to_vec());
        let confirmed = confirm_uninstall_with_input(
            &mut out,
            &targets(&[UninstallTarget::AgentHooks, UninstallTarget::Data]),
            &[
                PathBuf::from("/tmp/project-a"),
                PathBuf::from("/tmp/project-b"),
            ],
            &[PathBuf::from("/tmp/repo-a"), PathBuf::from("/tmp/repo-b")],
            &[PathBuf::from("/tmp/repo-a")],
            &mut input,
        )
        .expect("confirmation should succeed");

        assert!(!confirmed);

        let output = String::from_utf8(out).expect("output should be utf-8");
        assert!(output.contains("This will remove the following Bitloops artefacts:"));
        assert!(output.contains("Agent hooks in 2 Bitloops project(s)"));
        assert!(output.contains("Global data directory and .bitloops dirs in 1 repo(s)"));
        assert!(output.contains("Continue? [y/N]: "));
    }
}
