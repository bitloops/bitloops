use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Subcommand};

use crate::config::settings;
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::state::find_most_recent_session;
use crate::host::checkpoints::transcript::parse::parse_from_file_at_line;
use crate::host::checkpoints::transcript::utils::extract_modified_files;
use crate::utils::paths;

#[derive(Args, Debug, Clone, Default)]
pub struct DebugArgs {
    #[command(subcommand)]
    pub command: Option<DebugCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DebugCommand {
    /// Show whether current state would trigger an auto-commit.
    #[command(name = "auto-commit")]
    AutoCommit(DebugAutoCommitArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct DebugAutoCommitArgs {
    /// Path to transcript file (.jsonl) to parse for modified files.
    #[arg(long, short = 't')]
    pub transcript: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct StatusChanges {
    modified: Vec<String>,
    untracked: Vec<String>,
    deleted: Vec<String>,
    staged: Vec<String>,
}

pub fn run(args: &DebugArgs) -> Result<()> {
    match &args.command {
        Some(DebugCommand::AutoCommit(cmd)) => run_auto_commit(cmd),
        None => {
            let mut cmd = crate::cli::Cli::command();
            let mut out = io::stdout();
            if let Some(debug_cmd) = cmd.find_subcommand_mut("debug") {
                debug_cmd.write_long_help(&mut out)?;
                writeln!(out)?;
                return Ok(());
            }
            bail!("debug command not found")
        }
    }
}

fn run_auto_commit(args: &DebugAutoCommitArgs) -> Result<()> {
    let repo_root = paths::repo_root();
    let mut out = io::stdout().lock();

    let Ok(repo_root) = repo_root else {
        writeln!(out, "Not in a git repository")?;
        return Ok(());
    };
    writeln!(out, "Repository: {}\n", repo_root.display())?;

    let strategy_name = settings::load_settings(&repo_root)
        .map(|s| s.strategy)
        .unwrap_or_else(|_| settings::DEFAULT_STRATEGY.to_string());
    let is_auto_commit = strategy_name == "auto-commit";

    writeln!(out, "Strategy: {strategy_name}")?;
    writeln!(out, "Auto-commit strategy: {is_auto_commit}")?;
    match crate::git::is_on_default_branch() {
        Ok((_is_default, branch)) if !branch.is_empty() => writeln!(out, "Branch: {branch}\n")?,
        Ok(_) => writeln!(out, "Branch: (detached)\n")?,
        Err(err) => writeln!(out, "Branch: (unable to determine: {err})\n")?,
    }

    writeln!(out, "=== Session State ===")?;
    let backend = create_session_backend_or_local(&repo_root);
    let sessions = backend.list_sessions().unwrap_or_default();
    let current_session = find_most_recent_session(&sessions, &repo_root.to_string_lossy());

    let mut transcript_path = args.transcript.clone().unwrap_or_default();
    let mut pre_untracked = Vec::<String>::new();

    if let Some(session) = &current_session {
        writeln!(out, "Current session: {}", session.session_id)?;
        if let Some(pre) = backend.load_pre_prompt(&session.session_id).ok().flatten() {
            writeln!(out, "Pre-prompt state: captured at {}", pre.timestamp)?;
            writeln!(
                out,
                "  Pre-existing untracked files: {}",
                pre.untracked_files.len()
            )?;
            pre_untracked = pre.untracked_files;
        } else {
            writeln!(out, "Pre-prompt state: (none captured)")?;
        }

        if transcript_path.is_empty() && !session.transcript_path.is_empty() {
            transcript_path = session.transcript_path.clone();
            writeln!(out, "\nAuto-detected transcript: {}", transcript_path)?;
        }
    } else {
        writeln!(out, "Current session: (none - no active session)")?;
    }

    writeln!(out, "\n=== File Changes ===")?;
    let status_changes = collect_status_changes(&repo_root)?;

    let total_changes = if !transcript_path.is_empty() {
        writeln!(out, "\nParsing transcript: {transcript_path}")?;
        let modified_from_transcript =
            extract_modified_from_transcript(&transcript_path, &repo_root).unwrap_or_default();
        writeln!(
            out,
            "  Found {} modified files in transcript",
            modified_from_transcript.len()
        )?;

        let new_files = status_changes
            .untracked
            .iter()
            .filter(|f| !pre_untracked.iter().any(|existing| existing == *f))
            .cloned()
            .collect::<Vec<_>>();

        print_file_list(
            &mut out,
            "Modified (from transcript)",
            "M",
            &modified_from_transcript,
        )?;
        print_file_list(
            &mut out,
            "New files (created during session)",
            "+",
            &new_files,
        )?;
        print_file_list(&mut out, "Deleted files", "D", &status_changes.deleted)?;

        let total = modified_from_transcript.len() + new_files.len() + status_changes.deleted.len();
        if total == 0 {
            writeln!(out, "\nNo changes detected from transcript")?;
        }
        total
    } else {
        writeln!(
            out,
            "\n(No --transcript provided, showing git status instead)"
        )?;
        writeln!(
            out,
            "Note: Stop hook uses transcript parsing, not git status"
        )?;

        print_file_list(&mut out, "Staged files", "+", &status_changes.staged)?;
        print_file_list(&mut out, "Modified files", "M", &status_changes.modified)?;
        print_file_list(&mut out, "Untracked files", "?", &status_changes.untracked)?;
        print_file_list(&mut out, "Deleted files", "D", &status_changes.deleted)?;

        let total = status_changes.modified.len()
            + status_changes.untracked.len()
            + status_changes.deleted.len()
            + status_changes.staged.len();
        if total == 0 {
            writeln!(out, "\nNo changes detected in git status")?;
        }
        total
    };

    writeln!(out, "\n=== Auto-Commit Decision ===")?;
    let would_commit = is_auto_commit && total_changes > 0;
    if would_commit {
        writeln!(out, "Result: YES - Auto-commit would be triggered")?;
        writeln!(out, "  {total_changes} file(s) would be committed")?;
    } else {
        writeln!(out, "Result: NO - Auto-commit would NOT be triggered")?;
        writeln!(out, "Reasons:")?;
        if !is_auto_commit {
            writeln!(
                out,
                "  - Strategy is not auto-commit (using {strategy_name})"
            )?;
        }
        if total_changes == 0 {
            writeln!(out, "  - No file changes to commit")?;
        }
    }

    if transcript_path.is_empty() {
        writeln!(out, "\n=== Finding Transcript ===")?;
        writeln!(out, "Claude Code transcripts are typically at:")?;
        if let Ok(home) = std::env::var("HOME") {
            writeln!(out, "  {home}/.claude/projects/*/sessions/*.jsonl")?;
        } else {
            writeln!(out, "  ~/.claude/projects/*/sessions/*.jsonl")?;
        }
    }

    Ok(())
}

fn extract_modified_from_transcript(
    transcript_path: &str,
    repo_root: &Path,
) -> Result<Vec<String>> {
    let (lines, _) = parse_from_file_at_line(transcript_path, 0)
        .with_context(|| format!("failed to parse transcript: {transcript_path}"))?;
    let modified = extract_modified_files(&lines);
    Ok(filter_and_normalize_paths(&modified, repo_root))
}

fn collect_status_changes(repo_root: &Path) -> Result<StatusChanges> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output()
        .context("failed to run git status --porcelain")?;
    if !output.status.success() {
        bail!(
            "failed to inspect git status: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut modified = BTreeSet::<String>::new();
    let mut untracked = BTreeSet::<String>::new();
    let mut deleted = BTreeSet::<String>::new();
    let mut staged = BTreeSet::<String>::new();

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.len() < 3 {
            continue;
        }
        let status = &line[..2];
        let mut path = line[3..].trim().to_string();
        if let Some(idx) = path.rfind(" -> ") {
            path = path[idx + 4..].to_string();
        }
        if path.is_empty() || path.ends_with('/') || paths::is_infrastructure_path(&path) {
            continue;
        }

        if status == "??" {
            untracked.insert(path);
            continue;
        }

        let bytes = status.as_bytes();
        let x = bytes.first().copied().unwrap_or(b' ');
        let y = bytes.get(1).copied().unwrap_or(b' ');

        if x == b'D' || y == b'D' {
            deleted.insert(path);
            continue;
        }

        if x != b' ' {
            staged.insert(path.clone());
        }
        if y != b' ' {
            modified.insert(path);
        }
    }

    Ok(StatusChanges {
        modified: modified.into_iter().collect(),
        untracked: untracked.into_iter().collect(),
        deleted: deleted.into_iter().collect(),
        staged: staged.into_iter().collect(),
    })
}

fn filter_and_normalize_paths(files: &[String], repo_root: &Path) -> Vec<String> {
    let base = repo_root.to_string_lossy();
    let mut out = Vec::new();
    for file in files {
        let rel = paths::to_relative_path(file, &base);
        if rel.is_empty() || rel.starts_with("..") || paths::is_infrastructure_path(&rel) {
            continue;
        }
        out.push(rel);
    }
    out.sort();
    out.dedup();
    out
}

fn print_file_list(
    out: &mut dyn Write,
    label: &str,
    prefix: &str,
    files: &[String],
) -> io::Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    writeln!(out, "\n{label} ({}):", files.len())?;
    for file in files {
        writeln!(out, "  {prefix} {file}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn debug_args_default_no_subcommand() {
        let args = DebugArgs::default();
        assert!(args.command.is_none());
    }

    #[test]
    fn filter_and_normalize_skips_infrastructure_and_outside_paths() {
        let repo_root = PathBuf::from("/repo");
        let input = vec![
            "/repo/src/main.rs".to_string(),
            "/repo/.bitloops/tmp/file".to_string(),
            "/other/outside.rs".to_string(),
            "relative.rs".to_string(),
        ];

        let out = filter_and_normalize_paths(&input, &repo_root);
        assert_eq!(
            out,
            vec!["relative.rs".to_string(), "src/main.rs".to_string()]
        );
    }
}
