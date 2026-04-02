use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

use bitloops_embeddings::runtime::{RuntimeOptions, run_stdio_runtime};

#[derive(Debug, Parser)]
#[command(name = "bitloops-embeddings")]
struct Args {
    #[arg(long, default_value = "bitloops-embeddings.toml")]
    config: PathBuf,

    #[arg(long)]
    profile: String,

    #[arg(long)]
    repo_root: Option<PathBuf>,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();
    let options = RuntimeOptions {
        config_path: args.config,
        selected_profile: args.profile,
        repo_root: args.repo_root,
    };
    run_stdio_runtime(&options)
}
