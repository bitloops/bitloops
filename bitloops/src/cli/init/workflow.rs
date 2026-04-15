use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::adapters::agents::AgentAdapterRegistry;
use crate::capability_packs::semantic_clones::workplane::activate_deferred_pipeline_mailboxes;
use crate::cli::embeddings::{enqueue_embeddings_bootstrap_task, install_or_bootstrap_embeddings};
use crate::cli::inference::{
    configure_local_summary_generation, prepare_local_summary_generation_plan,
};
use crate::cli::telemetry_consent;
use crate::config::settings::{
    DEFAULT_STRATEGY, load_settings, set_scope_exclusions,
    write_project_bootstrap_settings_with_daemon_binding,
};
use crate::config::{REPO_POLICY_LOCAL_FILE_NAME, default_daemon_config_exists};
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, bitloops_wordmark, color_hex_if_enabled};

use super::progress::{InitProgressOptions, run_dual_init_progress};
use super::{
    AgentSelector, DEFAULT_INIT_INGEST_BACKFILL, InitArgs, QueuedEmbeddingsBootstrapTask,
    detect_or_select_agent, ensure_repo_local_policy_excluded, maybe_install_default_daemon,
    normalize_cli_exclusions, normalize_exclude_from_paths,
    should_configure_summaries_during_init, should_install_embeddings_during_init,
    should_run_initial_ingest, should_run_initial_sync,
};

