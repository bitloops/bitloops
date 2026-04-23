//! `bitloops enable` / `bitloops disable` command implementation.

use std::collections::BTreeSet;
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::adapters::agents::AgentAdapterRegistry;
#[cfg(test)]
use crate::adapters::agents::claude_code::git_hooks;
use crate::capability_packs::semantic_clones::workplane::activate_embedding_pipeline_mailboxes;
use crate::cli::embeddings::{
    EmbeddingsInstallState, EmbeddingsRuntime, enqueue_embeddings_bootstrap_task,
    inspect_embeddings_install_state, install_or_configure_platform_embeddings,
    platform_embeddings_gateway_url_override,
};
use crate::cli::inference::{
    SummarySetupSelection, configure_cloud_summary_generation, configure_local_summary_generation,
    platform_summary_gateway_url_override, prompt_summary_setup_selection,
    summary_generation_configured,
};
use crate::cli::root::DisableArgs;
use crate::cli::telemetry_consent;
use crate::cli::terminal_picker::{
    MultiSelectOption, can_use_terminal_picker, prompt_multi_select,
};
#[cfg(test)]
use crate::config::REPO_POLICY_FILE_NAME;
#[cfg(test)]
use crate::config::REPO_POLICY_LOCAL_FILE_NAME;
use crate::config::discover_repo_policy;
#[cfg(test)]
use crate::config::settings::BitloopsSettings;
use crate::config::settings::{
    self, SETTINGS_DIR, devql_guidance_enabled_from_policy, load_settings, set_capture_enabled,
    set_devql_guidance_enabled,
};
#[cfg(test)]
use crate::config::settings::{settings_local_path, settings_path};
use crate::host::checkpoints::session::create_session_backend_or_local;

#[derive(Args, Debug, Clone)]
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

    /// Deprecated hidden compatibility flag. Use `bitloops init --agent <agent>`
    /// to persist supported agents before running `bitloops enable`.
    #[arg(long, hide = true)]
    pub agent: Option<String>,

    /// Enable capture for this Bitloops project.
    #[arg(long, default_value_t = false)]
    pub capture: bool,

    /// Enable the repo-local DevQL guidance surface for configured agents.
    #[arg(long = "devql-guidance", default_value_t = false)]
    pub devql_guidance: bool,

    /// Enable anonymous telemetry for this CLI version.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub telemetry: Option<bool>,

    /// Disable anonymous telemetry for this CLI version.
    #[arg(
        long = "no-telemetry",
        conflicts_with = "telemetry",
        default_value_t = false
    )]
    pub no_telemetry: bool,

    /// Configure and bootstrap local embeddings so sync can include them.
    #[arg(long, default_value_t = false)]
    pub install_embeddings: bool,

    /// Select which managed embeddings runtime to configure when embeddings are installed.
    #[arg(long, value_enum)]
    pub embeddings_runtime: Option<EmbeddingsRuntime>,

    /// Public platform embeddings endpoint used when `--embeddings-runtime platform` is selected.
    #[arg(long)]
    pub embeddings_gateway_url: Option<String>,

    /// Environment variable that contains the platform gateway bearer token.
    #[arg(long)]
    pub embeddings_api_key_env: Option<String>,
}

const ENABLE_NO_FLAGS_ERROR: &str = "`bitloops enable` without flags requires an interactive terminal; pass explicit flags such as `--capture` or `--devql-guidance`";
const DISABLE_NO_FLAGS_ERROR: &str = "`bitloops disable` without flags requires an interactive terminal; pass explicit flags such as `--capture` or `--devql-guidance`";
const DEFAULT_EMBEDDINGS_API_KEY_ENV: &str = "BITLOOPS_PLATFORM_GATEWAY_TOKEN";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ToggleTarget {
    Capture,
    DevqlGuidance,
}

