use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::progress::{InitProgressOptions, run_dual_init_progress};
use super::workflow_output::{
    InitSetupHandoffOptions, planned_integrations, write_default_daemon_bootstrap,
    write_init_setup_handoff, write_integrations_installed, write_integrations_installing,
};
use super::{
    AgentSelector, DEFAULT_INIT_INGEST_BACKFILL, InitAgentSelection, InitArgs,
    InitEmbeddingsSetupSelection, InitFinalSetupPromptOptions,
    choose_context_guidance_setup_during_init, choose_final_setup_options,
    choose_summary_setup_during_init, detect_or_select_agent, ensure_repo_init_files_excluded,
    maybe_enable_default_daemon_service, maybe_install_default_daemon, normalize_cli_exclusions,
    normalize_exclude_from_paths, should_install_embeddings_during_init,
};
use crate::adapters::agents::AgentAdapterRegistry;
use crate::capability_packs::semantic_clones::workplane::{
    activate_selected_pipeline_mailboxes, deactivate_embedding_pipeline_mailboxes,
};
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
    prepare_local_summary_generation_plan,
};
use crate::cli::telemetry_consent;
use crate::config::settings::{
    DEFAULT_STRATEGY, load_settings, repo_semantic_embedding_policy,
    restore_repo_semantic_embedding_policy, set_devql_producer_settings,
    set_repo_semantic_embedding_policy, set_scope_exclusions,
    write_project_bootstrap_settings_with_daemon_binding_and_devql_guidance,
};
use crate::config::{
    DaemonEmbeddingsInstallPlan, InferenceProfileConfig, InferenceTask,
    REPO_POLICY_LOCAL_FILE_NAME, RepoSemanticEmbeddingPolicy, SemanticCloneEmbeddingMode,
    default_daemon_config_exists, prepare_daemon_local_embeddings_profile_install,
    resolve_semantic_clones_config_for_repo,
};

