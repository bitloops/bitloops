use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::cli::embeddings::EmbeddingsRuntime;

pub(crate) const DEFAULT_INIT_INGEST_BACKFILL: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SummariesRuntime {
    Local,
    Platform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SummaryEmbeddingsMode {
    On,
    Off,
}

#[derive(Subcommand, Debug, Clone)]
pub enum InitCommand {
    /// Show init status for the current repository.
    Status(InitStatusArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct InitStatusArgs {
    /// Output machine-readable JSON. With `--watch`, emits one JSON object per update.
    #[arg(long, default_value_t = false)]
    pub json: bool,

    /// Wait for the selected init session to reach a terminal state before printing.
    #[arg(long, default_value_t = false, conflicts_with = "watch")]
    pub wait: bool,

    /// Stream status updates for the selected active init session until it reaches a terminal state.
    #[arg(long, default_value_t = false)]
    pub watch: bool,

    /// Show only the matching active init session id (exact match or prefix).
    #[arg(long, value_name = "SESSION_ID")]
    pub session_id: Option<String>,
}

#[derive(Args, Debug, Clone)]
#[command(args_conflicts_with_subcommands = true)]
pub struct InitArgs {
    #[command(subcommand)]
    pub command: Option<InitCommand>,

    /// Remove and reinstall existing hooks for selected agents.
    #[arg(long, short = 'f')]
    pub force: bool,

    /// Do not install the repo-local DevQL guidance surface; when disabled,
    /// Bitloops also suppresses DevQL session-start bootstrap messaging.
    #[arg(long = "disable-devql-guidance", default_value_t = false)]
    pub disable_devql_guidance: bool,

    /// Target specific agent setups (repeatable).
    #[arg(long = "agent", value_name = "AGENT")]
    pub agent: Vec<String>,

    /// Queue an initial DevQL sync after hook setup.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub sync: Option<bool>,

    /// Run historical DevQL ingest after hook setup.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub ingest: Option<bool>,

    /// Bound init-triggered historical ingest to the latest N commits (bare flag = 50).
    #[arg(
        long,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "50",
        value_parser = parse_backfill_value
    )]
    pub backfill: Option<usize>,

    /// Exclude repo-relative paths/globs from DevQL indexing (repeatable).
    #[arg(long = "exclude")]
    pub exclude: Vec<String>,

    /// Load additional exclusion globs from files under the repo-policy root (repeatable).
    #[arg(long = "exclude-from")]
    pub exclude_from: Vec<String>,

    /// Select which embeddings runtime to configure during init.
    #[arg(long, value_enum)]
    pub embeddings_runtime: Option<EmbeddingsRuntime>,

    /// Select which summaries runtime to configure during init.
    #[arg(long, value_enum)]
    pub summaries_runtime: Option<SummariesRuntime>,

    /// Control summary embeddings setup during init.
    #[arg(long, value_enum)]
    pub summary_embeddings_mode: Option<SummaryEmbeddingsMode>,
}

fn parse_backfill_value(raw: &str) -> std::result::Result<usize, String> {
    let parsed = raw
        .parse::<usize>()
        .map_err(|_| format!("invalid value `{raw}` for `--backfill`"))?;
    if parsed == 0 {
        return Err("`--backfill` must be greater than zero".to_string());
    }
    Ok(parsed)
}

pub(crate) fn normalize_cli_exclusions(values: &[String]) -> Vec<String> {
    let mut normalized = values
        .iter()
        .map(|value| value.trim().replace('\\', "/"))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

pub(crate) fn normalize_exclude_from_paths(
    policy_root: &Path,
    values: &[String],
) -> Result<Vec<String>> {
    let policy_root = policy_root
        .canonicalize()
        .unwrap_or_else(|_| policy_root.to_path_buf());
    let mut normalized = Vec::new();

    for raw_value in values {
        let raw_value = raw_value.trim();
        if raw_value.is_empty() {
            continue;
        }
        let candidate = PathBuf::from(raw_value);
        let absolute = if candidate.is_absolute() {
            candidate
        } else {
            policy_root.join(candidate)
        };
        let absolute = normalize_lexical_path(&absolute);
        if !absolute.starts_with(&policy_root) {
            bail!(
                "`--exclude-from` path `{}` must be under repo-policy root {}",
                raw_value,
                policy_root.display()
            );
        }
        let relative = absolute
            .strip_prefix(&policy_root)
            .unwrap_or(absolute.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        if !relative.is_empty() {
            normalized.push(relative);
        }
    }

    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}