impl ToggleTarget {
    fn label(self) -> &'static str {
        match self {
            Self::Capture => "Capture",
            Self::DevqlGuidance => "DevQL Guidance",
        }
    }

    fn details(self) -> Vec<String> {
        match self {
            Self::Capture => vec![
                "Turns repository capture on or off while keeping managed hooks installed."
                    .to_string(),
            ],
            Self::DevqlGuidance => vec![
                "Controls the repo-local DevQL guidance surface used by Bitloops hook augmentation."
                    .to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ToggleState {
    capture_enabled: bool,
    devql_guidance_enabled: bool,
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

    let entry = relative_local_policy.as_str();
    if !content.lines().any(|line| line.trim() == entry) {
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

fn reconcile_repo_watcher(repo_root: &Path) {
    if let Err(err) = crate::cli::watcher_bootstrap::reconcile_repo_watcher(repo_root) {
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

fn requested_targets_from_enable_args(args: &EnableArgs) -> BTreeSet<ToggleTarget> {
    let mut targets = BTreeSet::new();
    if args.capture {
        targets.insert(ToggleTarget::Capture);
    }
    if args.devql_guidance {
        targets.insert(ToggleTarget::DevqlGuidance);
    }
    targets
}

fn requested_targets_from_disable_args(args: &DisableArgs) -> BTreeSet<ToggleTarget> {
    let mut targets = BTreeSet::new();
    if args.capture {
        targets.insert(ToggleTarget::Capture);
    }
    if args.devql_guidance {
        targets.insert(ToggleTarget::DevqlGuidance);
    }
    targets
}

fn map_selected_target_indexes(selected_indexes: Vec<usize>) -> BTreeSet<ToggleTarget> {
    selected_indexes
        .into_iter()
        .filter_map(|index| match index {
            0 => Some(ToggleTarget::Capture),
            1 => Some(ToggleTarget::DevqlGuidance),
            _ => None,
        })
        .collect()
}

fn prompt_enable_targets(
    out: &mut dyn Write,
    state: ToggleState,
) -> Result<Option<BTreeSet<ToggleTarget>>> {
    let options = vec![
        MultiSelectOption::new(
            ToggleTarget::Capture.label(),
            ToggleTarget::Capture.details(),
            state.capture_enabled,
        ),
        MultiSelectOption::new(
            ToggleTarget::DevqlGuidance.label(),
            ToggleTarget::DevqlGuidance.details(),
            state.devql_guidance_enabled,
        ),
    ];

    match prompt_multi_select(
        out,
        "Select what to enable:",
        &["Press enter to apply the current selection.".to_string()],
        &options,
        &["x toggle • ↑/↓ move • enter submit • ctrl+a all".to_string()],
    ) {
        Ok(selected_indexes) => Ok(Some(map_selected_target_indexes(selected_indexes))),
        Err(err) if err.to_string() == "cancelled by user" => Ok(None),
        Err(err) if err.to_string() == "no options selected" => Ok(Some(BTreeSet::new())),
        Err(err) => Err(err),
    }
}

fn prompt_disable_targets(out: &mut dyn Write) -> Result<Option<BTreeSet<ToggleTarget>>> {
    let options = vec![
        MultiSelectOption::new(
            ToggleTarget::Capture.label(),
            ToggleTarget::Capture.details(),
            false,
        ),
        MultiSelectOption::new(
            ToggleTarget::DevqlGuidance.label(),
            ToggleTarget::DevqlGuidance.details(),
            false,
        ),
    ];

    match prompt_multi_select(
        out,
        "Select what to disable:",
        &["Use space to select, enter to confirm.".to_string()],
        &options,
        &["x toggle • ↑/↓ move • enter submit • ctrl+a all".to_string()],
    ) {
        Ok(selected_indexes) => {
            let selected_targets = map_selected_target_indexes(selected_indexes);
            if selected_targets.is_empty() {
                bail!("no disable targets selected");
            }
            Ok(Some(selected_targets))
        }
        Err(err) if err.to_string() == "cancelled by user" => Ok(None),
        Err(err) if err.to_string() == "no options selected" => {
            bail!("no disable targets selected")
        }
        Err(err) => Err(err),
    }
}

fn collect_enable_targets(
    args: &EnableArgs,
    state: ToggleState,
    out: &mut dyn Write,
) -> Result<Option<BTreeSet<ToggleTarget>>> {
    let requested = requested_targets_from_enable_args(args);
    if !requested.is_empty() {
        return Ok(Some(requested));
    }
    if !can_use_terminal_picker() {
        bail!(ENABLE_NO_FLAGS_ERROR);
    }
    prompt_enable_targets(out, state)
}

fn collect_disable_targets(
    args: &DisableArgs,
    out: &mut dyn Write,
) -> Result<Option<BTreeSet<ToggleTarget>>> {
    let requested = requested_targets_from_disable_args(args);
    if !requested.is_empty() {
        return Ok(Some(requested));
    }
    if !can_use_terminal_picker() {
        bail!(DISABLE_NO_FLAGS_ERROR);
    }
    prompt_disable_targets(out)
}

fn enable_uses_embeddings_flags(args: &EnableArgs) -> bool {
    args.install_embeddings
        || args.embeddings_runtime.is_some()
        || args.embeddings_gateway_url.is_some()
        || args.embeddings_api_key_env.is_some()
}

/// Main handler for `bitloops enable`.
pub async fn run(args: EnableArgs) -> Result<()> {
    if args.local && args.project {
        bail!("cannot use both --local and --project flags");
    }

    let mut out = io::stdout().lock();
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    run_with_io(args, &mut out, &mut input).await
}

pub(crate) async fn run_with_io(
    args: EnableArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<()> {
    if let Some(agent) = args.agent.as_deref() {
        bail!(
            "`bitloops enable --agent {agent}` is no longer supported. \
Run `bitloops init --agent {agent}` to persist supported agents before enabling Bitloops."
        );
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
    let current_state = ToggleState {
        capture_enabled: load_settings(&cwd).unwrap_or_default().enabled,
        devql_guidance_enabled: devql_guidance_enabled_from_policy(&policy)?,
    };
    let Some(targets) = collect_enable_targets(&args, current_state, out)? else {
        writeln!(out, "Enable cancelled.")?;
        return Ok(());
    };
    if targets.is_empty() {
        writeln!(out, "No enable targets selected; nothing to do.")?;
        return Ok(());
    }

    let capture_selected = targets.contains(&ToggleTarget::Capture);
    let devql_guidance_selected = targets.contains(&ToggleTarget::DevqlGuidance);
    if enable_uses_embeddings_flags(&args) && !capture_selected {
        bail!(
            "`--install-embeddings`, `--embeddings-runtime`, `--embeddings-gateway-url`, and `--embeddings-api-key-env` require `--capture`"
        );
    }

    let telemetry_choice =
        telemetry_consent::telemetry_flag_choice(args.telemetry, args.no_telemetry);
    if (capture_selected && !current_state.capture_enabled) || telemetry_choice.is_some() {
        telemetry_consent::ensure_default_daemon_running().await?;
        telemetry_consent::ensure_existing_config_telemetry_consent(
            cwd.as_path(),
            telemetry_choice,
            out,
            input,
        )
        .await?;
    }

    let project_root = policy
        .root
        .clone()
        .context("resolving Bitloops project root from repo policy")?;
    let target_path = policy
        .local_path
        .clone()
        .or(policy.shared_path.clone())
        .context("resolving editable Bitloops project config")?;
    let selected_agents = crate::cli::agent_surfaces::configured_agents_or_bail(&cwd)?;
    let settings = load_settings(&cwd).unwrap_or_default();

    let final_devql_guidance_enabled =
        devql_guidance_selected || current_state.devql_guidance_enabled;

    if capture_selected {
        let git_count = crate::adapters::agents::claude_code::git_hooks::install_git_hooks(
            &git_root,
            settings.local_dev,
        )?;
        if git_count > 0 {
            writeln!(out, "Installed {git_count} git hook(s).")?;
        }
        crate::cli::agent_surfaces::reconcile_project_agent_surfaces_with_options(
            &project_root,
            &selected_agents,
            settings.local_dev,
            args.force,
            crate::cli::agent_surfaces::ReconcileProjectAgentSurfacesOptions {
                install_bitloops_skill: final_devql_guidance_enabled,
            },
            out,
        )?;
        set_capture_enabled(&target_path, true)?;
        reconcile_repo_watcher(&git_root);

        writeln!(out, "Bitloops enabled in this project! :)")?;
        writeln!(out, "Strategy: {}.", settings.strategy)?;
        writeln!(out, "Updated project config: {}", target_path.display())?;
    }

    if devql_guidance_selected {
        set_devql_guidance_enabled(&target_path, true)?;
        if !capture_selected {
            crate::cli::agent_surfaces::install_project_prompt_surfaces(
                &project_root,
                &selected_agents,
                out,
            )?;
            writeln!(
                out,
                "DevQL guidance enabled in this project ({})",
                target_path.display()
            )?;
        }
    }

    let embeddings_runtime = args.embeddings_runtime.unwrap_or(EmbeddingsRuntime::Local);
    let embeddings_api_key_env = args
        .embeddings_api_key_env
        .as_deref()
        .unwrap_or(DEFAULT_EMBEDDINGS_API_KEY_ENV);
    let capture_was_disabled = !current_state.capture_enabled;

    if capture_selected
        && (capture_was_disabled || args.install_embeddings)
        && should_install_embeddings(&cwd, args.install_embeddings, out, input)?
    {
        match embeddings_runtime {
            EmbeddingsRuntime::Local => {
                activate_embedding_pipeline_mailboxes(&git_root, "enable")
                    .context("activating semantic clones embedding mailboxes for enable")?;
                let (scope, queued) = enqueue_embeddings_bootstrap_task(
                    &cwd,
                    None,
                    crate::daemon::DevqlTaskSource::ManualCli,
                )
                .await?;
                match crate::cli::devql::graphql::watch_task_id_via_graphql(
                    &scope,
                    &queued.task.task_id,
                    false,
                )
                .await
                {
                    Ok(Some(task)) => {
                        writeln!(
                            out,
                            "{}",
                            crate::cli::devql::format_task_completion_summary(&task)
                        )?;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        bail!(
                            "Bitloops capture was enabled, but embeddings installation failed: {err:#}"
                        );
                    }
                }
            }
            EmbeddingsRuntime::Platform => {
                let gateway_url = platform_embeddings_gateway_url_override(
                    args.embeddings_gateway_url.as_deref(),
                );
                for line in install_or_configure_platform_embeddings(
                    &cwd,
                    gateway_url.as_deref(),
                    embeddings_api_key_env,
                )? {
                    writeln!(out, "{line}")?;
                }
            }
        }
    }

    if !capture_selected || !capture_was_disabled {
        return Ok(());
    }

    match choose_summary_setup(&cwd, out, input).await? {
        SummarySetupSelection::Cloud => {
            crate::cli::login::ensure_logged_in().await?;
            let gateway_url_override = platform_summary_gateway_url_override();
            let message = configure_cloud_summary_generation(&cwd, gateway_url_override.as_deref())
                .map_err(|err| {
                    anyhow::anyhow!(
                        "Bitloops capture was enabled, but semantic summary setup failed: {err:#}"
                    )
                })?;
            writeln!(out, "{message}")?;
        }
        SummarySetupSelection::Local => {
            configure_local_summary_generation(&cwd, out, input, true).map_err(|err| {
                anyhow::anyhow!(
                    "Bitloops capture was enabled, but semantic summary setup failed: {err:#}"
                )
            })?;
        }
        SummarySetupSelection::Skip => {}
    }
    Ok(())
}

fn should_install_embeddings(
    repo_root: &Path,
    explicit_install: bool,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<bool> {
    if explicit_install {
        return Ok(true);
    }

    if !telemetry_consent::can_prompt_interactively() {
        return Ok(false);
    }

    if !matches!(
        inspect_embeddings_install_state(repo_root),
        EmbeddingsInstallState::NotConfigured
    ) {
        return Ok(false);
    }

    prompt_install_embeddings(out, input)
}

fn prompt_install_embeddings(out: &mut dyn Write, input: &mut dyn BufRead) -> Result<bool> {
    writeln!(out)?;
    writeln!(
        out,
        "Install local embeddings as well? This is recommended and lets sync include embeddings."
    )?;

    loop {
        write!(out, "Install embeddings now? [Y/n] ")?;
        out.flush()?;

        let mut line = String::new();
        input
            .read_line(&mut line)
            .context("reading embeddings install prompt response")?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(out, "Please answer yes or no.")?,
        }
    }
}

async fn choose_summary_setup(
    repo_root: &Path,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<SummarySetupSelection> {
    if !telemetry_consent::can_prompt_interactively() {
        return Ok(SummarySetupSelection::Skip);
    }

    if summary_generation_configured(repo_root) {
        return Ok(SummarySetupSelection::Skip);
    }

    let cloud_logged_in = crate::daemon::resolve_workos_session_status()
        .await?
        .is_some();

    prompt_summary_setup_selection(out, input, true, false, cloud_logged_in)
}

pub fn initialized_agents(repo_root: &Path) -> Vec<String> {
    AgentAdapterRegistry::builtin().installed_agents(repo_root)
}

// ── internal helpers used by tests ──────────────────────────────────────────

pub fn run_disable_with_args(start: &Path, out: &mut dyn Write, args: &DisableArgs) -> Result<()> {
    if args.project {
        eprintln!(
            "Note: `--project` is deprecated and ignored. \
`bitloops disable` updates the nearest discovered project policy file."
        );
    }

    let policy = discover_repo_policy(start)?;
    let Some(targets) = collect_disable_targets(args, out)? else {
        writeln!(out, "Disable cancelled.")?;
        return Ok(());
    };
    let capture_selected = targets.contains(&ToggleTarget::Capture);
    let devql_guidance_selected = targets.contains(&ToggleTarget::DevqlGuidance);
    let project_root = policy
        .root
        .clone()
        .context("resolving Bitloops project root from repo policy")?;
    let target_path = policy
        .local_path
        .clone()
        .or(policy.shared_path.clone())
        .context("resolving editable Bitloops project config")?;
    if capture_selected {
        set_capture_enabled(&target_path, false)?;
        let repo_root = find_repo_root(start)?;
        reconcile_repo_watcher(&repo_root);
        writeln!(
            out,
            "Bitloops capture is now disabled for this project ({})",
            target_path.display()
        )?;
    }
    if devql_guidance_selected {
        let configured_agents = crate::config::settings::supported_agents(start)?;
        set_devql_guidance_enabled(&target_path, false)?;
        crate::cli::agent_surfaces::remove_project_prompt_surfaces(
            &project_root,
            &configured_agents,
            out,
        )?;
        writeln!(
            out,
            "DevQL guidance is now disabled for this project ({})",
            target_path.display()
        )?;
    }
    Ok(())
}

/// Sets `enabled = false` in the nearest project policy. This helper preserves
/// the historical capture-only behavior used by older tests.
pub fn run_disable(start: &Path, out: &mut dyn Write, use_project_settings: bool) -> Result<()> {
    run_disable_with_args(
        start,
        out,
        &DisableArgs {
            project: use_project_settings,
            capture: true,
            devql_guidance: false,
        },
    )
}

/// Returns `true` (is disabled) and prints a message when Bitloops is disabled.
/// Returns `false` when enabled (default when no settings file).
pub fn check_disabled_guard(start: &Path, out: &mut dyn Write) -> bool {
    match settings::is_enabled(start) {
        Ok(true) | Err(_) => false,
        Ok(false) => {
            let _ = writeln!(
                out,
                "Bitloops is disabled. Run `bitloops enable --capture` to re-enable capture."
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