struct PreparedEmbeddingsBootstrapRequest {
    request: crate::cli::devql::graphql::RuntimeEmbeddingsBootstrapRequestInput,
    rollback_plan: Option<DaemonEmbeddingsInstallPlan>,
}

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
    if let Some(target_daemon_config_path) = daemon_config_path.as_deref() {
        let source_daemon_config_path =
            crate::config::resolve_preferred_daemon_config_path_for_repo(project_root)
                .ok()
                .filter(|path| path.is_file())
                .or_else(|| {
                    target_daemon_config_path
                        .is_file()
                        .then(|| target_daemon_config_path.to_path_buf())
                });
        if let Some(source_daemon_config_path) = source_daemon_config_path {
            crate::config::persist_daemon_store_backend_selection(
                &source_daemon_config_path,
                target_daemon_config_path,
            )?;
        }
    }
    let local_policy_path = project_root.join(REPO_POLICY_LOCAL_FILE_NAME);
    let previous_embeddings_policy = repo_semantic_embedding_policy(project_root)?;
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
            show_telemetry: should_prompt_for_telemetry,
            show_auto_start_daemon: args.install_default_daemon && !daemon_already_always_on,
        },
    )?;
    let mut embeddings_bootstrap = None;
    let mut embeddings_bootstrap_rollback_plan = None;
    let mut prepared_summary_setup = None;
    let mut selected_code_embedding_profile_name: Option<String> = None;
    let mut selected_summary_embedding_profile_name: Option<String> = None;
    let mut selected_summary_generation_profile_name: Option<String> = None;
    let mut login_required = false;
    let embeddings_selection = should_install_embeddings_during_init(
        project_root,
        &args,
        !args.no_embeddings,
        out,
        input,
    )?;
    match embeddings_selection {
        InitEmbeddingsSetupSelection::Unchanged => {}
        InitEmbeddingsSetupSelection::Existing => {
            let profile_names = existing_init_embedding_profile_names(project_root)?;
            selected_code_embedding_profile_name = profile_names.code_embeddings;
            selected_summary_embedding_profile_name = profile_names.summary_embeddings;
        }
        InitEmbeddingsSetupSelection::Cloud => {
            login_required = true;
            let gateway_url =
                platform_embeddings_gateway_url_override(args.embeddings_gateway_url.as_deref());
            if args.install_default_daemon {
                let prepared_bootstrap = resolve_embeddings_bootstrap_request(
                    project_root,
                    InitEmbeddingsSetupSelection::Cloud,
                    gateway_url.as_deref(),
                    Some(args.embeddings_api_key_env.as_str()),
                )?;
                selected_code_embedding_profile_name =
                    Some(prepared_bootstrap.request.profile_name.clone());
                selected_summary_embedding_profile_name =
                    Some(prepared_bootstrap.request.profile_name.clone());
                embeddings_bootstrap_rollback_plan = prepared_bootstrap.rollback_plan;
                embeddings_bootstrap = Some(prepared_bootstrap.request);
            } else {
                for line in install_or_configure_platform_embeddings(
                    project_root,
                    gateway_url.as_deref(),
                    &args.embeddings_api_key_env,
                )? {
                    writeln!(out, "{line}")?;
                }
                let profile_names = existing_init_embedding_profile_names(project_root)?;
                selected_code_embedding_profile_name = profile_names.code_embeddings;
                selected_summary_embedding_profile_name = profile_names.summary_embeddings;
            }
        }
        InitEmbeddingsSetupSelection::Local => {
            if args.install_default_daemon {
                let prepared_bootstrap = resolve_embeddings_bootstrap_request(
                    project_root,
                    InitEmbeddingsSetupSelection::Local,
                    None,
                    None,
                )?;
                selected_code_embedding_profile_name =
                    Some(prepared_bootstrap.request.profile_name.clone());
                selected_summary_embedding_profile_name =
                    Some(prepared_bootstrap.request.profile_name.clone());
                embeddings_bootstrap_rollback_plan = prepared_bootstrap.rollback_plan;
                embeddings_bootstrap = Some(prepared_bootstrap.request);
            } else {
                install_embeddings_during_init(project_root, out)?;
                let profile_names = existing_init_embedding_profile_names(project_root)?;
                selected_code_embedding_profile_name = profile_names.code_embeddings;
                selected_summary_embedding_profile_name = profile_names.summary_embeddings;
            }
        }
        InitEmbeddingsSetupSelection::Skip => {}
    }
    let summary_selection = choose_summary_setup_during_init(
        project_root,
        args.install_default_daemon,
        args.no_summaries,
        !args.no_summaries,
        out,
        input,
    )
    .await?;
    if matches!(summary_selection, SummarySetupSelection::Cloud) {
        login_required = true;
    }
    if !args.no_summaries {
        selected_summary_generation_profile_name =
            existing_init_summary_generation_profile_name(project_root);
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
                let plan = prepare_cloud_summary_generation_plan(gateway_url_override.as_deref());
                selected_summary_generation_profile_name =
                    prepared_summary_generation_profile_name(project_root, &plan);
                prepared_summary_setup = Some(plan);
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
                selected_summary_generation_profile_name =
                    existing_init_summary_generation_profile_name(project_root);
            }
        }
        SummarySetupSelection::Local => {
            if args.install_default_daemon {
                let plan = prepare_local_summary_generation_plan(
                    out,
                    input,
                    telemetry_consent::can_prompt_interactively(),
                )
                .map_err(|err| {
                    anyhow::anyhow!(
                        "Bitloops init completed, but semantic summary setup failed: {err:#}"
                    )
                })?;
                selected_summary_generation_profile_name =
                    prepared_summary_generation_profile_name(project_root, &plan);
                prepared_summary_setup = Some(plan);
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
                selected_summary_generation_profile_name =
                    existing_init_summary_generation_profile_name(project_root);
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
    let semantic_selection = InitRepoSemanticSelection {
        code_embeddings: !args.no_embeddings && selected_code_embedding_profile_name.is_some(),
        summaries: !args.no_summaries && selected_summary_generation_profile_name.is_some(),
        summary_embeddings: !args.no_embeddings
            && !args.no_summaries
            && selected_summary_generation_profile_name.is_some()
            && selected_summary_embedding_profile_name.is_some(),
    };
    persist_init_semantic_policy(
        &local_policy_path,
        semantic_selection,
        selected_code_embedding_profile_name.as_deref(),
        selected_summary_embedding_profile_name.as_deref(),
        selected_summary_generation_profile_name.as_deref(),
    )?;
    let summaries_selected = semantic_selection.summaries;
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
    let semantic_policy = resolve_semantic_clones_config_for_repo(project_root);
    let code_embeddings_selected = semantic_selection.code_embeddings
        && semantic_policy.embedding_mode != SemanticCloneEmbeddingMode::Off
        && semantic_policy
            .inference
            .code_embeddings
            .as_deref()
            .is_some_and(|profile| !profile.trim().is_empty());
    let summary_embeddings_selected = semantic_selection.summary_embeddings
        && semantic_policy.embedding_mode != SemanticCloneEmbeddingMode::Off
        && semantic_policy
            .inference
            .summary_embeddings
            .as_deref()
            .is_some_and(|profile| !profile.trim().is_empty());
    let run_code_embeddings = should_sync && code_embeddings_selected;
    let run_summaries = should_sync && summaries_selected;
    let run_summary_embeddings = run_summaries && summary_embeddings_selected;
    if args.install_default_daemon {
        activate_selected_init_mailboxes(
            &git_root,
            run_code_embeddings,
            run_summaries,
            run_summary_embeddings,
        )?;
    }
    crate::cli::watcher_bootstrap::reconcile_repo_watcher(project_root)
        .await
        .map_err(|err| {
            anyhow::anyhow!("Bitloops init completed, but DevQL watcher reconcile failed: {err:#}")
        })?;

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
        let has_embeddings_bootstrap = embeddings_bootstrap.is_some();
        let scope = crate::devql_transport::discover_slim_cli_repo_scope(Some(project_root))?;
        let ingest_backfill =
            should_ingest.then_some(args.backfill.unwrap_or(DEFAULT_INIT_INGEST_BACKFILL));
        let summaries_bootstrap = prepared_summary_setup
            .as_ref()
            .map(runtime_summary_bootstrap_request_from_plan);
        let init_progress_result = run_dual_init_progress(
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
        .await;
        if let Err(err) = init_progress_result {
            if has_embeddings_bootstrap {
                if let Some(plan) = embeddings_bootstrap_rollback_plan.as_ref() {
                    plan.rollback().with_context(|| {
                        format!(
                            "rolling back daemon embeddings config after failed init bootstrap: {err:#}"
                        )
                    })?;
                }
                restore_repo_semantic_embedding_policy(
                    &local_policy_path,
                    &previous_embeddings_policy,
                )
                .with_context(|| {
                    format!("restoring repo embedding policy after failed init bootstrap: {err:#}")
                })?;
                deactivate_embedding_pipeline_mailboxes(
                    project_root,
                    "init_embeddings_bootstrap_rollback",
                )
                .with_context(|| {
                    format!("deactivating embedding mailboxes after failed init bootstrap: {err:#}")
                })?;
            }
            return Err(err);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct InitRepoSemanticSelection {
    code_embeddings: bool,
    summaries: bool,
    summary_embeddings: bool,
}

fn init_repo_selected_embedding_lanes(selection: InitRepoSemanticSelection) -> bool {
    selection.code_embeddings || selection.summary_embeddings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_bound_daemon_config(repo_root: &Path, contents: &str) {
        let config_path = repo_root
            .join("daemon")
            .join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH);
        std::fs::create_dir_all(config_path.parent().expect("config parent"))
            .expect("create config parent");
        std::fs::write(&config_path, contents).expect("write daemon config");
        crate::config::settings::write_repo_daemon_binding(
            &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
            &config_path,
        )
        .expect("write daemon binding");
    }

    fn semantic_selection(
        code_embeddings: bool,
        summaries: bool,
        summary_embeddings: bool,
    ) -> InitRepoSemanticSelection {
        InitRepoSemanticSelection {
            code_embeddings,
            summaries,
            summary_embeddings,
        }
    }

    #[test]
    fn repo_selected_embedding_lanes_tracks_semantic_embedding_choices() {
        let summary_only = semantic_selection(false, true, true);

        assert!(init_repo_selected_embedding_lanes(summary_only));

        let code_embeddings = semantic_selection(true, false, false);

        assert!(init_repo_selected_embedding_lanes(code_embeddings));
    }

    #[test]
    fn prepared_local_summary_profile_name_uses_suffix_when_default_is_not_managed() {
        let repo = tempfile::tempdir().expect("tempdir");
        write_bound_daemon_config(
            repo.path(),
            r#"
[inference.profiles.summary_local]
task = "embeddings"
driver = "custom_driver"
runtime = "custom_runtime"
model = "custom-model"
"#,
        );
        let plan = crate::cli::inference::PreparedSummarySetupPlan::new(
            PreparedSummarySetupAction::ConfigureLocal {
                model_name: "model".to_string(),
            },
        );

        assert_eq!(
            prepared_summary_generation_profile_name(repo.path(), &plan),
            Some("summary_local_1".to_string())
        );
    }

    #[test]
    fn prepared_platform_summary_profile_name_uses_suffix_when_default_is_not_managed() {
        let repo = tempfile::tempdir().expect("tempdir");
        write_bound_daemon_config(
            repo.path(),
            r#"
[inference.profiles.summary_llm]
task = "text_generation"
driver = "ollama_chat"
runtime = "bitloops_inference"
model = "local-model"
"#,
        );
        let plan = crate::cli::inference::PreparedSummarySetupPlan::new(
            PreparedSummarySetupAction::ConfigureCloud {
                gateway_url_override: None,
                api_key_env: None,
            },
        );

        assert_eq!(
            prepared_summary_generation_profile_name(repo.path(), &plan),
            Some("summary_llm_1".to_string())
        );
    }

    #[test]
    fn existing_init_embedding_profile_names_fills_missing_slots_from_daemon_config() {
        let repo = tempfile::tempdir().expect("tempdir");
        write_bound_daemon_config(
            repo.path(),
            r#"
[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
code_embeddings = "daemon_code"
summary_embeddings = "daemon_summary"

[inference.profiles.daemon_code]
task = "embeddings"
driver = "openai"
model = "text-embedding-3-large"

[inference.profiles.daemon_summary]
task = "embeddings"
driver = "openai"
model = "text-embedding-3-small"
"#,
        );
        let local_policy_path = repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME);
        let mut local_policy = std::fs::OpenOptions::new()
            .append(true)
            .open(&local_policy_path)
            .expect("open local policy");
        write!(
            local_policy,
            r#"

[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
code_embeddings = "repo_code"
"#
        )
        .expect("write local semantic policy");

        let names =
            existing_init_embedding_profile_names(repo.path()).expect("resolve profile names");

        assert_eq!(
            names,
            InitEmbeddingProfileNames {
                code_embeddings: Some("repo_code".to_string()),
                summary_embeddings: Some("daemon_summary".to_string()),
            }
        );
    }
}

fn persist_init_semantic_policy(
    local_policy_path: &Path,
    selection: InitRepoSemanticSelection,
    code_embedding_profile_name: Option<&str>,
    summary_embedding_profile_name: Option<&str>,
    summary_generation_profile_name: Option<&str>,
) -> Result<()> {
    let summaries = selection.summaries && summary_generation_profile_name.is_some();
    let code_embeddings = selection.code_embeddings && code_embedding_profile_name.is_some();
    let summary_embeddings = selection.summary_embeddings
        && selection.summaries
        && summary_embedding_profile_name.is_some();
    let embedding_mode = if code_embeddings || summary_embeddings {
        Some(SemanticCloneEmbeddingMode::SemanticAwareOnce)
    } else {
        Some(SemanticCloneEmbeddingMode::Off)
    };

    set_repo_semantic_embedding_policy(
        local_policy_path,
        &RepoSemanticEmbeddingPolicy {
            present: true,
            summary_mode: Some(if summaries {
                crate::config::SemanticSummaryMode::Auto
            } else {
                crate::config::SemanticSummaryMode::Off
            }),
            embedding_mode,
            inference: crate::config::SemanticClonesInferenceBindings {
                summary_generation: summaries.then(|| {
                    summary_generation_profile_name
                        .expect("summary profile checked above")
                        .to_string()
                }),
                code_embeddings: code_embeddings.then(|| {
                    code_embedding_profile_name
                        .expect("code embedding profile checked above")
                        .to_string()
                }),
                summary_embeddings: summary_embeddings.then(|| {
                    summary_embedding_profile_name
                        .expect("summary embedding profile checked above")
                        .to_string()
                }),
            },
        },
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct InitEmbeddingProfileNames {
    code_embeddings: Option<String>,
    summary_embeddings: Option<String>,
}

impl InitEmbeddingProfileNames {
    fn has_any(&self) -> bool {
        self.code_embeddings.is_some() || self.summary_embeddings.is_some()
    }
}

fn existing_init_embedding_profile_names(repo_root: &Path) -> Result<InitEmbeddingProfileNames> {
    let existing_policy = repo_semantic_embedding_policy(repo_root)?;
    let mut profile_names = embedding_profile_names_from_policy(&existing_policy);
    if profile_names.code_embeddings.is_some() && profile_names.summary_embeddings.is_some() {
        return Ok(profile_names);
    }

    let daemon_profile_names = match daemon_init_embedding_profile_names(repo_root) {
        Ok(profile_names) => profile_names,
        Err(_) if profile_names.has_any() => return Ok(profile_names),
        Err(err) => return Err(err),
    };
    if profile_names.code_embeddings.is_none() {
        profile_names.code_embeddings = daemon_profile_names.code_embeddings;
    }
    if profile_names.summary_embeddings.is_none() {
        profile_names.summary_embeddings = daemon_profile_names.summary_embeddings;
    }
    Ok(profile_names)
}

fn daemon_init_embedding_profile_names(repo_root: &Path) -> Result<InitEmbeddingProfileNames> {
    let config_path = crate::config::resolve_bound_daemon_config_path_for_repo(repo_root)
        .or_else(|_| crate::config::resolve_daemon_config_path_for_repo(repo_root))?;
    let capability = crate::cli::embeddings::embedding_capability_for_config_path(&config_path)?;
    let mut profile_names = InitEmbeddingProfileNames {
        code_embeddings: trimmed_profile_name(
            capability
                .semantic_clones
                .inference
                .code_embeddings
                .as_deref(),
        ),
        summary_embeddings: trimmed_profile_name(
            capability
                .semantic_clones
                .inference
                .summary_embeddings
                .as_deref(),
        ),
    };
    if !profile_names.has_any()
        && let Some(profile_name) =
            crate::cli::embeddings::selected_inference_profile_name(&capability)
    {
        let profile_name = profile_name.to_string();
        profile_names.code_embeddings = Some(profile_name.clone());
        profile_names.summary_embeddings = Some(profile_name);
    }
    Ok(profile_names)
}

fn embedding_profile_names_from_policy(
    policy: &RepoSemanticEmbeddingPolicy,
) -> InitEmbeddingProfileNames {
    InitEmbeddingProfileNames {
        code_embeddings: trimmed_profile_name(policy.inference.code_embeddings.as_deref()),
        summary_embeddings: trimmed_profile_name(policy.inference.summary_embeddings.as_deref()),
    }
}

fn trimmed_profile_name(profile_name: Option<&str>) -> Option<String> {
    profile_name
        .map(str::trim)
        .filter(|profile| !profile.is_empty())
        .map(str::to_string)
}

fn existing_init_summary_generation_profile_name(repo_root: &Path) -> Option<String> {
    let capability = crate::config::resolve_inference_capability_config_for_repo(repo_root);
    capability
        .semantic_clones
        .inference
        .summary_generation
        .as_deref()
        .map(str::trim)
        .filter(|profile| !profile.is_empty())
        .map(str::to_string)
}

#[derive(Clone, Copy)]
enum PreparedSummaryProfileKind {
    Local,
    Platform,
}

fn prepared_summary_generation_profile_name(
    repo_root: &Path,
    plan: &crate::cli::inference::PreparedSummarySetupPlan,
) -> Option<String> {
    match plan.action() {
        PreparedSummarySetupAction::ConfigureCloud { .. } => Some(planned_summary_profile_name(
            repo_root,
            "summary_llm",
            PreparedSummaryProfileKind::Platform,
        )),
        PreparedSummarySetupAction::ConfigureLocal { .. } => Some(planned_summary_profile_name(
            repo_root,
            "summary_local",
            PreparedSummaryProfileKind::Local,
        )),
        PreparedSummarySetupAction::InstallRuntimeOnly { .. }
        | PreparedSummarySetupAction::InstallRuntimeOnlyPendingProbe { .. } => None,
    }
}

fn planned_summary_profile_name(
    repo_root: &Path,
    default_name: &str,
    kind: PreparedSummaryProfileKind,
) -> String {
    let capability = crate::config::resolve_inference_capability_config_for_repo(repo_root);
    let profiles = &capability.inference.profiles;

    match profiles.get(default_name) {
        None => default_name.to_string(),
        Some(profile) if is_managed_prepared_summary_profile(profile, kind) => {
            default_name.to_string()
        }
        Some(_) => next_available_summary_profile_name(profiles, default_name),
    }
}

fn next_available_summary_profile_name(
    profiles: &std::collections::BTreeMap<String, InferenceProfileConfig>,
    prefix: &str,
) -> String {
    let mut suffix = 1usize;
    loop {
        let candidate = format!("{prefix}_{suffix}");
        if !profiles.contains_key(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn is_managed_prepared_summary_profile(
    profile: &InferenceProfileConfig,
    kind: PreparedSummaryProfileKind,
) -> bool {
    match kind {
        PreparedSummaryProfileKind::Local => is_managed_local_summary_profile(profile),
        PreparedSummaryProfileKind::Platform => is_managed_platform_summary_profile(profile),
    }
}

fn is_managed_local_summary_profile(profile: &InferenceProfileConfig) -> bool {
    profile.task == InferenceTask::TextGeneration
        && profile.runtime.as_deref().map(str::trim) == Some("bitloops_inference")
        && profile.driver.trim() == "ollama_chat"
}

fn is_managed_platform_summary_profile(profile: &InferenceProfileConfig) -> bool {
    profile.task == InferenceTask::TextGeneration
        && profile.runtime.as_deref().map(str::trim) == Some("bitloops_inference")
        && matches!(
            profile.driver.trim(),
            "bitloops_platform_chat" | "openai_chat_completions"
        )
        && profile
            .api_key
            .as_deref()
            .map(str::trim)
            .is_none_or(|api_key| api_key == "${BITLOOPS_PLATFORM_GATEWAY_TOKEN}")
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
) -> Result<PreparedEmbeddingsBootstrapRequest> {
    let config_path = crate::config::resolve_bound_daemon_config_path_for_repo(repo_root)
        .or_else(|_| crate::config::resolve_daemon_config_path_for_repo(repo_root))
        .unwrap_or_else(|_| repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH));
    match selection {
        InitEmbeddingsSetupSelection::Unchanged => {
            bail!("cannot resolve embeddings bootstrap request without an embeddings decision")
        }
        InitEmbeddingsSetupSelection::Cloud => {
            let plan = crate::config::prepare_daemon_platform_embeddings_install(
                &config_path,
                gateway_url_override,
                api_key_env.unwrap_or("BITLOOPS_PLATFORM_GATEWAY_TOKEN"),
            )?;
            plan.apply()?;
            let profile_name = plan.profile_name.clone();
            Ok(PreparedEmbeddingsBootstrapRequest {
                request: crate::cli::devql::graphql::RuntimeEmbeddingsBootstrapRequestInput {
                    config_path: config_path.display().to_string(),
                    profile_name,
                    mode: "platform".to_string(),
                    gateway_url_override: gateway_url_override.map(str::to_string),
                    api_key_env: api_key_env.map(str::to_string),
                },
                rollback_plan: Some(plan),
            })
        }
        InitEmbeddingsSetupSelection::Local
        | InitEmbeddingsSetupSelection::Existing
        | InitEmbeddingsSetupSelection::Skip => {
            let profile_name = match selection {
                InitEmbeddingsSetupSelection::Local => {
                    let plan = prepare_daemon_local_embeddings_profile_install(&config_path)?;
                    plan.apply()?;
                    let profile_name = plan.profile_name.clone();
                    return Ok(PreparedEmbeddingsBootstrapRequest {
                        request:
                            crate::cli::devql::graphql::RuntimeEmbeddingsBootstrapRequestInput {
                                config_path: config_path.display().to_string(),
                                profile_name,
                                mode: "local".to_string(),
                                gateway_url_override: None,
                                api_key_env: None,
                            },
                        rollback_plan: Some(plan),
                    });
                }
                _ => crate::cli::embeddings::embedding_capability_for_config_path(&config_path)
                    .ok()
                    .and_then(|capability| {
                        crate::cli::embeddings::selected_inference_profile_name(&capability)
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "local_code".to_string()),
            };

            Ok(PreparedEmbeddingsBootstrapRequest {
                request: crate::cli::devql::graphql::RuntimeEmbeddingsBootstrapRequestInput {
                    config_path: config_path.display().to_string(),
                    profile_name,
                    mode: "local".to_string(),
                    gateway_url_override: None,
                    api_key_env: None,
                },
                rollback_plan: None,
            })
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
                api_key_env: None,
            }
        }
        PreparedSummarySetupAction::InstallRuntimeOnlyPendingProbe { message } => {
            crate::cli::devql::graphql::RuntimeSummaryBootstrapRequestInput {
                action: "install_runtime_only_pending_probe".to_string(),
                message: Some(message.clone()),
                model_name: None,
                gateway_url_override: None,
                api_key_env: None,
            }
        }
        PreparedSummarySetupAction::ConfigureLocal { model_name } => {
            crate::cli::devql::graphql::RuntimeSummaryBootstrapRequestInput {
                action: "configure_local".to_string(),
                message: None,
                model_name: Some(model_name.clone()),
                gateway_url_override: None,
                api_key_env: None,
            }
        }
        PreparedSummarySetupAction::ConfigureCloud {
            gateway_url_override,
            api_key_env,
        } => crate::cli::devql::graphql::RuntimeSummaryBootstrapRequestInput {
            action: "configure_cloud".to_string(),
            message: None,
            model_name: None,
            gateway_url_override: gateway_url_override.clone(),
            api_key_env: api_key_env.clone(),
        },
    }
}
