//! `bitloops enable` / `bitloops disable` command implementation.

use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::adapters::agents::AgentAdapterRegistry;
#[cfg(test)]
use crate::adapters::agents::claude_code::git_hooks;
#[cfg(test)]
use crate::config::REPO_POLICY_FILE_NAME;
#[cfg(test)]
use crate::config::REPO_POLICY_LOCAL_FILE_NAME;
use crate::config::discover_repo_policy;
#[cfg(test)]
use crate::config::settings::BitloopsSettings;
use crate::config::settings::{self, SETTINGS_DIR, load_settings, set_capture_enabled};
#[cfg(test)]
use crate::config::settings::{settings_local_path, settings_path};
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::devql::watch;

#[derive(Args)]
pub struct EnableArgs {
    /// Deprecated: the nearest discovered project policy is edited automatically.
    #[arg(long)]
    pub local: bool,

    /// Deprecated: the nearest discovered project policy is edited automatically.
    #[arg(long)]
    pub project: bool,

    /// Remove and reinstall existing hooks for selected agents.
    #[arg(long, short = 'f', hide = true)]
    pub force: bool,

    /// Target a specific agent setup (claude-code|copilot|cursor|gemini|opencode).
    #[arg(long, hide = true)]
    pub agent: Option<String>,
}

/// Finds the git repository root by walking up from `start`.
pub fn find_repo_root(start: &Path) -> Result<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".git").exists() {
            return Ok(dir);
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => bail!("not inside a git repository (no .git directory found)"),
        }
    }
}

#[cfg(test)]
fn ensure_repo_local_policy_excluded(git_root: &Path, project_root: &Path) -> Result<()> {
    use crate::config::REPO_POLICY_LOCAL_FILE_NAME;

    let exclude_path = git_root.join(".git").join("info").join("exclude");
    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating git exclude directory {}", parent.display()))?;
    }

    let mut content = fs::read_to_string(&exclude_path).unwrap_or_default();
    let relative_local_policy = project_root
        .strip_prefix(git_root)
        .unwrap_or(project_root)
        .join(REPO_POLICY_LOCAL_FILE_NAME);
    let relative_local_policy = relative_local_policy.to_string_lossy().replace('\\', "/");

    for entry in [relative_local_policy.as_str(), ".bitloops/"] {
        if content.lines().any(|line| line.trim() == entry) {
            continue;
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(entry);
        content.push('\n');
    }
    fs::write(&exclude_path, content)
        .with_context(|| format!("writing {}", exclude_path.display()))?;

    Ok(())
}

fn restart_watcher_if_running(repo_root: &Path, config_root: &Path) {
    if let Err(err) = watch::restart_watcher(repo_root, config_root) {
        log::debug!("skipping watcher restart after policy change: {err:#}");
    }
}

#[cfg(test)]
fn setup_bitloops_dir(repo_root: &Path) -> Result<()> {
    fs::create_dir_all(repo_root.join(SETTINGS_DIR))
        .with_context(|| format!("creating {SETTINGS_DIR}/ directory"))?;
    Ok(())
}

#[cfg(test)]
fn determine_settings_target(
    repo_root: &Path,
    use_local: bool,
    use_project: bool,
) -> (PathBuf, bool) {
    if use_local {
        return (settings_local_path(repo_root), false);
    }
    if use_project {
        return (settings_path(repo_root), false);
    }
    if settings_path(repo_root).exists() {
        (settings_local_path(repo_root), true)
    } else {
        (settings_path(repo_root), false)
    }
}

/// Main handler for `bitloops enable`.
pub async fn run(args: EnableArgs) -> Result<()> {
    if args.local && args.project {
        bail!("cannot use both --local and --project flags");
    }

    let cwd = env::current_dir().context("getting current directory")?;
    let git_root = find_repo_root(&cwd)?;

    if args.local || args.project {
        eprintln!(
            "Note: `--local` and `--project` are deprecated and ignored. \
`bitloops enable` updates the nearest discovered project policy file."
        );
    }

    let policy = discover_repo_policy(&cwd)?;
    let target_path = policy
        .local_path
        .clone()
        .or(policy.shared_path.clone())
        .context("resolving editable Bitloops project config")?;
    let policy_root = policy.root.unwrap_or_else(|| cwd.clone());
    set_capture_enabled(&target_path, true)?;
    let settings = load_settings(&cwd).unwrap_or_default();
    restart_watcher_if_running(&git_root, &policy_root);

    println!("Bitloops enabled in this project! :)");
    println!("Strategy: {}.", settings.strategy);
    println!("Updated project config: {}", target_path.display());
    Ok(())
}

