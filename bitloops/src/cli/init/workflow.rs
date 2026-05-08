use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::adapters::agents::AgentAdapterRegistry;
use crate::capability_packs::semantic_clones::workplane::activate_selected_pipeline_mailboxes;
use crate::cli::embeddings::{
    install_or_bootstrap_embeddings, install_or_configure_platform_embeddings,
    platform_embeddings_gateway_url_override,
};
use crate::cli::inference::{
    ContextGuidanceSetupSelection, PreparedSummarySetupAction, SummarySetupSelection,
    TextGenerationRuntime, configure_cloud_context_guidance_generation,
    configure_cloud_summary_generation, configure_local_context_guidance_generation,
    configure_local_summary_generation, platform_context_guidance_gateway_url_override,
    platform_summary_gateway_url_override, prepare_cloud_summary_generation_plan,
    prepare_local_summary_generation_plan, summary_generation_configured,
};
use crate::cli::telemetry_consent;
use crate::config::settings::{
    DEFAULT_STRATEGY, load_settings, set_devql_producer_settings, set_scope_exclusions,
    write_project_bootstrap_settings_with_daemon_binding_and_devql_guidance,
};
use crate::config::{REPO_POLICY_LOCAL_FILE_NAME, default_daemon_config_exists};
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, bitloops_wordmark, color_hex_if_enabled};
use crate::utils::platform_dirs::bitloops_home_dir;

use super::progress::{InitProgressOptions, run_dual_init_progress};
use super::{
    AgentSelector, DEFAULT_INIT_INGEST_BACKFILL, InitAgentSelection, InitArgs,
    InitEmbeddingsSetupSelection, InitFinalSetupPromptOptions,
    choose_context_guidance_setup_during_init, choose_final_setup_options,
    choose_summary_setup_during_init, detect_or_select_agent, ensure_repo_init_files_excluded,
    maybe_enable_default_daemon_service, maybe_install_default_daemon, normalize_cli_exclusions,
    normalize_exclude_from_paths, should_install_embeddings_during_init,
};

const SUCCESS_GREEN_HEX: &str = "#22c55e";
const INTEGRATION_SPINNER_FRAME: &str = "⠋";

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

fn validate_context_guidance_init_args(args: &InitArgs) -> Result<()> {
    if args.context_guidance_runtime == Some(TextGenerationRuntime::Local)
        && (args.context_guidance_gateway_url.is_some()
            || args.context_guidance_api_key_env.is_some())
    {
        bail!(
            "`--context-guidance-gateway-url` and `--context-guidance-api-key-env` require `--context-guidance-runtime platform`"
        );
    }

    Ok(())
}

fn shell_escape_display_path(path: &Path) -> String {
    let preferred = display_path_with_home(path);
    preferred
        .chars()
        .flat_map(|ch| match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '/' | '.' | '_' | '-' | '~' => [None, Some(ch)],
            _ => [Some('\\'), Some(ch)],
        })
        .flatten()
        .collect()
}

fn display_path_with_home(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let Ok(home) = bitloops_home_dir() else {
        return canonical.display().to_string();
    };
    if canonical == home {
        return "~".to_string();
    }
    if let Ok(relative) = canonical.strip_prefix(&home) {
        let relative = relative.to_string_lossy();
        if relative.is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", relative)
        }
    } else {
        canonical.display().to_string()
    }
}

