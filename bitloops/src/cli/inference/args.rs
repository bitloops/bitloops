use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::env;
use std::path::{Path, PathBuf};

use crate::cli::enable::find_repo_root;

use super::managed::install_or_bootstrap_inference;
use super::setup::{ArchitectureInferenceRuntime, write_codex_architecture_inference_profiles};

#[derive(Args, Debug, Clone, Default)]
pub struct InferenceArgs {
    #[command(subcommand)]
    pub command: Option<InferenceCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum InferenceCommand {
    /// Install or update the managed standalone inference runtime.
    Install(InferenceInstallArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct InferenceInstallArgs {
    /// Configure architecture structured-generation inference through a local CLI agent.
    #[arg(long, value_enum)]
    pub architecture_runtime: Option<ArchitectureInferenceRuntime>,

    /// Model used by the selected architecture runtime.
    #[arg(long, default_value = "gpt-5.4-mini")]
    pub architecture_model: String,
}

pub fn run(args: InferenceArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!("missing subcommand. Use `bitloops inference install`.");
    };

    match command {
        InferenceCommand::Install(args) => {
            let repo_root = current_repo_root()?;
            for line in install_or_bootstrap_inference(&repo_root)? {
                println!("{line}");
            }
            if matches!(
                args.architecture_runtime,
                Some(ArchitectureInferenceRuntime::Codex)
            ) {
                println!(
                    "{}",
                    write_codex_architecture_inference_profiles(
                        &repo_root,
                        &args.architecture_model,
                    )?
                );
            }
            Ok(())
        }
    }
}

fn current_repo_root() -> Result<PathBuf> {
    let cwd = env::current_dir().context("getting current directory")?;
    repo_root_from_cwd(&cwd)
}

pub(crate) fn repo_root_from_cwd(cwd: &Path) -> Result<PathBuf> {
    find_repo_root(cwd)
}