pub fn initialized_agents(repo_root: &Path) -> Vec<String> {
    AgentAdapterRegistry::builtin().installed_agents(repo_root)
}

// ── internal helpers used by tests ──────────────────────────────────────────

/// Sets `enabled = false` and writes to the appropriate file.
pub fn run_disable(start: &Path, out: &mut dyn Write, use_project_settings: bool) -> Result<()> {
    let _ = use_project_settings;
    let policy = discover_repo_policy(start)?;
    let target_path = policy
        .local_path
        .clone()
        .or(policy.shared_path.clone())
        .context("resolving editable Bitloops project config")?;
    let policy_root = policy.root.unwrap_or_else(|| start.to_path_buf());
    set_capture_enabled(&target_path, false)?;
    let repo_root = find_repo_root(start)?;
    restart_watcher_if_running(&repo_root, &policy_root);
    writeln!(
        out,
        "Bitloops capture is now disabled for this project ({})",
        target_path.display()
    )?;
    Ok(())
}

/// Returns `true` (is disabled) and prints a message when Bitloops is disabled.
/// Returns `false` when enabled (default when no settings file).
pub fn check_disabled_guard(start: &Path, out: &mut dyn Write) -> bool {
    match settings::is_enabled(start) {
        Ok(true) | Err(_) => false,
        Ok(false) => {
            let _ = writeln!(
                out,
                "Bitloops is disabled. Run `bitloops enable` to re-enable."
            );
            true
        }
    }
}

pub const SHELL_COMPLETION_COMMENT: &str = "# Bitloops CLI shell completion";

#[cfg(test)]
pub fn run_enable_with_strategy(
    repo_root: &Path,
    selected_strategy: &str,
    use_local_settings: bool,
    use_project_settings: bool,
) -> Result<PathBuf> {
    let _ = use_local_settings;
    let _ = use_project_settings;
    let target_path = settings_path(repo_root);
    let settings = BitloopsSettings {
        strategy: selected_strategy.to_string(),
        enabled: true,
        ..BitloopsSettings::default()
    };
    crate::config::settings::save_settings(&settings, &target_path)?;
    Ok(target_path)
}

pub fn count_session_states(repo_root: &Path) -> usize {
    let backend = create_session_backend_or_local(repo_root);
    backend.list_sessions().map_or(0, |sessions| sessions.len())
}

#[cfg(test)]
pub fn count_shadow_branches(repo_root: &Path) -> usize {
    let _ = repo_root;
    0
}

pub(crate) fn remove_agent_hooks(repo_root: &Path, out: &mut dyn Write) -> Result<()> {
    let registry = AgentAdapterRegistry::builtin();
    for agent in registry.available_agents() {
        if registry.are_agent_hooks_installed(repo_root, &agent)? {
            let label = registry.uninstall_agent_hooks(repo_root, &agent)?;
            writeln!(out, "  Removed {label} hooks")?;
        }
    }

    Ok(())
}

pub fn check_bitloops_dir_exists(repo_root: &Path) -> bool {
    repo_root.join(SETTINGS_DIR).exists()
}

#[cfg(test)]
pub fn is_fully_enabled(repo_root: &Path) -> (bool, String, String) {
    let enabled = settings::is_enabled(repo_root).unwrap_or(false);
    if !enabled {
        return (false, String::new(), String::new());
    }
    if !check_bitloops_dir_exists(repo_root) {
        return (false, String::new(), String::new());
    }
    if !git_hooks::is_git_hook_installed(repo_root) {
        return (false, String::new(), String::new());
    }
    let registry = AgentAdapterRegistry::builtin();
    let enabled_agents = registry.installed_agents(repo_root);
    if enabled_agents.is_empty() {
        return (false, String::new(), String::new());
    }
    let config = if settings_local_path(repo_root).exists() {
        REPO_POLICY_LOCAL_FILE_NAME
    } else {
        REPO_POLICY_FILE_NAME
    };
    let agent = registry
        .agent_display(&enabled_agents[0])
        .unwrap_or("Unknown");
    (true, agent.to_string(), config.to_string())
}

