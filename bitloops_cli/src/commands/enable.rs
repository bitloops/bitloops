//! `bitloops enable` / `bitloops disable` command implementation.

use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::config::settings::{
    self, BitloopsSettings, SETTINGS_DIR, SETTINGS_LOCAL_FILE, load_settings, save_settings,
    settings_local_path, settings_path,
};
use crate::engine::agent::AgentAdapterRegistry;
use crate::engine::agent::claude_code::git_hooks;
use crate::engine::session::create_session_backend_or_local;

#[derive(Args)]
pub struct EnableArgs {
    /// Write settings to .bitloops/settings.local.json (user-local, gitignored)
    #[arg(long)]
    pub local: bool,

    /// Force write to .bitloops/settings.json even if it already exists
    #[arg(long)]
    pub project: bool,

    /// Deprecated: no-op. Use `bitloops init` to initialize agents.
    #[arg(long, short = 'f', hide = true)]
    pub force: bool,

    /// Deprecated: no-op. Use `bitloops init --agent <name>` to initialize agents.
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

/// Entries that must be present in `.bitloops/.gitignore`.
const GITIGNORE_ENTRIES: &[&str] = &[
    "tmp/",
    SETTINGS_LOCAL_FILE,
    "metadata/",
    "logs/",
    "stores/",
    "embeddings/",
];

/// Ensures the `.bitloops/` directory and its `.gitignore` exist.
fn setup_bitloops_dir(repo_root: &Path) -> Result<()> {
    let dir = repo_root.join(SETTINGS_DIR);
    fs::create_dir_all(&dir).with_context(|| format!("creating {SETTINGS_DIR}/ directory"))?;

    let gitignore = dir.join(".gitignore");
    let mut content = fs::read_to_string(&gitignore).unwrap_or_default();

    let mut changed = false;
    for entry in GITIGNORE_ENTRIES {
        if !content.contains(entry) {
            if !content.ends_with('\n') && !content.is_empty() {
                content.push('\n');
            }
            content.push_str(entry);
            content.push('\n');
            changed = true;
        }
    }

    if changed || !gitignore.exists() {
        fs::write(&gitignore, &content).context("writing .bitloops/.gitignore")?;
    }

    Ok(())
}

/// Determines which settings file to write to.
///
/// Returns `(path, show_notification)`:
/// - `--local`  → local file, no notification
/// - `--project` → project file, no notification
/// - neither + settings.json exists → local file, show notification
/// - neither + no settings.json → project file, no notification
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
    let repo_root = find_repo_root(&cwd)?;

    setup_bitloops_dir(&repo_root)?;

    let mut merged = load_settings(&repo_root).unwrap_or_default();
    merged.strategy = settings::DEFAULT_STRATEGY.to_string();
    merged.enabled = true;

    let (target_path, show_notification) =
        determine_settings_target(&repo_root, args.local, args.project);

    save_settings(&merged, &target_path)?;

    if show_notification {
        eprintln!(
            "Note: writing settings to {} (project settings.json already exists)",
            target_path.display()
        );
    }

    if args.agent.is_some() || args.force {
        eprintln!(
            "Note: agent initialization moved to `bitloops init`; `bitloops enable` no longer initializes agents."
        );
    }

    let git_count = git_hooks::install_git_hooks(&repo_root, merged.local_dev)?;
    if git_count > 0 {
        println!("Installed {git_count} git hook(s).");
    }

    let agents = initialized_agents(&repo_root);
    if agents.is_empty() {
        println!("Bitloops enabled, but no agents are initialized.");
        println!("Run `bitloops init` to initialize agent integrations.");
    } else {
        println!("Initialized agents: {}", agents.join(", "));
    }

    println!("Bitloops is enabled (strategy: manual-commit).");
    Ok(())
}

pub fn initialized_agents(repo_root: &Path) -> Vec<String> {
    AgentAdapterRegistry::builtin().installed_agents(repo_root)
}

// ── internal helpers used by tests ──────────────────────────────────────────

/// Sets `enabled = false` and writes to the appropriate file.
pub fn run_disable(
    repo_root: &Path,
    out: &mut dyn Write,
    use_project_settings: bool,
) -> Result<()> {
    if use_project_settings {
        let path = settings_path(repo_root);
        let mut settings = load_from_file_or_default(&path);
        settings.enabled = false;
        save_settings(&settings, &path)?;
    } else {
        // Write to local settings file (creates it if absent).
        let local_path = settings_local_path(repo_root);
        let mut settings = load_from_file_or_default(&local_path);
        settings.enabled = false;
        save_settings(&settings, &local_path)?;
    }
    writeln!(out, "Bitloops is now disabled.")?;
    Ok(())
}

