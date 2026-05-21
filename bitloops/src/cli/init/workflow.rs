use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::progress::{InitProgressOptions, run_dual_init_progress};
use super::workflow_output::{
    planned_integrations, write_integrations_installed, write_integrations_installing,
};
use super::{
    AgentSelector, DEFAULT_INIT_INGEST_BACKFILL, InitAgentSelection, InitArgs,
    InitFinalSetupPromptOptions, choose_final_setup_options, detect_or_select_agent,
    ensure_repo_init_files_excluded, normalize_cli_exclusions, normalize_exclude_from_paths,
};
use crate::adapters::agents::AgentAdapterRegistry;
use crate::cli::telemetry_consent;
use crate::config::REPO_POLICY_LOCAL_FILE_NAME;
use crate::config::settings::{
    DEFAULT_STRATEGY, load_settings, set_devql_producer_settings, set_scope_exclusions,
    write_project_bootstrap_settings_with_daemon_binding_and_devql_guidance,
};

fn resolve_cli_agents(values: &[String]) -> Result<Vec<String>> {
    let registry = AgentAdapterRegistry::builtin();
    let mut seen = std::collections::BTreeSet::new();
    let mut resolved = Vec::new();

    for value in values {
        let normalized = registry.normalise_agent_name(value)?;
        if seen.insert(normalized.clone()) {
            resolved.push(normalized);
        }
    }

    Ok(resolved)
}

pub(crate) async fn run_for_project_root(
    args: InitArgs,
    project_root: &Path,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let git_root = crate::cli::enable::find_repo_root(project_root)?;
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

    let daemon_config_path = existing_daemon_config_path()?;
    let selection = if !args.agent.is_empty() {
        InitAgentSelection {
            agents: resolve_cli_agents(&args.agent)?,
            enable_devql_guidance: !args.disable_devql_guidance,
        }
    } else {
        detect_or_select_agent(project_root, out, !args.disable_devql_guidance, select_fn)?
    };
    let selected_agents = selection.agents;
    ensure_repo_init_files_excluded(&git_root, project_root, &selected_agents)?;

    let strategy = load_settings(project_root)
        .map(|settings| settings.strategy)
        .unwrap_or_else(|_| DEFAULT_STRATEGY.to_string());
    let scope_exclude = normalize_cli_exclusions(&args.exclude);
    let scope_exclude_from = normalize_exclude_from_paths(project_root, &args.exclude_from)?;
    let local_policy_path = project_root.join(REPO_POLICY_LOCAL_FILE_NAME);
    write_project_bootstrap_settings_with_daemon_binding_and_devql_guidance(
        &local_policy_path,
        &strategy,
        &selected_agents,
        daemon_config_path.as_deref(),
        selection.enable_devql_guidance,
    )?;
    if !scope_exclude.is_empty() || !scope_exclude_from.is_empty() {
        set_scope_exclusions(&local_policy_path, &scope_exclude, &scope_exclude_from)?;
    }

    let settings = load_settings(project_root).unwrap_or_default();
    let _git_count = crate::adapters::agents::claude_code::git_hooks::install_git_hooks(
        &git_root,
        settings.local_dev,
    )?;
    writeln!(out)?;

    let planned_integrations = planned_integrations(&selected_agents);
    let installing_lines = write_integrations_installing(out, &planned_integrations)?;
    let mut surface_updates = Vec::new();
    let integration_report =
        crate::cli::agent_surfaces::reconcile_project_agent_surfaces_with_options(
            project_root,
            &selected_agents,
            settings.local_dev,
            args.force,
            crate::cli::agent_surfaces::ReconcileProjectAgentSurfacesOptions {
                install_bitloops_skill: selection.enable_devql_guidance,
            },
            &mut surface_updates,
        )?;
    write_integrations_installed(out, &integration_report.integrations, installing_lines)?;
    if !surface_updates.is_empty() {
        out.write_all(&surface_updates)?;
        out.flush()?;
    }

    let final_setup_selection = choose_final_setup_options(
        args.sync,
        out,
        input,
        effective_ingest,
        InitFinalSetupPromptOptions {
            show_sync_and_ingest: true,
            show_telemetry: false,
            show_auto_start_daemon: false,
        },
    )?;
    let should_sync = final_setup_selection.sync;
    let should_ingest = final_setup_selection.ingest;
    set_devql_producer_settings(&local_policy_path, should_sync, should_ingest)?;

    if crate::daemon::daemon_url()?.is_some() {
        crate::cli::watcher_bootstrap::reconcile_repo_watcher(project_root)
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "Bitloops init completed, but DevQL watcher reconcile failed: {err:#}"
                )
            })?;
    }

    if should_sync || should_ingest {
        let scope = crate::devql_transport::discover_slim_cli_repo_scope(Some(project_root))?;
        let ingest_backfill =
            should_ingest.then_some(args.backfill.unwrap_or(DEFAULT_INIT_INGEST_BACKFILL));
        run_dual_init_progress(
            out,
            &scope,
            InitProgressOptions {
                start_input: crate::cli::devql::graphql::RuntimeStartInitInput {
                    repo_id: scope.repo.repo_id.clone(),
                    run_sync: should_sync,
                    run_ingest: should_ingest,
                    run_code_embeddings: false,
                    run_summaries: false,
                    run_summary_embeddings: false,
                    ingest_backfill,
                    embeddings_bootstrap: None,
                    summaries_bootstrap: None,
                },
                show_live_progress_notice: true,
            },
        )
        .await?;
    }

    Ok(())
}

fn existing_daemon_config_path() -> Result<Option<PathBuf>> {
    if let Some(runtime) = crate::daemon::runtime_state()? {
        return Ok(Some(canonical_or_original(runtime.config_path)));
    }
    if let Some(service) = crate::daemon::service_metadata()? {
        return Ok(Some(canonical_or_original(service.config_path)));
    }
    if crate::config::default_daemon_config_exists()? {
        let config_path = crate::config::default_daemon_config_path()?;
        if config_path.is_file() {
            return Ok(Some(canonical_or_original(config_path)));
        }
    }
    Ok(None)
}

fn canonical_or_original(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}