pub fn remove_bitloops_directory(repo_root: &Path) -> Result<()> {
    let bitloops_dir = repo_root.join(SETTINGS_DIR);
    if !bitloops_dir.exists() {
        return Ok(());
    }
    fs::remove_dir_all(&bitloops_dir).context("removing .bitloops directory")
}

#[cfg(test)]
fn load_from_file_or_default(path: &Path) -> BitloopsSettings {
    let Some(repo_root) = path.parent() else {
        return BitloopsSettings::default();
    };
    settings::load_settings(repo_root).unwrap_or_default()
}

pub fn shell_completion_target(home: &Path) -> Result<(String, PathBuf, String)> {
    let shell = env::var("SHELL").unwrap_or_default();
    if shell.contains("zsh") {
        return Ok((
            "Zsh".to_string(),
            home.join(".zshrc"),
            "autoload -Uz compinit && compinit && source <(bitloops completion zsh)".to_string(),
        ));
    }
    if shell.contains("bash") {
        let mut rc = home.join(".bashrc");
        if home.join(".bash_profile").exists() {
            rc = home.join(".bash_profile");
        }
        return Ok((
            "Bash".to_string(),
            rc,
            "source <(bitloops completion bash)".to_string(),
        ));
    }
    if shell.contains("fish") {
        return Ok((
            "Fish".to_string(),
            home.join(".config").join("fish").join("config.fish"),
            "bitloops completion fish | source".to_string(),
        ));
    }
    bail!("unsupported shell")
}

pub fn append_shell_completion(rc_file: &Path, completion_line: &str) -> Result<()> {
    if let Some(parent) = rc_file.parent() {
        fs::create_dir_all(parent).context("creating shell rc directory")?;
    }
    let mut f = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(rc_file)
        .with_context(|| format!("opening {}", rc_file.display()))?;
    writeln!(f)?;
    writeln!(f, "{SHELL_COMPLETION_COMMENT}")?;
    writeln!(f, "{completion_line}")?;
    Ok(())
}

fn is_completion_configured(rc_file: &Path) -> bool {
    fs::read_to_string(rc_file)
        .map(|content| content.contains("bitloops completion"))
        .unwrap_or(false)
}

fn prompt_enable_shell_completion(
    w: &mut dyn Write,
    input: &mut dyn BufRead,
    shell_name: &str,
) -> Result<bool> {
    write!(
        w,
        "Enable shell completion? (detected: {shell_name}) [y/N]: "
    )?;
    w.flush()?;

    let mut line = String::new();
    let read = input
        .read_line(&mut line)
        .context("reading shell completion prompt response")?;
    if read == 0 {
        return Ok(false);
    }

    let answer = line.trim().to_ascii_lowercase();
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

pub(crate) fn run_post_install_shell_completion_with_io(
    w: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<()> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;

    let (shell_name, rc_file, completion_line) = match shell_completion_target(&home) {
        Ok(target) => target,
        Err(err) if err.to_string().contains("unsupported shell") => {
            writeln!(
                w,
                "Note: Shell completion not available for your shell. Supported: zsh, bash, fish."
            )?;
            return Ok(());
        }
        Err(err) => return Err(err),
    };

    if is_completion_configured(&rc_file) {
        writeln!(
            w,
            "✓ Shell completion already configured in {}",
            rc_file.display()
        )?;
        return Ok(());
    }

    if !prompt_enable_shell_completion(w, input, &shell_name)? {
        return Ok(());
    }

    append_shell_completion(&rc_file, &completion_line)
        .with_context(|| format!("failed to update {}", rc_file.display()))?;
    writeln!(w, "✓ Shell completion added to {}", rc_file.display())?;
    writeln!(w, "  Restart your shell to activate")?;
    Ok(())
}

pub fn run_post_install_shell_completion(w: &mut dyn Write) -> Result<()> {
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        writeln!(
            w,
            "Note: Shell completion setup skipped: non-interactive environment."
        )?;
        return Ok(());
    }

    let mut input = BufReader::new(stdin.lock());
    run_post_install_shell_completion_with_io(w, &mut input)
}

#[cfg(test)]
#[path = "enable_tests.rs"]
mod tests;
