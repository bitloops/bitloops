use clap::{Args, ValueEnum};

#[derive(Args, Debug, Clone, Default)]
pub struct CleanArgs {
    /// Actually delete items (default: dry run).
    #[arg(long, short = 'f', default_value_t = false)]
    pub force: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DisableArgs {
    /// Deprecated: the nearest discovered project policy is edited automatically.
    #[arg(long, default_value_t = false)]
    pub project: bool,

    /// Disable capture for this Bitloops project.
    #[arg(long, default_value_t = false)]
    pub capture: bool,

    /// Disable the repo-local DevQL guidance surface for configured agents.
    #[arg(long = "devql-guidance", default_value_t = false)]
    pub devql_guidance: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DoctorArgs {
    /// Fix all stuck sessions without prompting.
    #[arg(long, short = 'f', default_value_t = false)]
    pub force: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct HelpArgs {
    /// Show full command tree.
    #[arg(short = 't', long = "tree", hide = true, default_value_t = false)]
    pub tree: bool,

    /// Optional target command path.
    #[arg(value_name = "command")]
    pub command: Vec<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ResetArgs {
    /// Skip confirmation prompt and active-session guard.
    #[arg(long, short = 'f', default_value_t = false)]
    pub force: bool,

    /// Reset a specific session by ID.
    #[arg(long)]
    pub session: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ResumeArgs {
    /// Branch to switch to before resume logic.
    pub branch: String,

    /// Resume from older checkpoint without confirmation.
    #[arg(long, short = 'f', default_value_t = false)]
    pub force: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Args, Debug, Clone)]
pub struct CompletionArgs {
    #[arg(value_enum)]
    pub shell: CompletionShell,
}

#[derive(Args, Debug, Clone, Default)]
pub struct VersionArgs {
    /// Check for updates now.
    #[arg(long, default_value_t = false)]
    pub check: bool,
}
