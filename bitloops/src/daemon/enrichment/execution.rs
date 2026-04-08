use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use crate::capability_packs::semantic_clones::embeddings::EmbeddingProviderConfig;
use crate::capability_packs::semantic_clones::extension_descriptor::{
    build_semantic_summary_provider, build_symbol_embedding_provider,
};
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::capability_packs::semantic_clones::features::SemanticSummaryProviderConfig;
use crate::capability_packs::semantic_clones::{
    RepoEmbeddingSyncAction, clear_repo_active_embedding_setup, clear_repo_symbol_embedding_rows,
    determine_repo_embedding_sync_action, load_semantic_feature_inputs_for_artefacts,
    load_semantic_summary_snapshot, persist_active_embedding_setup, persist_semantic_summary_row,
    refresh_current_repo_symbol_embeddings_and_clone_edges, upsert_symbol_embedding_rows,
};
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, resolve_embedding_capability_config_for_repo,
    resolve_store_backend_config_for_repo, resolve_store_semantic_config_for_repo,
};
use crate::host::devql::{DevqlConfig, RelationalStorage, resolve_repo_identity};

use super::{
    EnrichmentJob, EnrichmentJobKind, EnrichmentJobTarget, FollowUpJob, JobExecutionOutcome,
    fallback_repo_identity,
};

type SharedEmbeddingProvider =
    Arc<dyn crate::adapters::model_providers::embeddings::EmbeddingProvider>;
type SharedEmbeddingProviderInit =
    Arc<OnceLock<std::result::Result<SharedEmbeddingProvider, String>>>;

pub(super) async fn execute_job(job: &EnrichmentJob) -> JobExecutionOutcome {
    let repo = resolve_repo_identity(&job.repo_root)
        .unwrap_or_else(|_| fallback_repo_identity(&job.repo_root, &job.repo_id));
    let cfg = match DevqlConfig::from_roots(job.config_root.clone(), job.repo_root.clone(), repo) {
        Ok(cfg) => cfg,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    let backends = match resolve_store_backend_config_for_repo(&job.config_root) {
        Ok(backends) => backends,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    let relational =
        match RelationalStorage::connect(&cfg, &backends.relational, "daemon enrichment worker")
            .await
        {
            Ok(relational) => relational,
            Err(err) => return JobExecutionOutcome::failed(err),
        };

    match &job.job {
        EnrichmentJobKind::SemanticSummaries {
            artefact_ids,
            input_hashes,
            ..
        } => {
            let inputs = match load_enrichment_job_inputs(&relational, job, artefact_ids).await {
                Ok(inputs) => inputs,
                Err(err) => return JobExecutionOutcome::failed(err),
            };
            execute_semantic_job(&relational, job, &inputs, input_hashes).await
        }
        EnrichmentJobKind::SymbolEmbeddings {
            artefact_ids,
            input_hashes,
            representation_kind,
            ..
        } => {
            let inputs = match load_enrichment_job_inputs(&relational, job, artefact_ids).await {
                Ok(inputs) => inputs,
                Err(err) => return JobExecutionOutcome::failed(err),
            };
            execute_embedding_job(
                &relational,
                job,
                &inputs,
                input_hashes,
                *representation_kind,
            )
            .await
        }
        EnrichmentJobKind::CloneEdgesRebuild {} => {
            execute_clone_edges_rebuild_job(&cfg, &relational, job).await
        }
    }
}

fn embedding_provider_cache() -> &'static Mutex<HashMap<String, SharedEmbeddingProviderInit>> {
    static CACHE: OnceLock<Mutex<HashMap<String, SharedEmbeddingProviderInit>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn build_embedding_provider_cache_key(
    provider_config: &EmbeddingProviderConfig,
    repo_root: &std::path::Path,
) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        provider_config.daemon_config_path.display(),
        embedding_provider_config_fingerprint(provider_config),
        provider_config
            .embedding_profile
            .as_deref()
            .unwrap_or_default(),
        provider_config.runtime_command,
        provider_config.runtime_args.join("\u{1f}"),
        provider_config.startup_timeout_secs,
        provider_config.request_timeout_secs,
        provider_config.warnings.join("\u{1f}"),
        std::env::var("BITLOOPS_TEST_EMBED_PROVIDER").unwrap_or_default(),
        std::env::var("BITLOOPS_TEST_EMBED_MODEL").unwrap_or_default(),
        std::env::var("BITLOOPS_TEST_EMBED_DIMENSION").unwrap_or_default(),
        repo_root.display(),
    )
}

