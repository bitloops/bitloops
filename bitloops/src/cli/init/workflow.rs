use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::progress::{InitProgressOptions, run_dual_init_progress};
use super::workflow_output::{
    planned_integrations, write_integrations_installed, write_integrations_installing,
};
use super::{
    AgentSelector, DEFAULT_INIT_INGEST_BACKFILL, InitAgentSelection, InitArgs,
    InitEmbeddingsSetupSelection, InitFinalSetupPromptOptions, InitSummaryEmbeddingsSetupSelection,
    choose_embeddings_setup_during_init, choose_final_setup_options,
    choose_summary_embeddings_setup_during_init, choose_summary_setup_during_init,
    detect_or_select_agent, ensure_repo_init_files_excluded, normalize_cli_exclusions,
    normalize_exclude_from_paths,
};
use crate::adapters::agents::AgentAdapterRegistry;
use crate::cli::embeddings::{
    EmbeddingsInstallState, inspect_embeddings_install_state, install_or_bootstrap_embeddings,
    install_or_configure_platform_embeddings, platform_embeddings_gateway_url_override,
};
use crate::cli::inference::{
    SummarySetupSelection, configure_cloud_summary_generation, configure_local_summary_generation,
    platform_summary_gateway_url_override, summary_generation_configured,
};
use crate::cli::telemetry_consent;
use crate::config::REPO_POLICY_LOCAL_FILE_NAME;
use crate::config::settings::{
    DEFAULT_STRATEGY, load_settings, repo_semantic_embedding_policy, set_devql_producer_settings,
    set_repo_semantic_embedding_policy, set_scope_exclusions,
    write_project_bootstrap_settings_with_daemon_binding_and_devql_guidance,
};
use crate::config::{
    RepoSemanticEmbeddingPolicy, SemanticCloneEmbeddingMode, SemanticClonesInferenceBindings,
    SemanticSummaryMode,
};

const DEFAULT_INIT_CODE_EMBEDDINGS_PROFILE: &str = "platform_code";
const DEFAULT_INIT_SUMMARY_GENERATION_PROFILE: &str = "summary_llm";
const DEFAULT_INIT_SUMMARY_EMBEDDINGS_PROFILE: &str = "platform_code";

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
    let semantic_policy =
        configure_init_semantic_policy(&local_policy_path, project_root, out, input).await?;

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
    let semantic_selection = init_semantic_runtime_selection(should_sync, &semantic_policy);

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
                    run_code_embeddings: semantic_selection.run_code_embeddings,
                    run_summaries: semantic_selection.run_summaries,
                    run_summary_embeddings: semantic_selection.run_summary_embeddings,
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

fn ensure_init_semantic_policy(
    local_policy_path: &Path,
    project_root: &Path,
) -> Result<RepoSemanticEmbeddingPolicy> {
    let existing = repo_semantic_embedding_policy(project_root)?;
    if existing.present {
        return Ok(existing);
    }

    let policy = default_init_semantic_policy();
    set_repo_semantic_embedding_policy(local_policy_path, &policy)?;
    Ok(policy)
}

