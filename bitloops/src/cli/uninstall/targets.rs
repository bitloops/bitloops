use std::collections::BTreeSet;
use std::io::Write;

use anyhow::{Result, anyhow, bail};
use clap::Args;

use super::{
    NO_FLAGS_ERROR, UninstallSelector, picker::prompt_select_targets, tty::can_prompt_interactively,
};

#[derive(Args, Debug, Clone, Default)]
pub struct UninstallArgs {
    /// Remove all Bitloops-managed artefacts.
    #[arg(
        long,
        default_value_t = false,
        conflicts_with_all = [
            "binaries",
            "service",
            "data",
            "caching",
            "config",
            "agent_hooks",
            "repo_config",
            "git_hooks",
            "shell",
        ]
    )]
    pub full: bool,

    /// Remove Bitloops binaries from recognised install locations.
    #[arg(long, default_value_t = false)]
    pub binaries: bool,

    /// Remove the Bitloops daemon service and state metadata.
    #[arg(long, default_value_t = false)]
    pub service: bool,

    /// Remove Bitloops data directories and repo-local `.bitloops/` data.
    #[arg(long, default_value_t = false)]
    pub data: bool,

    /// Remove Bitloops cache directories.
    #[arg(long, default_value_t = false)]
    pub caching: bool,

    /// Remove Bitloops config directories and TLS artefacts under `~/.bitloops/certs`.
    #[arg(long, default_value_t = false)]
    pub config: bool,

    /// Remove supported agent hooks and Bitloops-managed repo-local agent prompt surfaces.
    #[arg(long = "agent-hooks", default_value_t = false)]
    pub agent_hooks: bool,

    /// Remove repo-local Bitloops policy files and managed `.git/info/exclude` entries.
    #[arg(long = "repo-config", default_value_t = false)]
    pub repo_config: bool,

    /// Remove git hooks.
    #[arg(long = "git-hooks", default_value_t = false)]
    pub git_hooks: bool,

    /// Remove shell completion integration.
    #[arg(long, default_value_t = false)]
    pub shell: bool,

    /// Limit repo-local uninstall targets to the current repository or Bitloops project.
    #[arg(long = "only-current-project", default_value_t = false)]
    pub only_current_project: bool,

    /// Skip the confirmation prompt.
    #[arg(long, short = 'f', default_value_t = false)]
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum UninstallTarget {
    AgentHooks,
    RepoConfig,
    GitHooks,
    Shell,
    Data,
    Caching,
    Config,
    Service,
    Binaries,
}

impl UninstallTarget {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::AgentHooks => "Agent hooks",
            Self::RepoConfig => "Repo config",
            Self::GitHooks => "Git hooks",
            Self::Shell => "Shell integration",
            Self::Data => "Data",
            Self::Caching => "Caching",
            Self::Config => "Config",
            Self::Service => "Service",
            Self::Binaries => "Binaries",
        }
    }

    pub(super) fn picker_label(self) -> &'static str {
        match self {
            Self::AgentHooks => "Agent hooks in known Bitloops projects",
            Self::RepoConfig => {
                "Repo-local `.bitloops*.toml` and managed exclude entries in known Bitloops projects"
            }
            Self::GitHooks => "Git hooks in known repositories",
            Self::Shell => "Shell completion integration",
            Self::Data => "Global data and repo-local `.bitloops/` data",
            Self::Caching => "Global cache directories",
            Self::Config => "Global config and TLS artefacts",
            Self::Service => "Daemon service and state metadata",
            Self::Binaries => "Installed bitloops binaries",
        }
    }

    pub(super) fn summary(self, hook_repo_count: usize, repo_data_count: usize) -> String {
        match self {
            Self::AgentHooks => format!("Agent hooks in {hook_repo_count} Bitloops project(s)"),
            Self::RepoConfig => {
                format!("Repo config in {hook_repo_count} Bitloops project(s)")
            }
            Self::GitHooks => format!("Git hooks in {hook_repo_count} repo(s)"),
            Self::Shell => "Shell completion integration".to_string(),
            Self::Data => {
                format!("Global data directory and .bitloops dirs in {repo_data_count} repo(s)")
            }
            Self::Caching => "Global cache directory".to_string(),
            Self::Config => "Global config directory and TLS artefacts".to_string(),
            Self::Service => "Global daemon service and state metadata".to_string(),
            Self::Binaries => "Recognised bitloops binaries".to_string(),
        }
    }
}

pub(super) const ALL_TARGETS: [UninstallTarget; 9] = [
    UninstallTarget::AgentHooks,
    UninstallTarget::RepoConfig,
    UninstallTarget::GitHooks,
    UninstallTarget::Shell,
    UninstallTarget::Data,
    UninstallTarget::Caching,
    UninstallTarget::Config,
    UninstallTarget::Service,
    UninstallTarget::Binaries,
];

pub(super) fn collect_requested_targets(
    args: &UninstallArgs,
    out: &mut dyn Write,
    select_fn: Option<&UninstallSelector>,
) -> Result<Option<BTreeSet<UninstallTarget>>> {
    if args.full {
        return Ok(Some(ALL_TARGETS.into_iter().collect()));
    }

    let selected = targets_from_flags(args);
    if !selected.is_empty() {
        return Ok(Some(selected));
    }

    if let Some(select) = select_fn {
        let mut picked = select(&ALL_TARGETS).map_err(|err| anyhow!(err))?;
        picked.sort();
        picked.dedup();
        if picked.is_empty() {
            bail!("no uninstall targets selected");
        }
        return Ok(Some(picked.into_iter().collect()));
    }

    if !can_prompt_interactively() {
        bail!(NO_FLAGS_ERROR);
    }

    prompt_select_targets(out)
}

pub(super) fn validate_scope_flags(
    args: &UninstallArgs,
    targets: &BTreeSet<UninstallTarget>,
) -> Result<()> {
    if !args.only_current_project {
        return Ok(());
    }

    if targets.is_empty() {
        bail!(
            "`--only-current-project` requires `--agent-hooks`, `--repo-config`, and/or `--git-hooks`"
        );
    }

    if targets.iter().any(|target| {
        !matches!(
            target,
            UninstallTarget::AgentHooks | UninstallTarget::RepoConfig | UninstallTarget::GitHooks
        )
    }) {
        bail!(
            "`--only-current-project` can only be used with `--agent-hooks`, `--repo-config`, and/or `--git-hooks`"
        );
    }

    Ok(())
}

fn targets_from_flags(args: &UninstallArgs) -> BTreeSet<UninstallTarget> {
    let mut selected = BTreeSet::new();

    if args.binaries {
        selected.insert(UninstallTarget::Binaries);
    }
    if args.service {
        selected.insert(UninstallTarget::Service);
    }
    if args.data {
        selected.insert(UninstallTarget::Data);
    }
    if args.caching {
        selected.insert(UninstallTarget::Caching);
    }
    if args.config {
        selected.insert(UninstallTarget::Config);
    }
    if args.agent_hooks {
        selected.insert(UninstallTarget::AgentHooks);
    }
    if args.repo_config {
        selected.insert(UninstallTarget::RepoConfig);
    }
    if args.git_hooks {
        selected.insert(UninstallTarget::GitHooks);
    }
    if args.shell {
        selected.insert(UninstallTarget::Shell);
    }

    selected
}