fn embedding_provider_config_fingerprint(provider_config: &EmbeddingProviderConfig) -> String {
    let bytes = std::fs::read(&provider_config.daemon_config_path).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn load_or_build_embedding_provider(
    provider_config: &EmbeddingProviderConfig,
    repo_root: &std::path::Path,
) -> Result<Option<SharedEmbeddingProvider>> {
    let Some(profile_name) = provider_config.embedding_profile.as_deref() else {
        return Ok(None);
    };
    if profile_name.trim().is_empty() {
        return Ok(None);
    }

    let cache_key = build_embedding_provider_cache_key(provider_config, repo_root);
    let init = {
        let mut cache = embedding_provider_cache()
            .lock()
            .expect("lock embedding provider cache");
        Arc::clone(
            cache
                .entry(cache_key.clone())
                .or_insert_with(|| Arc::new(OnceLock::new())),
        )
    };

    let profile_name = profile_name.to_string();
    match init.get_or_init(|| {
        match build_symbol_embedding_provider(provider_config, Some(repo_root)) {
            Ok(Some(provider)) => Ok(provider),
            Ok(None) => Err(format!(
                "embedding profile `{profile_name}` is not available"
            )),
            Err(err) => Err(format!("{err:#}")),
        }
    }) {
        Ok(provider) => Ok(Some(Arc::clone(provider))),
        Err(message) => {
            evict_cached_embedding_provider(provider_config, repo_root);
            Err(anyhow::anyhow!(message.clone()))
        }
    }
}

fn evict_cached_embedding_provider(
    provider_config: &EmbeddingProviderConfig,
    repo_root: &std::path::Path,
) {
    let cache_key = build_embedding_provider_cache_key(provider_config, repo_root);
    if let Ok(mut cache) = embedding_provider_cache().lock() {
        cache.remove(&cache_key);
    }
}

async fn execute_semantic_job(
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    inputs: &[semantic_features::SemanticFeatureInput],
    input_hashes: &BTreeMap<String, String>,
) -> JobExecutionOutcome {
    let artefact_ids = inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let semantic_cfg = resolve_store_semantic_config_for_repo(&job.config_root);
    let summary_provider = match build_semantic_summary_provider(&SemanticSummaryProviderConfig {
        semantic_provider: semantic_cfg.semantic_provider,
        semantic_model: semantic_cfg.semantic_model,
        semantic_api_key: semantic_cfg.semantic_api_key,
        semantic_base_url: semantic_cfg.semantic_base_url,
    }) {
        Ok(provider) => provider,
        Err(err) => return JobExecutionOutcome::failed(err),
    };

    let mut produced_enriched_semantics = false;
    for input in inputs {
        let Some(expected_hash) = input_hashes.get(&input.artefact_id) else {
            continue;
        };
        let current = match load_semantic_summary_snapshot(relational, &input.artefact_id).await {
            Ok(snapshot) => snapshot,
            Err(err) => return JobExecutionOutcome::failed(err),
        };
        let Some(current) = current else {
            continue;
        };
        if current.semantic_features_input_hash != *expected_hash {
            continue;
        }
        if current.is_llm_enriched() {
            continue;
        }

        let input = input.clone();
        let summary_provider = Arc::clone(&summary_provider);
        let rows = match tokio::task::spawn_blocking(move || {
            semantic_features::build_semantic_feature_rows(&input, summary_provider.as_ref())
        })
        .await
        .context("building queued semantic summary rows on blocking worker")
        {
            Ok(rows) => rows,
            Err(err) => return JobExecutionOutcome::failed(err),
        };

        produced_enriched_semantics |= rows
            .semantics
            .llm_summary
            .as_deref()
            .is_some_and(|summary| !summary.trim().is_empty())
            || rows
                .semantics
                .source_model
                .as_deref()
                .is_some_and(|source_model| !source_model.trim().is_empty());
        if let Err(err) =
            persist_semantic_summary_row(relational, &rows.semantics, expected_hash).await
        {
            return JobExecutionOutcome::failed(err);
        }
    }

    let mut outcome = JobExecutionOutcome::ok();
    if produced_enriched_semantics && embeddings_enabled(&job.config_root) {
        outcome.follow_ups.push(symbol_embeddings_follow_up(
            job,
            &artefact_ids,
            input_hashes,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Enriched,
        ));
    }
    outcome
}

async fn execute_embedding_job(
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    inputs: &[semantic_features::SemanticFeatureInput],
    input_hashes: &BTreeMap<String, String>,
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> JobExecutionOutcome {
    if !embeddings_enabled(&job.config_root) {
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome::ok(),
            Err(err) => JobExecutionOutcome::failed(err),
        };
    }

    let capability = resolve_embedding_capability_config_for_repo(&job.config_root);
    let provider_config = EmbeddingProviderConfig {
        daemon_config_path: job.config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH),
        embedding_profile: capability.semantic_clones.embedding_profile,
        runtime_command: capability.embeddings.runtime.command,
        runtime_args: capability.embeddings.runtime.args,
        startup_timeout_secs: capability.embeddings.runtime.startup_timeout_secs,
        request_timeout_secs: capability.embeddings.runtime.request_timeout_secs,
        warnings: capability.embeddings.warnings,
    };

    let provider = match load_or_build_embedding_provider(&provider_config, &job.repo_root) {
        Ok(provider) => provider,
        Err(err) => {
            let error = format!("{err:#}");
            evict_cached_embedding_provider(&provider_config, &job.repo_root);
            return match clear_embedding_outputs(relational, &job.repo_id).await {
                Ok(()) => JobExecutionOutcome {
                    error: Some(error),
                    follow_ups: Vec::new(),
                },
                Err(clear_err) => JobExecutionOutcome::failed(clear_err),
            };
        }
    };
    let Some(provider) = provider else {
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome::ok(),
            Err(err) => JobExecutionOutcome::failed(err),
        };
    };

    let setup = match crate::capability_packs::semantic_clones::embeddings::resolve_embedding_setup(
        provider.as_ref(),
    ) {
        Ok(setup) => setup,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    let sync_action = match determine_repo_embedding_sync_action(
        relational,
        &job.repo_id,
        representation_kind,
        &setup,
    )
    .await
    {
        Ok(action) => action,
        Err(err) => return JobExecutionOutcome::failed(err),
    };

    if sync_action == RepoEmbeddingSyncAction::RefreshCurrentRepo {
        if let Err(err) = clear_embedding_outputs(relational, &job.repo_id).await {
            return JobExecutionOutcome::failed(err);
        }
        return match refresh_current_repo_symbol_embeddings_and_clone_edges(
            relational,
            &job.repo_root,
            &job.repo_id,
            representation_kind,
            Arc::clone(&provider),
        )
        .await
        {
            Ok(_) => JobExecutionOutcome::ok(),
            Err(err) => {
                evict_cached_embedding_provider(&provider_config, &job.repo_root);
                JobExecutionOutcome::failed(err)
            }
        };
    }

    if sync_action == RepoEmbeddingSyncAction::AdoptExisting
        && let Err(err) = persist_active_embedding_setup(
            relational,
            &job.repo_id,
            &crate::capability_packs::semantic_clones::embeddings::ActiveEmbeddingRepresentationState::new(
                representation_kind,
                setup.clone(),
            ),
        )
        .await
    {
        return JobExecutionOutcome::failed(err);
    }

    let current_inputs = match filter_current_inputs(relational, inputs, input_hashes).await {
        Ok(filtered) => filtered,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    if current_inputs.is_empty() {
        return JobExecutionOutcome::ok();
    }

    let embedding_stats = match upsert_symbol_embedding_rows(
        relational,
        &current_inputs,
        representation_kind,
        Arc::clone(&provider),
    )
    .await
    {
        Ok(stats) => stats,
        Err(err) => {
            evict_cached_embedding_provider(&provider_config, &job.repo_root);
            let error = format!("{err:#}");
            return match clear_embedding_outputs(relational, &job.repo_id).await {
                Ok(()) => JobExecutionOutcome {
                    error: Some(error),
                    follow_ups: Vec::new(),
                },
                Err(clear_err) => JobExecutionOutcome::failed(clear_err),
            };
        }
    };
    if embedding_stats.eligible == 0 {
        return JobExecutionOutcome::ok();
    }

    if let Err(err) = persist_active_embedding_setup(
        relational,
        &job.repo_id,
        &crate::capability_packs::semantic_clones::embeddings::ActiveEmbeddingRepresentationState::new(
            representation_kind,
            setup,
        ),
    )
    .await
    {
        return JobExecutionOutcome::failed(err);
    }

    let mut outcome = JobExecutionOutcome::ok();
    outcome.follow_ups.push(clone_edges_rebuild_follow_up(job));
    outcome
}

async fn execute_clone_edges_rebuild_job(
    _cfg: &DevqlConfig,
    relational: &RelationalStorage,
    job: &EnrichmentJob,
) -> JobExecutionOutcome {
    if !embeddings_enabled(&job.config_root) {
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome::ok(),
            Err(err) => JobExecutionOutcome::failed(err),
        };
    }

    let capability = resolve_embedding_capability_config_for_repo(&job.config_root);
    if capability.semantic_clones.embedding_profile.is_none() {
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome::ok(),
            Err(err) => JobExecutionOutcome::failed(err),
        };
    }

    match crate::capability_packs::semantic_clones::pipeline::rebuild_symbol_clone_edges(
        relational,
        &job.repo_id,
    )
    .await
    {
        Ok(_) => JobExecutionOutcome::ok(),
        Err(err) => JobExecutionOutcome::failed(err),
    }
}

