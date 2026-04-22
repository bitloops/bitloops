use std::future::Future;
use std::io::{self, Write};
use std::path::PathBuf;
use std::pin::Pin;

use anyhow::{Result, bail};

mod confirm;
mod hooks;
mod picker;
mod repo;
mod shell;
mod system;
mod targets;
mod tty;

pub use targets::UninstallArgs;

use confirm::confirm_uninstall;
use hooks::{uninstall_agent_hooks, uninstall_git_hooks, uninstall_repo_config};
use repo::resolve_scope;
use shell::uninstall_shell_integration;
use system::{
    default_service_uninstaller, known_binary_candidates, uninstall_binaries, uninstall_cache,
    uninstall_config, uninstall_data, uninstall_service,
};
use targets::{ALL_TARGETS, UninstallTarget, collect_requested_targets, validate_scope_flags};

const NO_FLAGS_ERROR: &str = "`bitloops uninstall` without flags requires an interactive terminal; pass explicit flags such as `--full` or `--git-hooks`";

type UninstallSelector =
    dyn Fn(&[UninstallTarget]) -> std::result::Result<Vec<UninstallTarget>, String>;
type ServiceUninstaller = dyn Fn() -> Result<()>;
type BinaryCandidatesFn = dyn Fn() -> Result<Vec<PathBuf>>;
type DaemonStopFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;
type DaemonStopper = dyn Fn() -> DaemonStopFuture;

struct RunContext<'a> {
    select_fn: Option<&'a UninstallSelector>,
    daemon_stopper: &'a DaemonStopper,
    service_uninstaller: &'a ServiceUninstaller,
    binary_candidates: &'a BinaryCandidatesFn,
}

pub async fn run(args: UninstallArgs) -> Result<()> {
    let stdout = io::stdout();
    let stderr = io::stderr();
    let mut out = stdout.lock();
    let mut err = stderr.lock();
    let context = RunContext {
        select_fn: None,
        daemon_stopper: &|| Box::pin(crate::daemon::stop()),
        service_uninstaller: &default_service_uninstaller,
        binary_candidates: &known_binary_candidates,
    };

    run_with_context(args, &mut out, &mut err, context).await
}

async fn run_with_context(
    args: UninstallArgs,
    out: &mut dyn Write,
    err_out: &mut dyn Write,
    context: RunContext<'_>,
) -> Result<()> {
    let Some(targets) = collect_requested_targets(&args, out, context.select_fn)? else {
        writeln!(out, "Uninstall cancelled.")?;
        return Ok(());
    };

    validate_scope_flags(&args, &targets)?;
    let scope = resolve_scope(&args, &targets)?;

    if !args.force
        && !confirm_uninstall(
            out,
            &targets,
            if scope.agent_project_roots.is_empty() {
                &scope.repo_config_project_roots
            } else {
                &scope.agent_project_roots
            },
            &scope.hook_repo_roots,
            &scope.repo_data_roots,
        )?
    {
        writeln!(out, "Uninstall cancelled.")?;
        return Ok(());
    }

    writeln!(out, "Running Bitloops uninstall...")?;

    let mut failures = Vec::new();
    for target in ALL_TARGETS
        .iter()
        .copied()
        .filter(|target| targets.contains(target))
    {
        let result = match target {
            UninstallTarget::AgentHooks => uninstall_agent_hooks(&scope.agent_project_roots, out),
            UninstallTarget::RepoConfig => {
                uninstall_repo_config(&scope.repo_config_project_roots, out)
            }
            UninstallTarget::GitHooks => uninstall_git_hooks(&scope.hook_repo_roots, out),
            UninstallTarget::Shell => uninstall_shell_integration(out),
            UninstallTarget::Data => uninstall_data(&scope.repo_data_roots, out),
            UninstallTarget::Caching => uninstall_cache(out),
            UninstallTarget::Config => uninstall_config(out),
            UninstallTarget::Service => {
                if let Err(err) = (context.daemon_stopper)().await
                    && !is_daemon_not_running_error(&err)
                {
                    writeln!(
                        err_out,
                        "Warning: unable to stop Bitloops daemon before removing the service: {err:#}"
                    )?;
                }
                uninstall_service(out, context.service_uninstaller)
            }
            UninstallTarget::Binaries => uninstall_binaries(out, context.binary_candidates),
        };

        if let Err(err) = result {
            let message = format!("{}: {err:#}", target.label());
            writeln!(err_out, "Warning: {message}")?;
            failures.push(message);
        }
    }

    if failures.is_empty() {
        writeln!(out, "Bitloops uninstall completed.")?;
        return Ok(());
    }

    bail!(
        "Bitloops uninstall completed with {} failure(s)",
        failures.len()
    );
}

#[cfg(test)]
mod tests;

fn is_daemon_not_running_error(err: &anyhow::Error) -> bool {
    err.to_string() == "Bitloops daemon is not running. Start it with `bitloops daemon start`."
}
