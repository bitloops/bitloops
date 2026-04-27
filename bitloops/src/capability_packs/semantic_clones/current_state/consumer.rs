use std::collections::BTreeSet;
use std::time::Instant;

use serde_json::json;

use crate::host::capability_host::{
    CapabilityConfigView, CurrentStateConsumer, CurrentStateConsumerContext,
    CurrentStateConsumerFuture, CurrentStateConsumerRequest, CurrentStateConsumerResult,
    ReconcileMode,
};

use super::super::runtime_config::resolve_semantic_clones_config;
use super::super::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID,
    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use super::super::workplane::{repo_backfill_dedupe_key, resolve_effective_mailbox_intent};
use super::jobs::{
    artefact_job, embedding_jobs_for_artefacts, repo_backfill_job, repo_backfill_jobs,
};
use super::projection::{
    clear_current_projection_rows, collect_affected_paths, current_repo_backfill_artefact_ids,
};

pub struct SemanticClonesCurrentStateConsumer;

impl CurrentStateConsumer for SemanticClonesCurrentStateConsumer {
    fn capability_id(&self) -> &str {
        SEMANTIC_CLONES_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID
    }

    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a> {
        Box::pin(async move {
            let total_started = Instant::now();
            let config = resolve_semantic_clones_config(&CapabilityConfigView::new(
                SEMANTIC_CLONES_CAPABILITY_ID,
                context.config_root.clone(),
            ));
            let intent = resolve_effective_mailbox_intent(context.workplane.as_ref(), &config)?;
            let affected_paths = collect_affected_paths(request);
            let clear_started = Instant::now();
            let cleared_paths = clear_current_projection_rows(
                context.storage.as_ref(),
                &request.repo_id,
                &affected_paths,
            )
            .await?;
            let clear_current_projection_ms = clear_started.elapsed().as_millis() as u64;

            if !intent.has_any_pipeline_intent() {
                super::super::pipeline::delete_repo_current_symbol_clone_edges(
                    context.storage.as_ref(),
                    &request.repo_id,
                )
                .await?;
                let total_ms = total_started.elapsed().as_millis() as u64;
                return Ok(CurrentStateConsumerResult {
                    applied_to_generation_seq: request.to_generation_seq_inclusive,
                    warnings: Vec::new(),
                    metrics: Some(json!({
                        "affected_paths": affected_paths.len(),
                        "cleared_paths": cleared_paths,
                        "clear_current_projection_ms": clear_current_projection_ms,
                        "load_backfill_ids_ms": 0_u64,
                        "build_jobs_ms": 0_u64,
                        "enqueue_jobs_ms": 0_u64,
                        "total_ms": total_ms,
                        "enqueued_summary_jobs": 0,
                        "enqueued_code_embedding_jobs": 0,
                        "enqueued_identity_embedding_jobs": 0,
                        "enqueued_summary_embedding_jobs": 0,
                        "enqueued_clone_rebuild": 0,
                        "reconcile_mode": reconcile_mode_label(request.reconcile_mode),
                    })),
                });
            }

            let artefact_ids = request
                .artefact_upserts
                .iter()
                .map(|artefact| artefact.artefact_id.clone())
                .collect::<BTreeSet<_>>();
            let changed_artefact_ids = artefact_ids.iter().cloned().collect::<Vec<_>>();
            let has_removals =
                !request.file_removals.is_empty() || !request.artefact_removals.is_empty();
            let is_full_reconcile = matches!(request.reconcile_mode, ReconcileMode::FullReconcile);
            let load_backfill_started = Instant::now();
            let full_reconcile_artefact_ids = if is_full_reconcile
                && (intent.summary_refresh_active
                    || intent.code_embeddings_active
                    || intent.summary_embeddings_active)
            {
                Some(
                    current_repo_backfill_artefact_ids(context.storage.as_ref(), &request.repo_id)
                        .await?,
                )
            } else {
                None
            };
            let load_backfill_ids_ms = load_backfill_started.elapsed().as_millis() as u64;
            let full_reconcile_artefact_ids = full_reconcile_artefact_ids.as_deref().unwrap_or(&[]);

            let build_jobs_started = Instant::now();
            let mut jobs = Vec::new();
            let mut summary_job_count = 0_u64;
            let mut code_embedding_job_count = 0_u64;
            let mut identity_embedding_job_count = 0_u64;
            let mut summary_embedding_job_count = 0_u64;
            let mut clone_rebuild_job_count = 0_u64;

            if intent.summary_refresh_active {
                if is_full_reconcile {
                    let repo_backfill_jobs = repo_backfill_jobs(
                        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                        full_reconcile_artefact_ids,
                    )?;
                    summary_job_count += repo_backfill_jobs.len() as u64;
                    jobs.extend(repo_backfill_jobs);
                } else {
                    for artefact_id in &artefact_ids {
                        jobs.push(artefact_job(
                            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                            artefact_id,
                        )?);
                        summary_job_count += 1;
                    }
                }
            }

            if intent.code_embeddings_active {
                if is_full_reconcile {
                    let repo_backfill_jobs = repo_backfill_jobs(
                        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                        full_reconcile_artefact_ids,
                    )?;
                    code_embedding_job_count += repo_backfill_jobs.len() as u64;
                    jobs.extend(repo_backfill_jobs);
                } else {
                    let embedding_jobs = embedding_jobs_for_artefacts(
                        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                        &changed_artefact_ids,
                    )?;
                    code_embedding_job_count += embedding_jobs.len() as u64;
                    jobs.extend(embedding_jobs);
                }

                if is_full_reconcile {
                    let repo_backfill_jobs = repo_backfill_jobs(
                        SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
                        full_reconcile_artefact_ids,
                    )?;
                    identity_embedding_job_count += repo_backfill_jobs.len() as u64;
                    jobs.extend(repo_backfill_jobs);
                } else {
                    let embedding_jobs = embedding_jobs_for_artefacts(
                        SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
                        &changed_artefact_ids,
                    )?;
                    identity_embedding_job_count += embedding_jobs.len() as u64;
                    jobs.extend(embedding_jobs);
                }
            }

            if intent.summary_embeddings_active && !intent.summary_refresh_active {
                if is_full_reconcile {
                    let repo_backfill_jobs = repo_backfill_jobs(
                        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                        full_reconcile_artefact_ids,
                    )?;
                    summary_embedding_job_count += repo_backfill_jobs.len() as u64;
                    jobs.extend(repo_backfill_jobs);
                } else {
                    for artefact_id in &artefact_ids {
                        jobs.push(artefact_job(
                            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                            artefact_id,
                        )?);
                        summary_embedding_job_count += 1;
                    }
                }
            }

            let embedding_pipeline_scheduled = code_embedding_job_count > 0
                || identity_embedding_job_count > 0
                || summary_embedding_job_count > 0
                || (summary_job_count > 0 && intent.summary_embeddings_active);
            if intent.clone_rebuild_active
                && (is_full_reconcile || has_removals || !embedding_pipeline_scheduled)
            {
                jobs.push(repo_backfill_job(
                    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
                    Some(1),
                    None,
                    repo_backfill_dedupe_key(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX),
                )?);
                clone_rebuild_job_count += 1;
            }
            let build_jobs_ms = build_jobs_started.elapsed().as_millis() as u64;

            let enqueue_started = Instant::now();
            if !jobs.is_empty() {
                context.workplane.enqueue_jobs(jobs)?;
            }
            let enqueue_jobs_ms = enqueue_started.elapsed().as_millis() as u64;

            if !intent.has_any_embedding_intent() {
                super::super::pipeline::delete_repo_current_symbol_clone_edges(
                    context.storage.as_ref(),
                    &request.repo_id,
                )
                .await?;
            }
            let total_ms = total_started.elapsed().as_millis() as u64;

            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings: Vec::new(),
                metrics: Some(json!({
                    "affected_paths": affected_paths.len(),
                    "cleared_paths": cleared_paths,
                    "clear_current_projection_ms": clear_current_projection_ms,
                    "load_backfill_ids_ms": load_backfill_ids_ms,
                    "build_jobs_ms": build_jobs_ms,
                    "enqueue_jobs_ms": enqueue_jobs_ms,
                    "total_ms": total_ms,
                    "enqueued_summary_jobs": summary_job_count,
                    "enqueued_code_embedding_jobs": code_embedding_job_count,
                    "enqueued_identity_embedding_jobs": identity_embedding_job_count,
                    "enqueued_summary_embedding_jobs": summary_embedding_job_count,
                    "enqueued_clone_rebuild": clone_rebuild_job_count,
                    "reconcile_mode": reconcile_mode_label(request.reconcile_mode),
                })),
            })
        })
    }
}

pub(super) fn reconcile_mode_label(mode: ReconcileMode) -> &'static str {
    match mode {
        ReconcileMode::MergedDelta => "merged_delta",
        ReconcileMode::FullReconcile => "full_reconcile",
    }
}