fn write_default_daemon_bootstrap(
    out: &mut dyn Write,
    config_path: &Path,
    port: u16,
) -> Result<()> {
    writeln!(out, "Starting Bitloops daemon…")?;
    writeln!(out, "  config: {}", shell_escape_display_path(config_path))?;
    writeln!(out, "  port:   {port}")?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

fn write_integrations_installing(
    out: &mut dyn Write,
    integrations: &[crate::cli::agent_surfaces::AgentIntegrationReport],
) -> Result<Option<usize>> {
    let spinner = color_hex_if_enabled(INTEGRATION_SPINNER_FRAME, BITLOOPS_PURPLE_HEX);
    let label_width = integrations
        .iter()
        .map(|integration| integration.label.chars().count())
        .max()
        .unwrap_or(0)
        + 3;
    let mut lines = Vec::new();
    lines.push("Installing integrations…".to_string());
    lines.push(String::new());
    for integration in integrations {
        lines.push(format!(
            "  {} {:<label_width$}({} hooks)",
            spinner,
            integration.label,
            integration.hook_count,
            label_width = label_width
        ));
    }

    for (index, line) in lines.iter().enumerate() {
        write!(out, "{line}")?;
        if index + 1 < lines.len() {
            writeln!(out)?;
        }
    }
    out.flush()?;

    #[cfg(test)]
    {
        Ok(None)
    }

    #[cfg(not(test))]
    {
        if super::agent_selection::can_prompt_interactively() {
            Ok(Some(lines.len()))
        } else {
            Ok(None)
        }
    }
}

fn write_integrations_installed(
    out: &mut dyn Write,
    integrations: &[crate::cli::agent_surfaces::AgentIntegrationReport],
    previous_lines: Option<usize>,
) -> Result<()> {
    let tick = color_hex_if_enabled("✓", SUCCESS_GREEN_HEX);
    let label_width = integrations
        .iter()
        .map(|integration| integration.label.chars().count())
        .max()
        .unwrap_or(0)
        + 3;
    let mut lines = Vec::new();
    lines.push("Integrations installed:".to_string());
    lines.push(String::new());
    for integration in integrations {
        let detail = if integration.state
            == crate::cli::agent_surfaces::AgentIntegrationState::AlreadyInstalled
        {
            format!("{} hooks were already installed", integration.hook_count)
        } else {
            format!("{} hooks", integration.hook_count)
        };
        lines.push(format!(
            "  {} {:<label_width$}({detail})",
            tick,
            integration.label,
            label_width = label_width
        ));
    }

    if let Some(previous_lines) = previous_lines {
        if previous_lines > 0 {
            write!(out, "\x1b[{}F", previous_lines - 1)?;
        } else {
            write!(out, "\r")?;
        }
    }

    for (index, line) in lines.iter().enumerate() {
        write!(out, "\r\x1b[2K{line}")?;
        if index + 1 < lines.len() {
            writeln!(out)?;
        }
    }
    writeln!(out)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

fn planned_integrations(
    selected_agents: &[String],
) -> Vec<crate::cli::agent_surfaces::AgentIntegrationReport> {
    selected_agents
        .iter()
        .map(|agent| crate::cli::agent_surfaces::AgentIntegrationReport {
            agent: agent.clone(),
            label: super::agent_hooks::agent_display(agent),
            hook_count: planned_hook_count(agent),
            newly_installed_hook_count: 0,
            state: crate::cli::agent_surfaces::AgentIntegrationState::Installed,
        })
        .collect()
}

fn planned_hook_count(agent: &str) -> usize {
    match agent {
        crate::adapters::agents::AGENT_NAME_CLAUDE_CODE => 7,
        crate::adapters::agents::AGENT_NAME_COPILOT => 8,
        crate::adapters::agents::AGENT_NAME_CODEX => 5,
        crate::adapters::agents::AGENT_NAME_CURSOR => 9,
        crate::adapters::agents::AGENT_NAME_GEMINI => 12,
        crate::adapters::agents::AGENT_NAME_OPEN_CODE => 5,
        _ => 0,
    }
}

pub(crate) async fn run_for_project_root(
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
    validate_context_guidance_init_args(&args)?;
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

    maybe_install_default_daemon(args.install_default_daemon, telemetry_choice).await?;
    let daemon_runtime = crate::daemon::runtime_state()?;
    let daemon_service = crate::daemon::service_metadata()?;
    let daemon_config_path = if let Some(runtime) = daemon_runtime.as_ref() {
        Some(
            runtime
                .config_path
                .canonicalize()
                .unwrap_or_else(|_| runtime.config_path.clone()),
        )
    } else if args.install_default_daemon {
        Some(bound_running_daemon_config_path().await?)
    } else {
        None
    };
    if args.install_default_daemon {
        let port = daemon_runtime
            .as_ref()
            .map(|runtime| runtime.port)
            .unwrap_or(crate::api::DEFAULT_DASHBOARD_PORT);
        write_default_daemon_bootstrap(
            out,
            daemon_config_path
                .as_deref()
                .expect("install default daemon should resolve a daemon config path"),
            port,
        )?;
    }
    let should_manage_telemetry_via_daemon =
        args.install_default_daemon || daemon_config_existed_at_entry;
    let should_prompt_for_telemetry = if should_manage_telemetry_via_daemon {
        telemetry_consent::ensure_default_daemon_running().await?;
        if let Some(choice) = telemetry_choice {
            let persisted = telemetry_consent::update_cli_telemetry_consent_via_daemon(
                project_root,
                Some(choice),
            )
            .await?;
            if persisted.needs_prompt {
                bail!("failed to persist telemetry consent");
            }
            false
        } else {
            let state =
                telemetry_consent::update_cli_telemetry_consent_via_daemon(project_root, None)
                    .await?;
            if state.needs_prompt && !telemetry_consent::can_prompt_interactively() {
                bail!(telemetry_consent::NON_INTERACTIVE_TELEMETRY_ERROR);
            }
            state.needs_prompt
        }
    } else {
        false
    };
    let daemon_already_always_on = args.install_default_daemon
        && (daemon_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.mode == crate::daemon::DaemonMode::Service)
            || daemon_service.is_some());
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
    let mut embeddings_bootstrap = None;
    let mut prepared_summary_setup = None;
    let mut login_required = false;
    let embeddings_selection =
        should_install_embeddings_during_init(project_root, &args, out, input)?;
    match embeddings_selection {
        InitEmbeddingsSetupSelection::Existing => {}
        InitEmbeddingsSetupSelection::Cloud => {
            login_required = true;
            let gateway_url =
                platform_embeddings_gateway_url_override(args.embeddings_gateway_url.as_deref());
            if args.install_default_daemon {
                embeddings_bootstrap = Some(resolve_embeddings_bootstrap_request(
                    project_root,
                    InitEmbeddingsSetupSelection::Cloud,
                    gateway_url.as_deref(),
                    Some(args.embeddings_api_key_env.as_str()),
                )?);
            } else {
                for line in install_or_configure_platform_embeddings(
                    project_root,
                    gateway_url.as_deref(),
                    &args.embeddings_api_key_env,
                )? {
                    writeln!(out, "{line}")?;
                }
            }
        }
        InitEmbeddingsSetupSelection::Local => {
            if args.install_default_daemon {
                embeddings_bootstrap = Some(resolve_embeddings_bootstrap_request(
                    project_root,
                    InitEmbeddingsSetupSelection::Local,
                    None,
                    None,
                )?);
            } else {
                install_embeddings_during_init(project_root, out)?;
            }
        }
        InitEmbeddingsSetupSelection::Skip => {}
    }
    let summary_selection = choose_summary_setup_during_init(
        project_root,
        args.install_default_daemon,
        args.no_summaries,
        out,
        input,
    )
    .await?;
    if matches!(summary_selection, SummarySetupSelection::Cloud) {
        login_required = true;
    }
    let context_guidance_selection =
        choose_context_guidance_setup_during_init(project_root, &args, out, input).await?;
    if matches!(
        context_guidance_selection,
        ContextGuidanceSetupSelection::Cloud
    ) {
        login_required = true;
    }
    if login_required {
        crate::cli::login::ensure_logged_in().await?;
    }
    match summary_selection {
        SummarySetupSelection::Cloud => {
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
    match context_guidance_selection {
        ContextGuidanceSetupSelection::Cloud => {
            let api_key_env = args
                .context_guidance_api_key_env
                .as_deref()
                .unwrap_or(crate::cli::inference::DEFAULT_PLATFORM_CONTEXT_GUIDANCE_API_KEY_ENV);
            let gateway_url_override = platform_context_guidance_gateway_url_override(
                args.context_guidance_gateway_url.as_deref(),
            );
            let message = configure_cloud_context_guidance_generation(
                project_root,
                gateway_url_override.as_deref(),
                Some(api_key_env),
            )
            .map_err(|err| {
                anyhow::anyhow!(
                    "Bitloops init completed, but context guidance setup failed: {err:#}"
                )
            })?;
            writeln!(out, "{message}")?;
        }
        ContextGuidanceSetupSelection::Local => {
            configure_local_context_guidance_generation(
                project_root,
                out,
                input,
                telemetry_consent::can_prompt_interactively(),
            )
            .map_err(|err| {
                anyhow::anyhow!(
                    "Bitloops init completed, but context guidance setup failed: {err:#}"
                )
            })?;
        }
        ContextGuidanceSetupSelection::Skip => {}
    }
    let code_embeddings_selected = matches!(
        embeddings_selection,
        InitEmbeddingsSetupSelection::Existing
            | InitEmbeddingsSetupSelection::Cloud
            | InitEmbeddingsSetupSelection::Local
    );
    let summaries_selected =
        prepared_summary_setup.is_some() || summary_generation_configured(project_root);
    let final_setup_selection = choose_final_setup_options(
        args.sync,
        out,
        input,
        effective_ingest,
        InitFinalSetupPromptOptions {
            show_telemetry: should_prompt_for_telemetry,
            show_auto_start_daemon: args.install_default_daemon && !daemon_already_always_on,
        },
    )?;
    if args.install_default_daemon {
        maybe_enable_default_daemon_service(
            final_setup_selection.auto_start_daemon,
            daemon_config_path
                .as_deref()
                .expect("install default daemon should resolve a daemon config path"),
            should_prompt_for_telemetry
                .then_some(final_setup_selection.telemetry)
                .or(telemetry_choice),
        )
        .await?;
    }
    if should_prompt_for_telemetry {
        let persisted = telemetry_consent::update_cli_telemetry_consent_via_daemon(
            project_root,
            Some(final_setup_selection.telemetry),
        )
        .await?;
        if persisted.needs_prompt {
            bail!("failed to persist telemetry consent");
        }
    }
    let should_sync = final_setup_selection.sync;
    let should_ingest = final_setup_selection.ingest;
    set_devql_producer_settings(&local_policy_path, should_sync, should_ingest)?;
    let run_code_embeddings = should_sync && code_embeddings_selected;
    let run_summaries = should_sync && summaries_selected;
    let run_summary_embeddings = run_summaries && code_embeddings_selected;
    if args.install_default_daemon {
        activate_selected_init_mailboxes(
            &git_root,
            run_code_embeddings,
            run_summaries,
            run_summary_embeddings,
        )?;
    }
    if args.install_default_daemon {
        write_init_setup_handoff(
            out,
            InitSetupHandoffOptions {
                run_sync: should_sync,
                run_ingest: should_ingest,
                run_code_embeddings,
                run_summaries,
                run_summary_embeddings,
                prepare_embeddings_runtime: embeddings_bootstrap.is_some() && !run_code_embeddings,
                prepare_summary_generation: prepared_summary_setup.is_some() && !run_summaries,
            },
        )
        .await?;
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
                    run_code_embeddings,
                    run_summaries,
                    run_summary_embeddings,
                    ingest_backfill,
                    embeddings_bootstrap,
                    summaries_bootstrap,
                },
                show_live_progress_notice: !args.install_default_daemon,
            },
        )
        .await?;
    }

    crate::cli::watcher_bootstrap::reconcile_repo_watcher(project_root).map_err(|err| {
        anyhow::anyhow!("Bitloops init completed, but DevQL watcher reconcile failed: {err:#}")
    })?;
    Ok(())
}

