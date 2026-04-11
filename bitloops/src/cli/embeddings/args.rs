use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::env;
use std::path::Path;
use std::path::PathBuf;

use crate::cli::enable::find_repo_root;
use crate::cli::telemetry_consent;
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, resolve_bound_daemon_config_path_for_repo,
    resolve_daemon_config_path_for_repo, resolve_embedding_capability_config_for_repo,
};
use crate::daemon::DevqlTaskSource;
use crate::devql_transport::{SlimCliRepoScope, discover_slim_cli_repo_scope};
use crate::host::devql::{DevqlConfig, resolve_repo_identity};

use super::profiles::{clear_cache_for_profile, doctor_profile};

#[derive(Args, Debug, Clone, Default)]
pub struct EmbeddingsArgs {
    #[command(subcommand)]
    pub command: Option<EmbeddingsCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum EmbeddingsCommand {
    /// Install or update the managed standalone embeddings runtime.
    Install(EmbeddingsInstallArgs),
    /// Download or warm a local embedding profile into its cache directory.
    Pull(EmbeddingsPullArgs),
    /// Inspect the selected or explicitly named embedding profile.
    Doctor(EmbeddingsDoctorArgs),
    /// Remove the cache for a local embedding profile.
    ClearCache(EmbeddingsClearCacheArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct EmbeddingsInstallArgs {}

#[derive(Args, Debug, Clone)]
pub struct EmbeddingsPullArgs {
    pub profile: String,
}

#[derive(Args, Debug, Clone, Default)]
pub struct EmbeddingsDoctorArgs {
    #[arg(value_name = "profile")]
    pub profile: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct EmbeddingsClearCacheArgs {
    pub profile: String,
}

pub fn run(args: EmbeddingsArgs) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("creating runtime for `bitloops embeddings`")?;
    runtime.block_on(run_async(args))
}

pub(crate) async fn run_async(args: EmbeddingsArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(
            "missing subcommand. Use one of: `bitloops embeddings install`, `bitloops embeddings pull`, `bitloops embeddings doctor`, `bitloops embeddings clear-cache`"
        );
    };

    let repo_root = current_repo_root()?;
    let capability = resolve_embedding_capability_config_for_repo(&repo_root);

    match command {
        EmbeddingsCommand::Install(_args) => {
            let (scope, queued) =
                enqueue_embeddings_bootstrap_task(&repo_root, None, DevqlTaskSource::ManualCli)
                    .await?;
            if let Some(task) = crate::cli::devql::graphql::watch_task_id_via_graphql(
                &scope,
                &queued.task.task_id,
                false,
            )
            .await?
            {
                println!(
                    "{}",
                    crate::cli::devql::format_task_completion_summary(&task)
                );
            }
            Ok(())
        }
        EmbeddingsCommand::Pull(args) => {
            let (scope, queued) = enqueue_embeddings_bootstrap_task(
                &repo_root,
                Some(args.profile.as_str()),
                DevqlTaskSource::ManualCli,
            )
            .await?;
            if let Some(task) = crate::cli::devql::graphql::watch_task_id_via_graphql(
                &scope,
                &queued.task.task_id,
                false,
            )
            .await?
            {
                println!(
                    "{}",
                    crate::cli::devql::format_task_completion_summary(&task)
                );
            }
            Ok(())
        }
        EmbeddingsCommand::Doctor(args) => {
            let lines = doctor_profile(&repo_root, &capability, args.profile.as_deref())?;
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        EmbeddingsCommand::ClearCache(args) => {
            let lines = clear_cache_for_profile(&repo_root, &capability, &args.profile)?;
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
    }
}

pub(crate) async fn enqueue_embeddings_bootstrap_task(
    repo_root: &Path,
    requested_profile: Option<&str>,
    source: DevqlTaskSource,
) -> Result<(SlimCliRepoScope, crate::daemon::DevqlTaskEnqueueResult)> {
    telemetry_consent::ensure_default_daemon_running().await?;
    let repo = resolve_repo_identity(repo_root)?;
    let config_path = resolve_bound_daemon_config_path_for_repo(repo_root)
        .or_else(|_| resolve_daemon_config_path_for_repo(repo_root))
        .unwrap_or_else(|_| repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH));
    let profile_name = match requested_profile {
        Some(profile_name) => profile_name.to_string(),
        None => crate::cli::embeddings::embedding_capability_for_config_path(&config_path)
            .ok()
            .and_then(|capability| {
                crate::cli::embeddings::selected_inference_profile_name(&capability)
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| "local_code".to_string()),
    };
    let config_root = config_path
        .parent()
        .context("resolving daemon config root for embeddings bootstrap")?
        .to_path_buf();
    let cfg = DevqlConfig::from_roots(config_root, repo_root.to_path_buf(), repo)?;
    let queued = crate::daemon::enqueue_embeddings_bootstrap_for_config(
        &cfg,
        source,
        config_path,
        profile_name,
    )?;
    let scope = discover_slim_cli_repo_scope(Some(repo_root))?;
    Ok((scope, queued))
}

fn current_repo_root() -> Result<PathBuf> {
    let cwd = env::current_dir().context("getting current directory")?;
    find_repo_root(&cwd)
}
