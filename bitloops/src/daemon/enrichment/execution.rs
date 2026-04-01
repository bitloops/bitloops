use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingProviderConfig;
use crate::capability_packs::semantic_clones::extension_descriptor::{
    build_semantic_summary_provider, build_symbol_embedding_provider,
};
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::capability_packs::semantic_clones::features::SemanticSummaryProviderConfig;
use crate::capability_packs::semantic_clones::{
    clear_repo_symbol_embedding_rows, load_semantic_feature_inputs_for_artefacts,
    load_semantic_summary_snapshot, persist_semantic_summary_row, upsert_symbol_embedding_rows,
};
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, SemanticCloneEmbeddingMode,
    resolve_embedding_capability_config_for_repo, resolve_store_backend_config_for_repo,
    resolve_store_semantic_config_for_repo,
};
use crate::host::devql::{DevqlConfig, RelationalStorage, resolve_repo_identity};

use super::{
    EnrichmentJob, EnrichmentJobKind, EnrichmentJobTarget, FollowUpJob, JobExecutionOutcome,
    fallback_repo_identity,
};

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
            embedding_mode,
            ..
        } => {
            let inputs = match load_enrichment_job_inputs(&relational, job, artefact_ids).await {
                Ok(inputs) => inputs,
                Err(err) => return JobExecutionOutcome::failed(err),
            };
            execute_semantic_job(&relational, job, &inputs, input_hashes, *embedding_mode).await
        }
        EnrichmentJobKind::SymbolEmbeddings {
            artefact_ids,
            input_hashes,
            embedding_mode,
            ..
        } => {
            let inputs = match load_enrichment_job_inputs(&relational, job, artefact_ids).await {
                Ok(inputs) => inputs,
                Err(err) => return JobExecutionOutcome::failed(err),
            };
            execute_embedding_job(&relational, job, &inputs, input_hashes, *embedding_mode).await
        }
        EnrichmentJobKind::CloneEdgesRebuild { embedding_mode } => {
            execute_clone_edges_rebuild_job(&cfg, &relational, job, *embedding_mode).await
        }
    }
}

async fn execute_semantic_job(
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    inputs: &[semantic_features::SemanticFeatureInput],
    input_hashes: &BTreeMap<String, String>,
    embedding_mode: SemanticCloneEmbeddingMode,
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
        Err(err) => {
            let mut outcome = JobExecutionOutcome::failed(err);
            if embedding_mode == SemanticCloneEmbeddingMode::SemanticAwareOnce {
                outcome.follow_ups.push(symbol_embeddings_follow_up(
                    job,
                    &artefact_ids,
                    input_hashes,
                    embedding_mode,
                ));
            }
            return outcome;
        }
    };

    let mut summary_changed = false;
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
            Err(err) => {
                let mut outcome = JobExecutionOutcome::failed(err);
                if embedding_mode == SemanticCloneEmbeddingMode::SemanticAwareOnce {
                    outcome.follow_ups.push(symbol_embeddings_follow_up(
                        job,
                        &artefact_ids,
                        input_hashes,
                        embedding_mode,
                    ));
                }
                return outcome;
            }
        };

        if current.summary != rows.semantics.summary {
            summary_changed = true;
        }
        if let Err(err) =
            persist_semantic_summary_row(relational, &rows.semantics, expected_hash).await
        {
            let mut outcome = JobExecutionOutcome::failed(err);
            if embedding_mode == SemanticCloneEmbeddingMode::SemanticAwareOnce {
                outcome.follow_ups.push(symbol_embeddings_follow_up(
                    job,
                    &artefact_ids,
                    input_hashes,
                    embedding_mode,
                ));
            }
            return outcome;
        }
    }

    let mut outcome = JobExecutionOutcome::ok();
    match embedding_mode {
        SemanticCloneEmbeddingMode::SemanticAwareOnce => {
            outcome.follow_ups.push(symbol_embeddings_follow_up(
                job,
                &artefact_ids,
                input_hashes,
                embedding_mode,
            ));
        }
        SemanticCloneEmbeddingMode::RefreshOnUpgrade if summary_changed => {
            outcome.follow_ups.push(symbol_embeddings_follow_up(
                job,
                &artefact_ids,
                input_hashes,
                embedding_mode,
            ));
        }
        _ => {}
    }
    outcome
}

async fn execute_embedding_job(
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    inputs: &[semantic_features::SemanticFeatureInput],
    input_hashes: &BTreeMap<String, String>,
    embedding_mode: SemanticCloneEmbeddingMode,
) -> JobExecutionOutcome {
    if embedding_mode == SemanticCloneEmbeddingMode::Off {
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome::ok(),
            Err(err) => JobExecutionOutcome::failed(err),
        };
    }

    let current_inputs = match filter_current_inputs(relational, inputs, input_hashes).await {
        Ok(filtered) => filtered,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    if current_inputs.is_empty() {
        return JobExecutionOutcome::ok();
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

    let provider = match build_symbol_embedding_provider(&provider_config, Some(&job.repo_root)) {
        Ok(provider) => provider,
        Err(err) => {
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
    let Some(provider) = provider else {
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome::ok(),
            Err(err) => JobExecutionOutcome::failed(err),
        };
    };

    if let Err(err) = upsert_symbol_embedding_rows(relational, &current_inputs, provider).await {
        let error = format!("{err:#}");
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome {
                error: Some(error),
                follow_ups: Vec::new(),
            },
            Err(clear_err) => JobExecutionOutcome::failed(clear_err),
        };
    }

    let mut outcome = JobExecutionOutcome::ok();
    outcome
        .follow_ups
        .push(clone_edges_rebuild_follow_up(job, embedding_mode));
    outcome
}

async fn execute_clone_edges_rebuild_job(
    _cfg: &DevqlConfig,
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    embedding_mode: SemanticCloneEmbeddingMode,
) -> JobExecutionOutcome {
    if embedding_mode == SemanticCloneEmbeddingMode::Off {
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
    embedding_mode: SemanticCloneEmbeddingMode,
) -> FollowUpJob {
    FollowUpJob::SymbolEmbeddings {
        target: EnrichmentJobTarget::from_job(job),
        artefact_ids: artefact_ids.to_vec(),
        input_hashes: input_hashes.clone(),
        embedding_mode,
    }
}

fn clone_edges_rebuild_follow_up(
    job: &EnrichmentJob,
    embedding_mode: SemanticCloneEmbeddingMode,
) -> FollowUpJob {
    FollowUpJob::CloneEdgesRebuild {
        target: EnrichmentJobTarget::from_job(job),
        embedding_mode,
    }
}

async fn clear_embedding_outputs(relational: &RelationalStorage, repo_id: &str) -> Result<()> {
    clear_repo_symbol_embedding_rows(relational, repo_id).await?;
    crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
        relational, repo_id,
    )
    .await
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
