use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result};

mod agent_hooks;
mod agent_selection;
mod args;
mod context_guidance_setup;
mod daemon_bootstrap;
mod embeddings_setup;
mod final_setup;
mod progress;
mod repo_excludes;
mod status;
mod summary_setup;
mod workflow;
mod workflow_output;

#[cfg(test)]
mod tests;

pub use agent_selection::{InitAgentSelection, detect_or_select_agent};
pub use args::{InitArgs, InitCommand, InitStatusArgs};

pub(super) use args::{
    DEFAULT_INIT_INGEST_BACKFILL, normalize_cli_exclusions, normalize_exclude_from_paths,
};
pub(super) use context_guidance_setup::choose_context_guidance_setup_during_init;
pub(super) use daemon_bootstrap::{
    maybe_enable_default_daemon_service, maybe_install_default_daemon,
};
#[cfg(test)]
pub(super) use daemon_bootstrap::{
    with_enable_default_daemon_service_hook, with_install_default_daemon_hook,
};
pub(super) use embeddings_setup::{
    InitEmbeddingsSetupSelection, should_install_embeddings_during_init,
};
#[cfg(test)]
pub(super) use embeddings_setup::{
    NON_INTERACTIVE_INIT_EMBEDDINGS_SELECTION_ERROR, prompt_install_embeddings_setup_selection,
};
#[cfg(test)]
pub(super) use final_setup::InitFinalSetupSelection;
pub(super) use final_setup::{InitFinalSetupPromptOptions, choose_final_setup_options};
pub(super) use repo_excludes::ensure_repo_init_files_excluded;
pub(crate) use repo_excludes::{
    clear_repo_local_policy_excluded, clear_repo_managed_skill_files_excluded,
};
pub(super) use summary_setup::choose_summary_setup_during_init;

pub type AgentSelector = dyn Fn(&[String], bool) -> std::result::Result<InitAgentSelection, String>;

pub async fn run(args: InitArgs) -> Result<()> {
    let mut out = io::stdout().lock();
    let stdin = io::stdin();
    let mut input = stdin.lock();
    run_with_io_async(args, &mut out, &mut input, None).await
}

#[cfg(test)]
fn run_with_writer_for_project_root(
    args: InitArgs,
    project_root: &Path,
    out: &mut dyn Write,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("creating runtime for `bitloops init`")?;
    let mut input = io::Cursor::new(Vec::<u8>::new());
    runtime.block_on(run_with_io_async_for_project_root(
        args,
        project_root,
        out,
        &mut input,
        select_fn,
    ))
}

async fn run_with_io_async(
    args: InitArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let project_root = std::env::current_dir().context("getting current directory")?;
    run_with_io_async_for_project_root(args, &project_root, out, input, select_fn).await
}

async fn run_with_io_async_for_project_root(
    args: InitArgs,
    project_root: &Path,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    if let Some(InitCommand::Status(status_args)) = args.command.clone() {
        return status::run_for_project_root(status_args, project_root, out).await;
    }

    workflow::run_for_project_root(args, project_root, out, input, select_fn).await
}