async fn configure_init_semantic_policy(
    local_policy_path: &Path,
    project_root: &Path,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<RepoSemanticEmbeddingPolicy> {
    let existing = repo_semantic_embedding_policy(project_root)?;
    if existing.present && !init_semantic_policy_needs_provider_setup(project_root, &existing) {
        return Ok(existing);
    }

    if !telemetry_consent::can_prompt_interactively() {
        if existing.present {
            return Ok(existing);
        }
        return ensure_init_semantic_policy(local_policy_path, project_root);
    }

    let repo_selected_embedding_lanes = if existing.present {
        init_semantic_policy_should_offer_embedding_setup(&existing)
    } else {
        true
    };
    let repo_selected_summaries = if existing.present {
        init_semantic_policy_should_offer_summary_setup(&existing)
    } else {
        true
    };
    let mut selected_code_embeddings = None;
    let mut selected_summary_generation = (repo_selected_summaries
        && init_repo_summary_generation_configured(project_root))
    .then(|| current_summary_generation_profile(project_root))
    .flatten();
    let mut selected_summary_embeddings = None;
    let mut login_required = false;

    let embeddings_selection = choose_embeddings_setup_during_init(
        project_root,
        repo_selected_embedding_lanes,
        out,
        input,
    )?;
    match embeddings_selection {
        InitEmbeddingsSetupSelection::Unchanged => {}
        InitEmbeddingsSetupSelection::Existing => {
            selected_code_embeddings = current_code_embeddings_profile(project_root);
            selected_summary_embeddings = current_summary_embeddings_profile(project_root);
        }
        InitEmbeddingsSetupSelection::Cloud => {
            login_required = true;
            let gateway_url = platform_embeddings_gateway_url_override(None);
            for line in install_or_configure_platform_embeddings(
                project_root,
                gateway_url.as_deref(),
                "BITLOOPS_PLATFORM_GATEWAY_TOKEN",
            )? {
                writeln!(out, "{line}")?;
            }
            selected_code_embeddings = current_code_embeddings_profile(project_root);
        }
        InitEmbeddingsSetupSelection::Local => {
            for line in install_or_bootstrap_embeddings(project_root)? {
                writeln!(out, "{line}")?;
            }
            selected_code_embeddings = current_code_embeddings_profile(project_root);
        }
        InitEmbeddingsSetupSelection::Skip => {}
    }

    let summary_selection =
        choose_summary_setup_during_init(project_root, repo_selected_summaries, out, input)?;
    match summary_selection {
        SummarySetupSelection::Cloud => {
            login_required = true;
            let gateway_url = platform_summary_gateway_url_override();
            let message = configure_cloud_summary_generation(project_root, gateway_url.as_deref())
                .map_err(|err| {
                    anyhow::anyhow!(
                        "Bitloops init completed, but semantic summary setup failed: {err:#}"
                    )
                })?;
            writeln!(out, "{message}")?;
            selected_summary_generation = current_summary_generation_profile(project_root);
        }
        SummarySetupSelection::Local => {
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
            selected_summary_generation = current_summary_generation_profile(project_root);
        }
        SummarySetupSelection::Skip => {}
    }

    let summary_embedding_candidate = selected_code_embeddings.clone();
    if selected_summary_generation.is_some() {
        let summary_embeddings_selection = choose_summary_embeddings_setup_during_init(
            repo_selected_summaries,
            selected_summary_embeddings.is_some(),
            summary_embedding_candidate.as_deref(),
            out,
            input,
        )?;
        match summary_embeddings_selection {
            InitSummaryEmbeddingsSetupSelection::Existing => {}
            InitSummaryEmbeddingsSetupSelection::UseSelectedEmbeddingsProvider => {
                selected_summary_embeddings = summary_embedding_candidate;
            }
            InitSummaryEmbeddingsSetupSelection::Cloud => {
                login_required = true;
                let gateway_url = platform_embeddings_gateway_url_override(None);
                for line in install_or_configure_platform_embeddings(
                    project_root,
                    gateway_url.as_deref(),
                    "BITLOOPS_PLATFORM_GATEWAY_TOKEN",
                )? {
                    writeln!(out, "{line}")?;
                }
                selected_summary_embeddings = current_code_embeddings_profile(project_root);
            }
            InitSummaryEmbeddingsSetupSelection::Local => {
                for line in install_or_bootstrap_embeddings(project_root)? {
                    writeln!(out, "{line}")?;
                }
                selected_summary_embeddings = current_code_embeddings_profile(project_root);
            }
            InitSummaryEmbeddingsSetupSelection::Skip => {}
        }
    }

    if login_required {
        crate::cli::login::ensure_logged_in().await?;
    }

    persist_init_semantic_policy(
        local_policy_path,
        selected_code_embeddings.as_deref(),
        selected_summary_generation.as_deref(),
        selected_summary_embeddings.as_deref(),
    )
}

fn init_semantic_policy_needs_provider_setup(
    project_root: &Path,
    policy: &RepoSemanticEmbeddingPolicy,
) -> bool {
    let embeddings_missing = init_semantic_policy_should_offer_embedding_setup(policy)
        && matches!(
            inspect_embeddings_install_state(project_root),
            EmbeddingsInstallState::NotConfigured
        )
        || policy.embedding_mode == Some(SemanticCloneEmbeddingMode::Off);
    let summaries_missing = init_semantic_policy_should_offer_summary_setup(policy)
        && (policy.summary_mode == Some(SemanticSummaryMode::Off)
            || !init_repo_summary_generation_configured(project_root));
    let summary_embeddings_missing = init_semantic_policy_selects_summaries(policy)
        && !non_empty_profile(policy.inference.summary_embeddings.as_deref());

    embeddings_missing || summaries_missing || summary_embeddings_missing
}

fn init_semantic_policy_should_offer_embedding_setup(policy: &RepoSemanticEmbeddingPolicy) -> bool {
    policy.embedding_mode == Some(SemanticCloneEmbeddingMode::Off)
        || init_semantic_policy_selects_embedding_lanes(policy)
}

fn init_semantic_policy_should_offer_summary_setup(policy: &RepoSemanticEmbeddingPolicy) -> bool {
    policy.summary_mode == Some(SemanticSummaryMode::Off)
        || init_semantic_policy_selects_summaries(policy)
}

fn init_semantic_policy_selects_embedding_lanes(policy: &RepoSemanticEmbeddingPolicy) -> bool {
    policy.embedding_mode != Some(SemanticCloneEmbeddingMode::Off)
        && (policy.embedding_mode.is_some()
            || non_empty_profile(policy.inference.code_embeddings.as_deref())
            || non_empty_profile(policy.inference.summary_embeddings.as_deref()))
}

fn init_semantic_policy_selects_summaries(policy: &RepoSemanticEmbeddingPolicy) -> bool {
    policy.summary_mode != Some(SemanticSummaryMode::Off)
        && (policy.summary_mode.is_some()
            || non_empty_profile(policy.inference.summary_generation.as_deref()))
}

fn init_repo_summary_generation_configured(project_root: &Path) -> bool {
    current_summary_generation_profile(project_root).is_some()
        && summary_generation_configured(project_root)
}

fn persist_init_semantic_policy(
    local_policy_path: &Path,
    code_embeddings: Option<&str>,
    summary_generation: Option<&str>,
    summary_embeddings: Option<&str>,
) -> Result<RepoSemanticEmbeddingPolicy> {
    let embeddings_enabled = code_embeddings.is_some() || summary_embeddings.is_some();
    let summaries_enabled = summary_generation.is_some();
    let policy = RepoSemanticEmbeddingPolicy {
        present: true,
        summary_mode: Some(if summaries_enabled {
            SemanticSummaryMode::Auto
        } else {
            SemanticSummaryMode::Off
        }),
        embedding_mode: Some(if embeddings_enabled {
            SemanticCloneEmbeddingMode::SemanticAwareOnce
        } else {
            SemanticCloneEmbeddingMode::Off
        }),
        inference: SemanticClonesInferenceBindings {
            summary_generation: summary_generation.map(str::to_string),
            code_embeddings: code_embeddings.map(str::to_string),
            summary_embeddings: summary_embeddings.map(str::to_string),
        },
    };
    set_repo_semantic_embedding_policy(local_policy_path, &policy)?;
    Ok(policy)
}

fn current_code_embeddings_profile(project_root: &Path) -> Option<String> {
    current_semantic_policy_profile(project_root, |policy| {
        policy.inference.code_embeddings.as_deref()
    })
}

fn current_summary_generation_profile(project_root: &Path) -> Option<String> {
    current_semantic_policy_profile(project_root, |policy| {
        policy.inference.summary_generation.as_deref()
    })
}

fn current_summary_embeddings_profile(project_root: &Path) -> Option<String> {
    current_semantic_policy_profile(project_root, |policy| {
        policy.inference.summary_embeddings.as_deref()
    })
}

fn current_semantic_policy_profile(
    project_root: &Path,
    select: impl FnOnce(&RepoSemanticEmbeddingPolicy) -> Option<&str>,
) -> Option<String> {
    let policy = repo_semantic_embedding_policy(project_root).ok()?;
    select(&policy)
        .map(str::trim)
        .filter(|profile| !profile.is_empty())
        .map(str::to_string)
}

fn default_init_semantic_policy() -> RepoSemanticEmbeddingPolicy {
    RepoSemanticEmbeddingPolicy {
        present: true,
        summary_mode: Some(SemanticSummaryMode::Auto),
        embedding_mode: Some(SemanticCloneEmbeddingMode::SemanticAwareOnce),
        inference: SemanticClonesInferenceBindings {
            summary_generation: Some(DEFAULT_INIT_SUMMARY_GENERATION_PROFILE.to_string()),
            code_embeddings: Some(DEFAULT_INIT_CODE_EMBEDDINGS_PROFILE.to_string()),
            summary_embeddings: Some(DEFAULT_INIT_SUMMARY_EMBEDDINGS_PROFILE.to_string()),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InitSemanticRuntimeSelection {
    run_code_embeddings: bool,
    run_summaries: bool,
    run_summary_embeddings: bool,
}

fn init_semantic_runtime_selection(
    should_sync: bool,
    policy: &RepoSemanticEmbeddingPolicy,
) -> InitSemanticRuntimeSelection {
    let embeddings_enabled = policy.embedding_mode != Some(SemanticCloneEmbeddingMode::Off);
    let summaries_enabled = policy.summary_mode != Some(SemanticSummaryMode::Off);
    let run_code_embeddings = should_sync
        && embeddings_enabled
        && non_empty_profile(policy.inference.code_embeddings.as_deref());
    let run_summaries = should_sync
        && summaries_enabled
        && non_empty_profile(policy.inference.summary_generation.as_deref());
    let run_summary_embeddings = should_sync
        && run_summaries
        && embeddings_enabled
        && non_empty_profile(policy.inference.summary_embeddings.as_deref());

    InitSemanticRuntimeSelection {
        run_code_embeddings,
        run_summaries,
        run_summary_embeddings,
    }
}

fn non_empty_profile(profile: Option<&str>) -> bool {
    profile
        .map(str::trim)
        .is_some_and(|profile| !profile.is_empty())
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
