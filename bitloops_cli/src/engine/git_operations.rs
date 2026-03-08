use crate::engine::paths;
use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashSet;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct GitAuthor {
    pub name: String,
    pub email: String,
}

pub fn get_current_branch() -> Result<String> {
    let output = run_git(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = stdout_trimmed(&output);
    if branch == "HEAD" {
        bail!("not on a branch (detached HEAD)");
    }
    Ok(branch.to_string())
}

pub fn get_merge_base(branch1: &str, branch2: &str) -> Result<String> {
    let output = run_git(&["merge-base", branch1, branch2])?;
    Ok(stdout_trimmed(&output).to_string())
}

pub fn has_uncommitted_changes() -> Result<bool> {
    let output = run_git(&["status", "--porcelain"])?;
    Ok(!stdout_trimmed(&output).is_empty())
}

pub fn find_new_untracked_files(current: &[String], pre_existing: &[String]) -> Vec<String> {
    let existing: HashSet<&str> = pre_existing.iter().map(String::as_str).collect();
    current
        .iter()
        .filter(|file| !existing.contains(file.as_str()))
        .cloned()
        .collect()
}

pub fn get_git_config_value(key: &str) -> String {
    match Command::new("git").args(["config", "--get", key]).output() {
        Ok(output) if output.status.success() => stdout_trimmed(&output).to_string(),
        _ => String::new(),
    }
}

pub fn get_git_author() -> Result<GitAuthor> {
    run_git(&["rev-parse", "--git-dir"]).context("failed to open git repository")?;

    let mut name = get_git_config_value("user.name");
    let mut email = get_git_config_value("user.email");

    if name.is_empty() {
        name = "Unknown".to_string();
    }
    if email.is_empty() {
        email = "unknown@local".to_string();
    }

    Ok(GitAuthor { name, email })
}

pub fn branch_exists_on_remote(branch_name: &str) -> Result<bool> {
    let output = Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{branch_name}"),
        ])
        .output()
        .context("failed to check remote branch")?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    bail!(
        "failed to check remote branch: {}",
        stderr_or_stdout(&output)
    );
}

pub fn branch_exists_locally(branch_name: &str) -> Result<bool> {
    let output = Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch_name}"),
        ])
        .output()
        .context("failed to check local branch")?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    bail!("failed to check branch: {}", stderr_or_stdout(&output));
}

pub fn checkout_branch(reference: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["checkout", reference])
        .output()
        .context("failed to execute git checkout")?;
    if !output.status.success() {
        bail!("checkout failed: {}", stderr_or_stdout(&output));
    }
    Ok(())
}

pub fn validate_branch_name(branch_name: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["check-ref-format", "--branch", branch_name])
        .status()
        .context("failed to validate branch name")?;
    if !status.success() {
        bail!("invalid branch name \"{branch_name}\"");
    }
    Ok(())
}

pub fn fetch_and_checkout_remote_branch(branch_name: &str) -> Result<()> {
    validate_branch_name(branch_name)?;
    let refspec = format!("+refs/heads/{branch_name}:refs/remotes/origin/{branch_name}");
    let fetch_output =
        run_git_with_timeout(&["fetch", "origin", &refspec], Duration::from_secs(120))?;
    if !fetch_output.status.success() {
        bail!(
            "failed to fetch branch from origin: {}",
            stderr_or_stdout(&fetch_output)
        );
    }

    if !branch_exists_on_remote(branch_name)? {
        bail!("branch '{branch_name}' not found on origin");
    }

    let update_output = Command::new("git")
        .args([
            "update-ref",
            &format!("refs/heads/{branch_name}"),
            &format!("refs/remotes/origin/{branch_name}"),
        ])
        .output()
        .context("failed to create local branch")?;
    if !update_output.status.success() {
        bail!(
            "failed to create local branch: {}",
            stderr_or_stdout(&update_output)
        );
    }

    checkout_branch(branch_name)
}