fn symbol_embeddings_follow_up(
    job: &EnrichmentJob,
    artefact_ids: &[String],
    input_hashes: &BTreeMap<String, String>,
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> FollowUpJob {
    FollowUpJob::SymbolEmbeddings {
        target: EnrichmentJobTarget::from_job(job),
        artefact_ids: artefact_ids.to_vec(),
        input_hashes: input_hashes.clone(),
        representation_kind,
    }
}

fn clone_edges_rebuild_follow_up(job: &EnrichmentJob) -> FollowUpJob {
    FollowUpJob::CloneEdgesRebuild {
        target: EnrichmentJobTarget::from_job(job),
    }
}

async fn clear_embedding_outputs(relational: &RelationalStorage, repo_id: &str) -> Result<()> {
    clear_repo_symbol_embedding_rows(relational, repo_id).await?;
    clear_repo_active_embedding_setup(relational, repo_id).await?;
    crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
        relational, repo_id,
    )
    .await
}

fn embeddings_enabled(config_root: &std::path::Path) -> bool {
    let capability = resolve_embedding_capability_config_for_repo(config_root);
    capability.semantic_clones.embedding_mode != crate::config::SemanticCloneEmbeddingMode::Off
        && capability.semantic_clones.embedding_profile.is_some()
}

async fn filter_current_inputs(
    relational: &RelationalStorage,
    inputs: &[semantic_features::SemanticFeatureInput],
    input_hashes: &BTreeMap<String, String>,
) -> Result<Vec<semantic_features::SemanticFeatureInput>> {
    let mut filtered = Vec::with_capacity(inputs.len());
    for input in inputs {
        let Some(expected_hash) = input_hashes.get(&input.artefact_id) else {
            continue;
        };
        let Some(snapshot) = load_semantic_summary_snapshot(relational, &input.artefact_id).await?
        else {
            continue;
        };
        if snapshot.semantic_features_input_hash == *expected_hash {
            filtered.push(input.clone());
        }
    }
    Ok(filtered)
}

async fn load_enrichment_job_inputs(
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    artefact_ids: &[String],
) -> Result<Vec<semantic_features::SemanticFeatureInput>> {
    load_semantic_feature_inputs_for_artefacts(relational, &job.repo_root, artefact_ids)
        .await
        .with_context(|| {
            format!(
                "rehydrating enrichment inputs for job `{}` in repo `{}`",
                job.id, job.repo_id
            )
        })
}

#[cfg(test)]
mod execution_tests;
