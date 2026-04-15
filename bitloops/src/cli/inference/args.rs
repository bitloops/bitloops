use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::env;
use std::path::{Path, PathBuf};

use crate::cli::enable::find_repo_root;

use super::managed::install_or_bootstrap_inference;

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
pub struct InferenceInstallArgs {}

pub fn run(args: InferenceArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!("missing subcommand. Use `bitloops inference install`.");
    };

    match command {
        InferenceCommand::Install(_args) => {
            let repo_root = current_repo_root()?;
            for line in install_or_bootstrap_inference(&repo_root)? {
                println!("{line}");
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
