use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

use bitloops_embeddings::runtime::{RuntimeOptions, run_stdio_runtime};

#[derive(Debug, Args, Clone)]
pub struct EmbeddingsRuntimeArgs {
    #[arg(long, default_value = "bitloops-embeddings.toml")]
    pub config: PathBuf,

    #[arg(long)]
    pub profile: String,

    #[arg(long)]
    pub repo_root: Option<PathBuf>,
}

pub fn run(args: EmbeddingsRuntimeArgs) -> Result<()> {
    run_stdio_runtime(&RuntimeOptions {
        config_path: args.config,
        selected_profile: args.profile,
        repo_root: args.repo_root,
    })
}