/// Returns `true` (is disabled) and prints a message when Bitloops is disabled.
/// Returns `false` when enabled (default when no settings file).
pub fn check_disabled_guard(repo_root: &Path, out: &mut dyn Write) -> bool {
    match settings::is_enabled(repo_root) {
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

fn load_from_file_or_default(path: &Path) -> BitloopsSettings {
    if path.exists() {
        match fs::read(path) {
            Ok(data) => serde_json::from_slice(&data).unwrap_or_default(),
            Err(_) => BitloopsSettings::default(),
        }
    } else {
        BitloopsSettings::default()
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
    setup_bitloops_dir(repo_root)?;
    let mut merged = load_settings(repo_root).unwrap_or_default();
    merged.strategy = selected_strategy.to_string();
    merged.enabled = true;
    let (target_path, _) =
        determine_settings_target(repo_root, use_local_settings, use_project_settings);
    save_settings(&merged, &target_path)?;
    Ok(target_path)
}

pub fn run_uninstall(
    repo_root: &Path,
    out: &mut dyn Write,
    err_out: &mut dyn Write,
    force: bool,
) -> Result<()> {
    if !repo_root.join(".git").exists() {
        writeln!(err_out, "Not a git repository. Nothing to uninstall.")?;
        return Err(crate::commands::SilentError.into());
    }

    let bitloops_dir_exists = check_bitloops_dir_exists(repo_root);
    let session_state_count = count_session_states(repo_root);
    let git_hooks_installed = git_hooks::is_git_hook_installed(repo_root);
    let installed_agents = AgentAdapterRegistry::builtin().installed_agents(repo_root);
    let installed_agent_labels = installed_agents
        .iter()
        .map(|agent| {
            AgentAdapterRegistry::builtin()
                .agent_display(agent)
                .unwrap_or("Unknown")
                .to_string()
        })
        .collect::<Vec<_>>();

    if !bitloops_dir_exists
        && !git_hooks_installed
        && session_state_count == 0
        && installed_agents.is_empty()
    {
        writeln!(out, "Bitloops is not installed in this repository.")?;
        return Ok(());
    }

    if !force {
        writeln!(
            out,
            "\nThis will completely remove Bitloops from this repository:"
        )?;
        if bitloops_dir_exists {
            writeln!(out, "  - .bitloops/ directory")?;
        }
        if git_hooks_installed {
            writeln!(
                out,
                "  - Git hooks (prepare-commit-msg, commit-msg, post-commit, pre-push)"
            )?;
        }
        if session_state_count > 0 {
            writeln!(out, "  - Session state files ({session_state_count})")?;
        }
        if !installed_agent_labels.is_empty() {
            writeln!(
                out,
                "  - Agent hooks ({})",
                installed_agent_labels.join(", ")
            )?;
        }

        write!(
            out,
            "\nAre you sure you want to uninstall Bitloops? [y/N]: "
        )?;
        out.flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let confirmed = matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        if !confirmed {
            writeln!(out, "Uninstall cancelled.")?;
            return Ok(());
        }
    }

    writeln!(out, "\nUninstalling Bitloops CLI...")?;

    if let Err(err) = remove_agent_hooks(repo_root, out) {
        writeln!(err_out, "Warning: failed to remove agent hooks: {err}")?;
    }

    let removed = git_hooks::uninstall_git_hooks(repo_root).unwrap_or(0);
    if removed > 0 {
        writeln!(out, "  Removed git hooks ({removed})")?;
    }

    let states_removed = remove_all_session_states(repo_root).unwrap_or(0);
    if states_removed > 0 {
        writeln!(out, "  Removed session states ({states_removed})")?;
    }

    if bitloops_dir_exists {
        remove_bitloops_directory(repo_root)?;
        writeln!(out, "  Removed .bitloops directory")?;
    }

    writeln!(out, "\nBitloops CLI uninstalled successfully.")?;
    Ok(())
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

fn remove_all_session_states(repo_root: &Path) -> Result<usize> {
    let backend = create_session_backend_or_local(repo_root);
    let sessions = backend.list_sessions().context("listing session states")?;

    let mut removed = 0usize;
    for session in sessions {
        let session_id = session.session_id;
        backend
            .delete_session(&session_id)
            .with_context(|| format!("removing session state {}", session_id))?;
        removed += 1;
    }

    Ok(removed)
}

fn remove_agent_hooks(repo_root: &Path, out: &mut dyn Write) -> Result<()> {
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
        "settings.local.json"
    } else {
        "settings.json"
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
