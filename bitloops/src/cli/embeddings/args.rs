use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::env;
use std::path::PathBuf;

use crate::cli::enable::find_repo_root;
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, resolve_daemon_config_path_for_repo,
    resolve_embedding_capability_config_for_repo,
};

use super::managed::ensure_managed_embeddings_runtime;
use super::profiles::{clear_cache_for_profile, doctor_profile, pull_profile_with_config_path};

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
    let Some(command) = args.command else {
        bail!(
            "missing subcommand. Use one of: `bitloops embeddings install`, `bitloops embeddings pull`, `bitloops embeddings doctor`, `bitloops embeddings clear-cache`"
        );
    };

    let repo_root = current_repo_root()?;
    let capability = resolve_embedding_capability_config_for_repo(&repo_root);
    let config_path = resolve_daemon_config_path_for_repo(&repo_root)
        .unwrap_or_else(|_| repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH));

    match command {
        EmbeddingsCommand::Install(_args) => {
            let lines = ensure_managed_embeddings_runtime(&repo_root, Some(&config_path))?;
            for line in lines {
                println!("{line}");
            }
            Ok(())
        }
        EmbeddingsCommand::Pull(args) => {
            let lines = pull_profile_with_config_path(
                &repo_root,
                &config_path,
                &capability,
                &args.profile,
            )?;
            for line in lines {
                println!("{line}");
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

fn current_repo_root() -> Result<PathBuf> {
    let cwd = env::current_dir().context("getting current directory")?;
    find_repo_root(&cwd)
}