pub(super) async fn run_for_project_root(
    args: InitArgs,
    project_root: &Path,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let git_root = crate::cli::enable::find_repo_root(project_root)?;
    let daemon_config_existed_at_entry = default_daemon_config_exists()?;
    let telemetry_choice =
        telemetry_consent::telemetry_flag_choice(args.telemetry, args.no_telemetry);
    if args.backfill.is_some() && args.ingest == Some(false) {
        bail!("`bitloops init --backfill` cannot be combined with `--ingest=false`.");
    }
    let effective_ingest = if args.backfill.is_some() {
        Some(true)
    } else {
        args.ingest
    };

    if (args.sync.is_none() || effective_ingest.is_none())
        && !telemetry_consent::can_prompt_interactively()
    {
        bail!(
            "`bitloops init` requires explicit `--sync=true|false` and `--ingest=true|false` choices when not running interactively."
        );
    }

    if !daemon_config_existed_at_entry
        && args.install_default_daemon
        && telemetry_choice.is_none()
        && !telemetry_consent::can_prompt_interactively()
    {
        bail!(telemetry_consent::NON_INTERACTIVE_TELEMETRY_ERROR);
    }

    maybe_install_default_daemon(args.install_default_daemon).await?;
    telemetry_consent::ensure_default_daemon_running().await?;
    let daemon_config_path = bound_running_daemon_config_path().await?;
    if daemon_config_existed_at_entry {
        telemetry_consent::ensure_existing_config_telemetry_consent(
            project_root,
            telemetry_choice,
            out,
            input,
        )
        .await?;
    } else if let Some(choice) = telemetry_choice {
        let persisted =
            telemetry_consent::update_cli_telemetry_consent_via_daemon(project_root, Some(choice))
                .await?;
        if persisted.needs_prompt {
            bail!("failed to persist telemetry consent");
        }
    }
    ensure_repo_local_policy_excluded(&git_root, project_root)?;

    let selected_agents = if let Some(agent) = args.agent.as_deref() {
        vec![AgentAdapterRegistry::builtin().normalise_agent_name(agent)?]
    } else {
        detect_or_select_agent(project_root, out, select_fn)?
    };
    let strategy = load_settings(project_root)
        .map(|settings| settings.strategy)
        .unwrap_or_else(|_| DEFAULT_STRATEGY.to_string());
    let scope_exclude = normalize_cli_exclusions(&args.exclude);
    let scope_exclude_from = normalize_exclude_from_paths(project_root, &args.exclude_from)?;
    let local_policy_path = project_root.join(REPO_POLICY_LOCAL_FILE_NAME);
    write_project_bootstrap_settings_with_daemon_binding(
        &local_policy_path,
        &strategy,
        &selected_agents,
        Some(&daemon_config_path),
    )?;
    if !scope_exclude.is_empty() || !scope_exclude_from.is_empty() {
        set_scope_exclusions(&local_policy_path, &scope_exclude, &scope_exclude_from)?;
    }

    let settings = load_settings(project_root).unwrap_or_default();
    let git_count = crate::adapters::agents::claude_code::git_hooks::install_git_hooks(
        &git_root,
        settings.local_dev,
    )?;
    if git_count > 0 {
        writeln!(out, "Installed {git_count} git hook(s).")?;
    }

    crate::cli::agent_surfaces::reconcile_project_agent_surfaces(
        project_root,
        &selected_agents,
        settings.local_dev,
        args.force,
        out,
    )?;
    if args.install_default_daemon {
        activate_deferred_pipeline_mailboxes(&git_root, "init")
            .context("activating semantic clones deferred mailboxes for init")?;
    }

    let mut queued_embeddings_bootstrap = None;
    let mut prepared_summary_setup = None;
    let should_install_embeddings = should_install_embeddings_during_init(
        project_root,
        args.install_default_daemon,
        out,
        input,
    )?;
    if should_install_embeddings {
        if args.install_default_daemon {
            queued_embeddings_bootstrap =
                Some(enqueue_embeddings_bootstrap_during_init(project_root, out).await?);
        } else {
            install_embeddings_during_init(project_root, out)?;
        }
    }
    if should_configure_summaries_during_init(
        project_root,
        args.install_default_daemon,
        out,
        input,
    )? {
        if args.install_default_daemon {
            prepared_summary_setup = Some(
                prepare_local_summary_generation_plan(
                    out,
                    input,
                    telemetry_consent::can_prompt_interactively(),
                )
                .map_err(|err| {
                    anyhow::anyhow!(
                        "Bitloops init completed, but semantic summary setup failed: {err:#}"
                    )
                })?,
            );
        } else {
            configure_local_summary_generation(
                project_root,
                out,
                input,
                telemetry_consent::can_prompt_interactively(),
            )
            .map_err(|err| {
                anyhow::anyhow!(
                    "Bitloops init completed, but semantic summary setup failed: {err:#}"
                )
            })?;
        }
    }
    let should_sync = should_run_initial_sync(args.sync, out, input)?;
    let should_ingest = should_run_initial_ingest(effective_ingest, out, input)?;
    if args.install_default_daemon {
        write_init_setup_handoff(out).await?;
    }
    let run_concurrent_init_progress = args.install_default_daemon
        && (prepared_summary_setup.is_some()
            || (queued_embeddings_bootstrap.is_some() && (should_sync || should_ingest)));
    if should_sync || should_ingest || prepared_summary_setup.is_some() {
        let scope = crate::devql_transport::discover_slim_cli_repo_scope(Some(project_root))?;
        if run_concurrent_init_progress {
            let initial_top_task = if should_sync {
                let (task, _merged) = crate::cli::devql::graphql::enqueue_sync_task_via_graphql(
                    &scope, false, None, false, false, "init", false,
                )
                .await?;
                Some(task)
            } else if should_ingest {
                let (task, _merged) = crate::cli::devql::graphql::enqueue_ingest_task_via_graphql(
                    &scope,
                    Some(args.backfill.unwrap_or(DEFAULT_INIT_INGEST_BACKFILL)),
                    false,
                )
                .await?;
                Some(task)
            } else {
                None
            };
            run_dual_init_progress(
                out,
                &scope,
                initial_top_task,
                InitProgressOptions {
                    show_sync: should_sync,
                    show_ingest: should_ingest,
                    enqueue_ingest_after_sync: should_sync && should_ingest,
                    ingest_backfill: args.backfill.unwrap_or(DEFAULT_INIT_INGEST_BACKFILL),
                    queued_embeddings_bootstrap,
                    prepared_summary_setup,
                },
            )
            .await?;
        } else if should_sync {
            writeln!(out, "Starting initial DevQL sync...")?;
            out.flush()?;
            let (task, _merged) = crate::cli::devql::graphql::enqueue_sync_task_via_graphql(
                &scope, false, None, false, false, "init", false,
            )
            .await?;
            if let Some(task) =
                crate::cli::devql::graphql::watch_task_via_graphql(&scope, task.clone()).await?
            {
                writeln!(
                    out,
                    "{}",
                    crate::cli::devql::format_task_completion_summary(&task)
                )?;
            }
            if should_ingest {
                writeln!(out, "Starting initial DevQL ingest after sync...")?;
                out.flush()?;
                let (task, _merged) = crate::cli::devql::graphql::enqueue_ingest_task_via_graphql(
                    &scope,
                    Some(args.backfill.unwrap_or(DEFAULT_INIT_INGEST_BACKFILL)),
                    false,
                )
                .await?;
                if let Some(task) =
                    crate::cli::devql::graphql::watch_task_via_graphql(&scope, task).await?
                {
                    writeln!(
                        out,
                        "{}",
                        crate::cli::devql::format_task_completion_summary(&task)
                    )?;
                }
            }
        } else if should_ingest {
            if should_sync {
                writeln!(out, "Starting initial DevQL ingest after sync...")?;
            } else {
                writeln!(out, "Starting initial DevQL ingest...")?;
            }
            out.flush()?;
            let (task, _merged) = crate::cli::devql::graphql::enqueue_ingest_task_via_graphql(
                &scope,
                Some(args.backfill.unwrap_or(DEFAULT_INIT_INGEST_BACKFILL)),
                false,
            )
            .await?;
            if let Some(task) =
                crate::cli::devql::graphql::watch_task_via_graphql(&scope, task).await?
            {
                writeln!(
                    out,
                    "{}",
                    crate::cli::devql::format_task_completion_summary(&task)
                )?;
            }
        }
    }
    Ok(())
}