pub fn fetch_metadata_branch() -> Result<()> {
    let branch_name = paths::METADATA_BRANCH_NAME;
    let refspec = format!("+refs/heads/{branch_name}:refs/remotes/origin/{branch_name}");
    let fetch_output =
        run_git_with_timeout(&["fetch", "origin", &refspec], Duration::from_secs(120))?;
    if !fetch_output.status.success() {
        bail!(
            "failed to fetch {branch_name} from origin: {}",
            stderr_or_stdout(&fetch_output)
        );
    }

    if !branch_exists_on_remote(branch_name)? {
        bail!("branch '{branch_name}' not found on origin");
    }

    let update_output = Command::new("git")
        .args([
            "update-ref",
            &format!("refs/heads/{branch_name}"),
            &format!("refs/remotes/origin/{branch_name}"),
        ])
        .output()
        .context("failed to create local metadata branch")?;
    if !update_output.status.success() {
        bail!(
            "failed to create local {branch_name} branch: {}",
            stderr_or_stdout(&update_output)
        );
    }
    Ok(())
}

pub fn is_on_default_branch() -> Result<(bool, String)> {
    let current = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .context("failed to get HEAD")?;
    if !current.status.success() {
        bail!("failed to get HEAD: {}", stderr_or_stdout(&current));
    }

    let current_branch = stdout_trimmed(&current).to_string();
    if current_branch == "HEAD" {
        return Ok((false, String::new()));
    }

    let default_branch = get_default_branch_name();
    if default_branch.is_empty() {
        return Ok((
            current_branch == "main" || current_branch == "master",
            current_branch,
        ));
    }

    Ok((current_branch == default_branch, current_branch))
}

pub fn should_skip_on_default_branch() -> (bool, String) {
    is_on_default_branch().unwrap_or((false, String::new()))
}

pub fn get_default_branch_name() -> String {
    let symbolic = Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD"])
        .output();
    if let Ok(output) = symbolic
        && output.status.success()
    {
        let target = stdout_trimmed(&output);
        if let Some(stripped) = target.strip_prefix("refs/remotes/origin/") {
            return stripped.to_string();
        }
    }

    for candidate in ["main", "master"] {
        let status = Command::new("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/remotes/origin/{candidate}"),
            ])
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return candidate.to_string();
        }
    }

    for candidate in ["main", "master"] {
        let status = Command::new("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{candidate}"),
            ])
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return candidate.to_string();
        }
    }

    String::new()
}

pub fn is_empty_repository() -> Result<bool> {
    run_git(&["rev-parse", "--git-dir"]).context("failed to open git repository")?;

    let output = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .context("failed to check repository HEAD")?;

    Ok(!output.status.success())
}

pub fn hard_reset_with_protection(commit_hash: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["reset", "--hard", commit_hash])
        .output()
        .context("failed to execute git reset --hard")?;

    if !output.status.success() {
        bail!("reset failed: {}", stderr_or_stdout(&output));
    }

    Ok(commit_hash.chars().take(7).collect())
}

fn run_git(args: &[&str]) -> Result<Output> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("failed to execute git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            stderr_or_stdout(&output)
        );
    }
    Ok(output)
}

fn run_git_with_timeout(args: &[&str], timeout: Duration) -> Result<Output> {
    let mut child = Command::new("git")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to execute git {}", args.join(" ")))?;

    let started_at = Instant::now();
    loop {
        if let Some(_status) = child.try_wait().context("failed while waiting for git")? {
            return child
                .wait_with_output()
                .context("failed to collect git output");
        }
        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("fetch timed out after 2 minutes");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn stdout_trimmed(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap_or("").trim()
}

fn stderr_or_stdout(output: &Output) -> String {
    let stderr = std::str::from_utf8(&output.stderr).unwrap_or("").trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    let stdout = std::str::from_utf8(&output.stdout).unwrap_or("").trim();
    if !stdout.is_empty() {
        return stdout.to_string();
    }
    anyhow!("git command failed").to_string()
}

#[cfg(test)]
#[path = "git_operations_tests.rs"]
mod tests;
