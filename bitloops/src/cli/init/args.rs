use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};
use clap::Args;

use crate::cli::embeddings::EmbeddingsRuntime;

pub(crate) const DEFAULT_INIT_INGEST_BACKFILL: usize = 50;

#[derive(Args)]
pub struct InitArgs {
    /// Bootstrap and start the default Bitloops daemon service if it is not already running.
    #[arg(long, default_value_t = false)]
    pub install_default_daemon: bool,

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

    /// Accepted for compatibility; `bitloops init` no longer runs the initial baseline sync.
    #[arg(long, default_value_t = false)]
    pub skip_baseline: bool,

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

    /// Select which embeddings runtime to configure when embeddings are installed during init.
    #[arg(long, value_enum)]
    pub embeddings_runtime: Option<EmbeddingsRuntime>,

    /// Skip embeddings setup during init.
    #[arg(
        long,
        default_value_t = false,
        conflicts_with = "embeddings_runtime",
        conflicts_with = "embeddings_gateway_url",
        conflicts_with = "embeddings_api_key_env"
    )]
    pub no_embeddings: bool,

    /// Public platform embeddings endpoint used when `--embeddings-runtime platform` is selected.
    #[arg(long)]
    pub embeddings_gateway_url: Option<String>,

    /// Environment variable that contains the platform gateway bearer token.
    #[arg(long, default_value = "BITLOOPS_PLATFORM_GATEWAY_TOKEN")]
    pub embeddings_api_key_env: String,
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
