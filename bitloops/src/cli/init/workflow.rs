use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::adapters::agents::AgentAdapterRegistry;
use crate::capability_packs::semantic_clones::workplane::{
    activate_deferred_pipeline_mailboxes, activate_embedding_pipeline_mailboxes,
    activate_summary_refresh_mailbox,
};
use crate::cli::embeddings::{
    install_or_bootstrap_embeddings, install_or_configure_platform_embeddings,
    platform_embeddings_gateway_url_override,
};
use crate::cli::inference::{
    PreparedSummarySetupAction, SummarySetupSelection, configure_cloud_summary_generation,
    configure_local_summary_generation, platform_summary_gateway_url_override,
    prepare_cloud_summary_generation_plan, prepare_local_summary_generation_plan,
    summary_generation_configured,
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
    AgentSelector, DEFAULT_INIT_INGEST_BACKFILL, InitArgs, InitEmbeddingsSetupSelection,
    choose_summary_setup_during_init, detect_or_select_agent, ensure_repo_local_policy_excluded,
    maybe_install_default_daemon, normalize_cli_exclusions, normalize_exclude_from_paths,
    should_install_embeddings_during_init, should_run_initial_ingest, should_run_initial_sync,
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

    let selected_agents = if !args.agent.is_empty() {
        resolve_cli_agents(&args.agent)?
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
    let mut embeddings_bootstrap = None;
    let mut prepared_summary_setup = None;
    let embeddings_selection =
        should_install_embeddings_during_init(project_root, &args, out, input)?;
    match embeddings_selection {
        InitEmbeddingsSetupSelection::Existing => {}
        InitEmbeddingsSetupSelection::Cloud => {
            let gateway_url =
                platform_embeddings_gateway_url_override(args.embeddings_gateway_url.as_deref());
            for line in install_or_configure_platform_embeddings(
                project_root,
                gateway_url.as_deref(),
                &args.embeddings_api_key_env,
            )? {
                writeln!(out, "{line}")?;
            }
        }
        InitEmbeddingsSetupSelection::Local => {
            if args.install_default_daemon {
                embeddings_bootstrap = Some(resolve_embeddings_bootstrap_request(project_root)?);
            } else {
                install_embeddings_during_init(project_root, out)?;
            }
        }
        InitEmbeddingsSetupSelection::Skip => {}
    }
    match choose_summary_setup_during_init(project_root, args.install_default_daemon, out, input)
        .await?
    {
        SummarySetupSelection::Cloud => {
            crate::cli::login::ensure_logged_in().await?;
            let gateway_url_override = platform_summary_gateway_url_override();
            if args.install_default_daemon {
                prepared_summary_setup = Some(prepare_cloud_summary_generation_plan(
                    gateway_url_override.as_deref(),
                ));
            } else {
                let message = configure_cloud_summary_generation(
                    project_root,
                    gateway_url_override.as_deref(),
                )
                .map_err(|err| {
                    anyhow::anyhow!(
                        "Bitloops init completed, but semantic summary setup failed: {err:#}"
                    )
                })?;
                writeln!(out, "{message}")?;
            }
        }
        SummarySetupSelection::Local => {
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
        SummarySetupSelection::Skip => {}
    }
    if args.install_default_daemon {
        activate_selected_init_mailboxes(
            &git_root,
            matches!(
                embeddings_selection,
                InitEmbeddingsSetupSelection::Existing
                    | InitEmbeddingsSetupSelection::Cloud
                    | InitEmbeddingsSetupSelection::Local
            ),
            prepared_summary_setup.is_some() || summary_generation_configured(project_root),
        )?;
    }
    let should_sync = should_run_initial_sync(args.sync, out, input)?;
    let should_ingest = should_run_initial_ingest(effective_ingest, out, input)?;
    if args.install_default_daemon {
        write_init_setup_handoff(out).await?;
    }
    if should_sync
        || should_ingest
        || embeddings_bootstrap.is_some()
        || prepared_summary_setup.is_some()
    {
        let scope = crate::devql_transport::discover_slim_cli_repo_scope(Some(project_root))?;
        let ingest_backfill =
            should_ingest.then_some(args.backfill.unwrap_or(DEFAULT_INIT_INGEST_BACKFILL));
        let summaries_bootstrap = prepared_summary_setup
            .as_ref()
            .map(runtime_summary_bootstrap_request_from_plan);
        run_dual_init_progress(
            out,
            &scope,
            InitProgressOptions {
                start_input: crate::cli::devql::graphql::RuntimeStartInitInput {
                    repo_id: scope.repo.repo_id.clone(),
                    run_sync: should_sync,
                    run_ingest: should_ingest,
                    ingest_backfill,
                    embeddings_bootstrap,
                    summaries_bootstrap,
                },
            },
        )
        .await?;
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

fn resolve_embeddings_bootstrap_request(
    repo_root: &Path,
) -> Result<crate::cli::devql::graphql::RuntimeEmbeddingsBootstrapRequestInput> {
    let config_path = crate::config::resolve_bound_daemon_config_path_for_repo(repo_root)
        .or_else(|_| crate::config::resolve_daemon_config_path_for_repo(repo_root))
        .unwrap_or_else(|_| repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH));
    let profile_name = crate::cli::embeddings::embedding_capability_for_config_path(&config_path)
        .ok()
        .and_then(|capability| {
            crate::cli::embeddings::selected_inference_profile_name(&capability).map(str::to_string)
        })
        .unwrap_or_else(|| "local_code".to_string());

    Ok(
        crate::cli::devql::graphql::RuntimeEmbeddingsBootstrapRequestInput {
            config_path: config_path.display().to_string(),
            profile_name,
        },
    )
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

fn activate_selected_init_mailboxes(
    repo_root: &Path,
    embeddings_enabled: bool,
    summaries_enabled: bool,
) -> Result<()> {
    match (embeddings_enabled, summaries_enabled) {
        (true, true) => activate_deferred_pipeline_mailboxes(repo_root, "init")
            .context("activating semantic clones deferred mailboxes for init"),
        (true, false) => activate_embedding_pipeline_mailboxes(repo_root, "init")
            .context("activating semantic clones embedding mailboxes for init"),
        (false, true) => activate_summary_refresh_mailbox(repo_root, "init")
            .context("activating semantic clones summary mailboxes for init"),
        (false, false) => Ok(()),
    }
}

fn runtime_summary_bootstrap_request_from_plan(
    plan: &crate::cli::inference::PreparedSummarySetupPlan,
) -> crate::cli::devql::graphql::RuntimeSummaryBootstrapRequestInput {
    match plan.action() {
        PreparedSummarySetupAction::InstallRuntimeOnly { message } => {
            crate::cli::devql::graphql::RuntimeSummaryBootstrapRequestInput {
                action: "install_runtime_only".to_string(),
                message: Some(message.clone()),
                model_name: None,
                gateway_url_override: None,
            }
        }
        PreparedSummarySetupAction::InstallRuntimeOnlyPendingProbe { message } => {
            crate::cli::devql::graphql::RuntimeSummaryBootstrapRequestInput {
                action: "install_runtime_only_pending_probe".to_string(),
                message: Some(message.clone()),
                model_name: None,
                gateway_url_override: None,
            }
        }
        PreparedSummarySetupAction::ConfigureLocal { model_name } => {
            crate::cli::devql::graphql::RuntimeSummaryBootstrapRequestInput {
                action: "configure_local".to_string(),
                message: None,
                model_name: Some(model_name.clone()),
                gateway_url_override: None,
            }
        }
        PreparedSummarySetupAction::ConfigureCloud {
            gateway_url_override,
        } => crate::cli::devql::graphql::RuntimeSummaryBootstrapRequestInput {
            action: "configure_cloud".to_string(),
            message: None,
            model_name: None,
            gateway_url_override: gateway_url_override.clone(),
        },
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