async fn bound_running_daemon_config_path() -> Result<std::path::PathBuf> {
    if let Some(runtime) = crate::daemon::runtime_state()? {
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

    let runtime = crate::daemon::runtime_state()?.context("Bitloops daemon is not running")?;
    Ok(runtime
        .config_path
        .canonicalize()
        .unwrap_or(runtime.config_path))
}

fn resolve_embeddings_bootstrap_request(
    repo_root: &Path,
    selection: InitEmbeddingsSetupSelection,
    gateway_url_override: Option<&str>,
    api_key_env: Option<&str>,
) -> Result<crate::cli::devql::graphql::RuntimeEmbeddingsBootstrapRequestInput> {
    let config_path = crate::config::resolve_bound_daemon_config_path_for_repo(repo_root)
        .or_else(|_| crate::config::resolve_daemon_config_path_for_repo(repo_root))
        .unwrap_or_else(|_| repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH));
    match selection {
        InitEmbeddingsSetupSelection::Cloud => {
            let profile_name = crate::config::prepare_daemon_platform_embeddings_install(
                &config_path,
                gateway_url_override,
                api_key_env.unwrap_or("BITLOOPS_PLATFORM_GATEWAY_TOKEN"),
            )?
            .profile_name;
            Ok(
                crate::cli::devql::graphql::RuntimeEmbeddingsBootstrapRequestInput {
                    config_path: config_path.display().to_string(),
                    profile_name,
                    mode: "platform".to_string(),
                    gateway_url_override: gateway_url_override.map(str::to_string),
                    api_key_env: api_key_env.map(str::to_string),
                },
            )
        }
        InitEmbeddingsSetupSelection::Local
        | InitEmbeddingsSetupSelection::Existing
        | InitEmbeddingsSetupSelection::Skip => {
            let profile_name =
                crate::cli::embeddings::embedding_capability_for_config_path(&config_path)
                    .ok()
                    .and_then(|capability| {
                        crate::cli::embeddings::selected_inference_profile_name(&capability)
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "local_code".to_string());

            Ok(
                crate::cli::devql::graphql::RuntimeEmbeddingsBootstrapRequestInput {
                    config_path: config_path.display().to_string(),
                    profile_name,
                    mode: "local".to_string(),
                    gateway_url_override: None,
                    api_key_env: None,
                },
            )
        }
    }
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
    code_embeddings_enabled: bool,
    summaries_enabled: bool,
    summary_embeddings_enabled: bool,
) -> Result<()> {
    activate_selected_pipeline_mailboxes(
        repo_root,
        "init",
        summaries_enabled,
        code_embeddings_enabled,
        summary_embeddings_enabled,
        code_embeddings_enabled || summary_embeddings_enabled,
    )
    .context("activating semantic clones init mailboxes")
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

#[derive(Debug, Clone, Copy)]
struct InitSetupHandoffOptions {
    run_sync: bool,
    run_ingest: bool,
    run_code_embeddings: bool,
    run_summaries: bool,
    run_summary_embeddings: bool,
    prepare_embeddings_runtime: bool,
    prepare_summary_generation: bool,
}

async fn write_init_setup_handoff(
    out: &mut dyn Write,
    options: InitSetupHandoffOptions,
) -> Result<()> {
    let tick = color_hex_if_enabled("✓", SUCCESS_GREEN_HEX);
    let dashboard_url = current_dashboard_url()
        .await?
        .unwrap_or_else(default_dashboard_url_for_init_handoff);
    let mut background_steps = Vec::new();
    if options.run_sync {
        background_steps.push("Syncing your current codebase");
    }
    if options.run_ingest {
        background_steps.push("Ingesting your git history");
    }
    if options.run_code_embeddings {
        background_steps.push("Creating code embeddings for semantic search");
    } else if options.prepare_embeddings_runtime {
        background_steps.push("Preparing the embeddings runtime");
    }
    if options.run_summaries {
        background_steps.push("Generating file and module summaries");
    } else if options.prepare_summary_generation {
        background_steps.push("Preparing summary generation");
    }
    if options.run_summary_embeddings {
        background_steps.push("Creating summary embeddings");
    }

    writeln!(out)?;
    writeln!(
        out,
        "{}",
        color_hex_if_enabled(&bitloops_wordmark(), BITLOOPS_PURPLE_HEX)
    )?;
    writeln!(out)?;
    writeln!(out, "{tick} Setup complete")?;
    writeln!(out)?;
    if background_steps.is_empty() {
        writeln!(
            out,
            "Bitloops is ready. No background indexing steps were selected during setup."
        )?;
        writeln!(out)?;
    } else {
        writeln!(
            out,
            "Bitloops is now continuing the setup you selected in the background."
        )?;
        writeln!(out)?;
        writeln!(out, "What’s happening:")?;
        for step in background_steps {
            writeln!(out, "  • {step}")?;
        }
        writeln!(out)?;
    }
    writeln!(out, "You can:")?;
    writeln!(out, "  • View progress: {dashboard_url}")?;
    writeln!(out, "  • Check status anytime: bitloops init status")?;
    writeln!(
        out,
        "  • Close this terminal — setup will continue in the background"
    )?;
    writeln!(out)?;
    if should_render_local_http_mkcert_notice(&dashboard_url) {
        write_local_http_mkcert_notice(out)?;
    }
    writeln!(
        out,
        "──────────────────────────────────────────────────────────────────"
    )?;
    writeln!(out, "                   🔍 Live Progress")?;
    writeln!(
        out,
        " Feel free to close this terminal and continue with your day! 🌟"
    )?;
    writeln!(
        out,
        "──────────────────────────────────────────────────────────────────"
    )?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

fn default_dashboard_url_for_init_handoff() -> String {
    let scheme = if crate::api::tls::mkcert_on_path() {
        "https"
    } else {
        "http"
    };
    format!(
        "{scheme}://127.0.0.1:{}",
        crate::api::DEFAULT_DASHBOARD_PORT
    )
}

fn should_render_local_http_mkcert_notice(dashboard_url: &str) -> bool {
    dashboard_url.starts_with("http://") && !crate::api::tls::mkcert_on_path()
}

fn write_local_http_mkcert_notice(out: &mut dyn Write) -> Result<()> {
    writeln!(
        out,
        "Notice: local dashboard HTTPS is unavailable because `mkcert` is not on your PATH."
    )?;
    writeln!(
        out,
        "Install `mkcert`, run `mkcert -install`, then run `bitloops daemon start --recheck-local-dashboard-net`."
    )?;
    writeln!(
        out,
        "Guide: {}",
        crate::api::tls::LOCAL_HTTPS_SETUP_DOCS_URL
    )?;
    writeln!(out)?;
    Ok(())
}

async fn current_dashboard_url() -> Result<Option<String>> {
    Ok(crate::daemon::runtime_state()?.map(|runtime| runtime.url))
}