async fn bound_running_daemon_config_path() -> Result<std::path::PathBuf> {
    if let Some(runtime) = crate::daemon::status().await?.runtime {
        return Ok(runtime
            .config_path
            .canonicalize()
            .unwrap_or(runtime.config_path));
    }

    #[cfg(test)]
    if crate::cli::telemetry_consent::test_assume_daemon_running_override() == Some(true) {
        let config_path = crate::config::ensure_daemon_config_exists()?;
        return Ok(config_path.canonicalize().unwrap_or(config_path));
    }

    #[cfg(test)]
    if std::env::var("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING")
        .ok()
        .is_some_and(|value| !value.trim().is_empty() && value.trim() != "0")
    {
        let config_path = crate::config::ensure_daemon_config_exists()?;
        return Ok(config_path.canonicalize().unwrap_or(config_path));
    }

    let runtime = crate::daemon::status()
        .await?
        .runtime
        .context("Bitloops daemon is not running")?;
    Ok(runtime
        .config_path
        .canonicalize()
        .unwrap_or(runtime.config_path))
}

async fn enqueue_embeddings_bootstrap_during_init(
    project_root: &Path,
    out: &mut dyn Write,
) -> Result<QueuedEmbeddingsBootstrapTask> {
    writeln!(out, "Queueing embeddings bootstrap in the daemon...")?;
    let (scope, queued) =
        enqueue_embeddings_bootstrap_task(project_root, None, crate::daemon::DevqlTaskSource::Init)
            .await?;
    let phase = queued
        .task
        .embeddings_bootstrap_progress()
        .map(|progress| progress.phase.as_str())
        .unwrap_or("queued");
    writeln!(out, "Embeddings bootstrap task: {}", queued.task.task_id)?;
    writeln!(out, "Embeddings bootstrap phase: {phase}")?;
    out.flush()?;
    Ok(QueuedEmbeddingsBootstrapTask {
        scope,
        task_id: queued.task.task_id,
    })
}

fn install_embeddings_during_init(project_root: &Path, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "Preparing local embeddings setup...")?;
    writeln!(
        out,
        "This can take a moment if the managed runtime needs to be downloaded."
    )?;
    out.flush()?;
    match install_or_bootstrap_embeddings(project_root) {
        Ok(lines) => {
            for line in lines {
                writeln!(out, "{line}")?;
            }
            Ok(())
        }
        Err(err) => {
            bail!("Bitloops init completed, but embeddings installation failed: {err:#}");
        }
    }
}

async fn write_init_setup_handoff(out: &mut dyn Write) -> Result<()> {
    writeln!(out)?;
    writeln!(
        out,
        "{}",
        color_hex_if_enabled(&bitloops_wordmark(), BITLOOPS_PURPLE_HEX)
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "The setup is complete! You can continue on with your work and Bitloops will continue enriching your codebase's Intelligence Layer in the background. You can continue viewing the progress here or you can close this terminal if you prefer and you can always run `bitloops status` or visit the dashboard for more information."
    )?;
    writeln!(out)?;
    if let Some(url) = current_dashboard_url().await? {
        writeln!(out, "Dashboard URL: {url}")?;
    }
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

async fn current_dashboard_url() -> Result<Option<String>> {
    Ok(crate::daemon::status()
        .await
        .ok()
        .and_then(|status| status.runtime.map(|runtime| runtime.url)))
}
