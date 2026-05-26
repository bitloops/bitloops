use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Context;
use serde_json::json;

use crate::capability_packs::architecture_graph::roles::{
    RoleAdjudicationEnqueueMetrics, default_queue_store, enqueue_adjudication_requests,
};
use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
    ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
};
use crate::capability_packs::semantic_clones::{
    runtime_config::resolve_semantic_clones_config,
    types::SEMANTIC_CLONES_CAPABILITY_ID,
    workplane::{
        architecture_embedding_jobs_for_artefacts, architecture_embedding_path_cleanup_jobs,
        load_effective_mailbox_intent_for_repo,
    },
};
use crate::host::capability_host::{
    CapabilityConfigView, CurrentStateConsumer, CurrentStateConsumerContext,
    CurrentStateConsumerFuture, CurrentStateConsumerRequest, CurrentStateConsumerResult,
};
use crate::host::devql::{RelationalStorage, esc_pg, sql_string_list_pg};

use super::classifier::{
    ArchitectureRoleClassificationInput, ArchitectureRoleClassificationScope,
    classify_architecture_roles_for_current_state, role_classification_scope_from_request,
};
use super::fact_extraction::RelationalArchitectureRoleCurrentStateSource;
use super::storage::{
    RoleClassificationStateReplacement, load_active_assignment_paths_not_in,
    load_active_detection_rules, replace_role_classification_state,
};
use super::taxonomy::{ArchitectureRoleReconcileMetrics, ArchitectureRoleReconcileOutcome};

pub struct ArchitectureGraphRoleCurrentStateConsumer;

const ROLE_CURRENT_STATE_FILE_BATCH_SIZE: usize = 1_000;

impl CurrentStateConsumer for ArchitectureGraphRoleCurrentStateConsumer {
    fn capability_id(&self) -> &str {
        ARCHITECTURE_GRAPH_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID
    }

    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a> {
        Box::pin(async move {
            let outcome = reconcile_role_current_state(request, context)
                .await
                .context("classifying architecture roles for current state")?;

            let mut warnings = outcome.warnings;
            let adjudication_request_count = outcome.adjudication_requests.len();
            let role_metrics = serde_json::to_value(&outcome.metrics)
                .unwrap_or_else(|_| json!({ "serialization_error": true }));
            let mut architecture_embedding_enqueue_failed = false;
            let architecture_embedding_metrics = match enqueue_architecture_embedding_refresh(
                &request.repo_id,
                &request.repo_root,
                outcome.metrics.full_reconcile,
                &outcome.architecture_embedding_refresh_paths,
                &outcome.architecture_embedding_cleanup_paths,
                context,
            )
            .await
            {
                Ok(metrics) => metrics,
                Err(err) => {
                    warnings.push(format!(
                        "Architecture embedding refresh enqueue failed: {err:#}"
                    ));
                    architecture_embedding_enqueue_failed = true;
                    ArchitectureEmbeddingEnqueueMetrics::default()
                }
            };
            let role_adjudication_configured = context
                .inference
                .has_slot(ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT);
            let mut role_adjudication_enqueue_failed = false;
            let mut role_adjudication_skipped_unconfigured = 0usize;
            let adjudication_metrics = if role_adjudication_configured {
                match enqueue_adjudication_requests(
                    &outcome.adjudication_requests,
                    context.workplane.as_ref(),
                    default_queue_store().as_ref(),
                ) {
                    Ok(metrics) => metrics,
                    Err(err) => {
                        warnings.push(format!(
                            "Architecture role adjudication enqueue failed: {err:#}"
                        ));
                        role_adjudication_enqueue_failed = true;
                        RoleAdjudicationEnqueueMetrics {
                            selected: adjudication_request_count,
                            enqueued: 0,
                            deduped: 0,
                        }
                    }
                }
            } else {
                role_adjudication_skipped_unconfigured = adjudication_request_count;
                if adjudication_request_count > 0 {
                    warnings.push(format!(
                        "Architecture role adjudication skipped because inference slot `{ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT}` is not configured"
                    ));
                }
                RoleAdjudicationEnqueueMetrics {
                    selected: adjudication_request_count,
                    enqueued: 0,
                    deduped: 0,
                }
            };

            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings,
                metrics: Some(role_current_state_metrics(
                    role_metrics,
                    &adjudication_metrics,
                    role_adjudication_enqueue_failed,
                    role_adjudication_skipped_unconfigured,
                    &architecture_embedding_metrics,
                    architecture_embedding_enqueue_failed,
                )),
            })
        })
    }
}

async fn reconcile_role_current_state(
    request: &CurrentStateConsumerRequest,
    context: &CurrentStateConsumerContext,
) -> anyhow::Result<ArchitectureRoleReconcileOutcome> {
    let scope = role_classification_scope_from_request(request);
    if !role_current_state_pipeline_active(&request.repo_id, &request.repo_root, context).await? {
        return Ok(inactive_role_current_state_outcome(&scope));
    }
    let role_current_state = RelationalArchitectureRoleCurrentStateSource::new(
        &request.repo_id,
        context.relational.as_ref(),
    );
    if !scope.full_reconcile {
        let files = context
            .relational
            .load_current_canonical_files(&request.repo_id)
            .context("loading current files for architecture role classification")?;
        return classify_architecture_roles_for_current_state(
            context.storage.as_ref(),
            &role_current_state,
            ArchitectureRoleClassificationInput {
                repo_id: &request.repo_id,
                generation_seq: request.to_generation_seq_inclusive,
                scope,
                files: &files,
            },
        )
        .await;
    }

    let mut aggregate = RoleBatchAggregate::default();
    let mut live_paths = BTreeSet::new();
    let mut offset = 0usize;
    loop {
        let files = context
            .relational
            .load_current_canonical_file_batch(
                &request.repo_id,
                offset,
                ROLE_CURRENT_STATE_FILE_BATCH_SIZE,
            )
            .context("loading current role-classification file batch")?;
        if files.is_empty() {
            break;
        }
        let affected_paths = files
            .iter()
            .map(|file| file.path.clone())
            .collect::<BTreeSet<_>>();
        live_paths.extend(affected_paths.iter().cloned());
        aggregate.batch_count += 1;
        aggregate.merge(
            classify_architecture_roles_for_current_state(
                context.storage.as_ref(),
                &role_current_state,
                ArchitectureRoleClassificationInput {
                    repo_id: &request.repo_id,
                    generation_seq: request.to_generation_seq_inclusive,
                    scope: super::classifier::ArchitectureRoleClassificationScope {
                        full_reconcile: false,
                        affected_paths,
                        removed_paths: BTreeSet::new(),
                    },
                    files: &files,
                },
            )
            .await?,
        );
        offset = offset.saturating_add(files.len());
    }

    let removed_paths = load_active_assignment_paths_not_in(
        context.storage.as_ref(),
        &request.repo_id,
        &live_paths,
    )
    .await?;
    if !removed_paths.is_empty() {
        let cleanup_outcome = replace_role_classification_state(
            context.storage.as_ref(),
            RoleClassificationStateReplacement {
                repo_id: &request.repo_id,
                fact_and_signal_paths: &[],
                facts: &[],
                signals: &[],
                assignment_paths: &[],
                assignments: &[],
                assignment_history_writes: &[],
                removed_assignment_paths: &removed_paths,
                generation_seq: request.to_generation_seq_inclusive,
            },
        )
        .await?;
        aggregate.metrics.assignments_marked_stale +=
            cleanup_outcome.write_counts.assignments_marked_stale;
        aggregate.metrics.assignment_history_rows +=
            cleanup_outcome.write_counts.assignment_history_rows;
        aggregate.metrics.transaction_count +=
            cleanup_outcome.sqlite_phase_metrics.transaction_count;
        aggregate.metrics.max_sqlite_lock_wait_ms = aggregate
            .metrics
            .max_sqlite_lock_wait_ms
            .max(cleanup_outcome.sqlite_phase_metrics.max_wait_ms);
        aggregate.metrics.max_sqlite_lock_hold_ms = aggregate
            .metrics
            .max_sqlite_lock_hold_ms
            .max(cleanup_outcome.sqlite_phase_metrics.max_hold_ms);
        aggregate.metrics.max_rss_kb = aggregate.metrics.max_rss_kb.max(cleanup_outcome.max_rss_kb);
        aggregate.removed_paths.extend(removed_paths);
    }

    aggregate.metrics.phase_name =
        Some(ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID.to_string());
    aggregate.metrics.reconcile_mode = Some("full_reconcile".to_string());
    aggregate.metrics.full_reconcile = true;
    aggregate.metrics.file_batches = aggregate.batch_count;
    aggregate.metrics.affected_paths = 0;
    aggregate.metrics.removed_paths = aggregate.removed_paths.len();
    Ok(aggregate.into_outcome())
}

async fn role_current_state_pipeline_active(
    repo_id: &str,
    _repo_root: &Path,
    context: &CurrentStateConsumerContext,
) -> anyhow::Result<bool> {
    let active_rules = load_active_detection_rules(context.storage.as_ref(), repo_id)
        .await
        .context("loading active architecture role detection rules for current-state activation")?;
    if !active_rules.is_empty() {
        return Ok(true);
    }
    if context
        .inference
        .has_slot(ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT)
    {
        return Ok(true);
    }

    Ok(false)
}

fn inactive_role_current_state_outcome(
    scope: &ArchitectureRoleClassificationScope,
) -> ArchitectureRoleReconcileOutcome {
    ArchitectureRoleReconcileOutcome {
        metrics: ArchitectureRoleReconcileMetrics {
            phase_name: Some(ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID.to_string()),
            reconcile_mode: Some(if scope.full_reconcile {
                "full_reconcile".to_string()
            } else {
                "merged_delta".to_string()
            }),
            skipped_inactive: true,
            full_reconcile: scope.full_reconcile,
            affected_paths: scope.affected_paths.len(),
            ..ArchitectureRoleReconcileMetrics::default()
        },
        warnings: Vec::new(),
        architecture_embedding_refresh_paths: Vec::new(),
        architecture_embedding_cleanup_paths: Vec::new(),
        adjudication_requests: Vec::new(),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ArchitectureEmbeddingEnqueueMetrics {
    selected: u64,
    enqueued: u64,
    deduped: u64,
}

async fn enqueue_architecture_embedding_refresh(
    repo_id: &str,
    repo_root: &Path,
    full_reconcile: bool,
    refresh_paths: &[String],
    cleanup_paths: &[String],
    context: &CurrentStateConsumerContext,
) -> anyhow::Result<ArchitectureEmbeddingEnqueueMetrics> {
    let semantic_clones_config = resolve_semantic_clones_config(&CapabilityConfigView::new(
        SEMANTIC_CLONES_CAPABILITY_ID,
        context.config_root.clone(),
    ));
    let intent = load_effective_mailbox_intent_for_repo(repo_root, &semantic_clones_config)
        .context("loading semantic-clones mailbox intent for architecture embedding refresh")?;
    if !intent.architecture_embeddings_active {
        return Ok(ArchitectureEmbeddingEnqueueMetrics::default());
    }

    let artefact_ids = if full_reconcile {
        load_current_embedding_artefact_ids(context.storage.as_ref(), repo_id, None).await?
    } else {
        load_current_embedding_artefact_ids(context.storage.as_ref(), repo_id, Some(refresh_paths))
            .await?
    };
    let mut jobs = architecture_embedding_jobs_for_artefacts(&artefact_ids)?;
    jobs.extend(architecture_embedding_path_cleanup_jobs(cleanup_paths)?);
    let selected = jobs.len() as u64;
    if jobs.is_empty() {
        return Ok(ArchitectureEmbeddingEnqueueMetrics::default());
    }
    let result = context.workplane.enqueue_jobs(jobs)?;
    Ok(ArchitectureEmbeddingEnqueueMetrics {
        selected,
        enqueued: result.inserted_jobs,
        deduped: result.updated_jobs,
    })
}

async fn load_current_embedding_artefact_ids(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: Option<&[String]>,
) -> anyhow::Result<Vec<String>> {
    let path_filter = paths
        .filter(|paths| !paths.is_empty())
        .map(|paths| format!("AND current.path IN ({})", sql_string_list_pg(paths)))
        .unwrap_or_default();
    let rows = relational
        .query_rows(&format!(
            "SELECT current.artefact_id \
             FROM artefacts_current current \
             JOIN current_file_state state ON state.repo_id = current.repo_id AND state.path = current.path \
             WHERE current.repo_id = '{}' \
               {path_filter} \
               AND state.analysis_mode = 'code' \
               AND LOWER(COALESCE(current.canonical_kind, COALESCE(current.language_kind, 'symbol'))) <> 'import' \
             ORDER BY current.path, current.start_line, current.symbol_id, COALESCE(current.start_byte, 0), current.artefact_id",
            esc_pg(repo_id),
        ))
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            row.get("artefact_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

fn role_current_state_metrics(
    role_metrics: serde_json::Value,
    adjudication_metrics: &RoleAdjudicationEnqueueMetrics,
    role_adjudication_enqueue_failed: bool,
    role_adjudication_skipped_unconfigured: usize,
    architecture_embedding_metrics: &ArchitectureEmbeddingEnqueueMetrics,
    architecture_embedding_enqueue_failed: bool,
) -> serde_json::Value {
    let mut metrics = json!({
        "roles": role_metrics,
        "architecture_embedding_selected": architecture_embedding_metrics.selected,
        "architecture_embedding_enqueued": architecture_embedding_metrics.enqueued,
        "architecture_embedding_deduped": architecture_embedding_metrics.deduped,
        "role_adjudication_selected": adjudication_metrics.selected,
        "role_adjudication_enqueued": adjudication_metrics.enqueued,
        "role_adjudication_deduped": adjudication_metrics.deduped,
    });
    if role_adjudication_skipped_unconfigured > 0 {
        metrics["role_adjudication_skipped_unconfigured"] = serde_json::Value::Number(
            serde_json::Number::from(role_adjudication_skipped_unconfigured as u64),
        );
    }
    if architecture_embedding_enqueue_failed {
        metrics["architecture_embedding_enqueue_failed"] = serde_json::Value::Bool(true);
    }
    if role_adjudication_enqueue_failed {
        metrics["role_adjudication_enqueue_failed"] = serde_json::Value::Bool(true);
    }
    metrics
}

#[derive(Debug, Default)]
struct RoleBatchAggregate {
    metrics: ArchitectureRoleReconcileMetrics,
    warnings: Vec<String>,
    architecture_embedding_refresh_paths: BTreeSet<String>,
    architecture_embedding_cleanup_paths: BTreeSet<String>,
    removed_paths: BTreeSet<String>,
    adjudication_scope_keys: BTreeSet<String>,
    adjudication_requests:
        Vec<crate::capability_packs::architecture_graph::roles::RoleAdjudicationRequest>,
    batch_count: usize,
}

impl RoleBatchAggregate {
    fn merge(&mut self, outcome: ArchitectureRoleReconcileOutcome) {
        let ArchitectureRoleReconcileOutcome {
            metrics,
            warnings,
            architecture_embedding_refresh_paths,
            architecture_embedding_cleanup_paths,
            adjudication_requests,
        } = outcome;
        self.metrics.rules_loaded = self.metrics.rules_loaded.max(metrics.rules_loaded);
        self.metrics.skipped_inactive |= metrics.skipped_inactive;
        self.metrics.skipped_unchanged_paths += metrics.skipped_unchanged_paths;
        self.metrics.refreshed_paths += metrics.refreshed_paths;
        self.metrics.facts_written += metrics.facts_written;
        self.metrics.facts_deleted += metrics.facts_deleted;
        self.metrics.signals_written += metrics.signals_written;
        self.metrics.signals_deleted += metrics.signals_deleted;
        self.metrics.assignments_written += metrics.assignments_written;
        self.metrics.assignments_marked_stale += metrics.assignments_marked_stale;
        self.metrics.assignment_history_rows += metrics.assignment_history_rows;
        self.metrics.adjudication_candidates += metrics.adjudication_candidates;
        self.metrics.unknown_targets_total += metrics.unknown_targets_total;
        self.metrics.unknown_targets_suppressed_non_role +=
            metrics.unknown_targets_suppressed_non_role;
        self.metrics.unknown_targets_rule_mining_eligible +=
            metrics.unknown_targets_rule_mining_eligible;
        self.metrics.unknown_targets_adjudication_escalated +=
            metrics.unknown_targets_adjudication_escalated;
        self.metrics.role_mining_clusters += metrics.role_mining_clusters;
        self.metrics.role_mining_representative_targets +=
            metrics.role_mining_representative_targets;
        self.metrics.unknown_adjudication_candidates += metrics.unknown_adjudication_candidates;
        self.metrics.high_impact_adjudication_candidates +=
            metrics.high_impact_adjudication_candidates;
        self.metrics.low_confidence_adjudication_candidates +=
            metrics.low_confidence_adjudication_candidates;
        self.metrics.conflict_adjudication_candidates += metrics.conflict_adjudication_candidates;
        self.metrics.repeated_adjudication_suppressed += metrics.repeated_adjudication_suppressed;
        self.metrics.deterministic_guard_skipped += metrics.deterministic_guard_skipped;
        self.metrics.transaction_count += metrics.transaction_count;
        self.metrics.max_rss_kb = self.metrics.max_rss_kb.max(metrics.max_rss_kb);
        self.metrics.max_sqlite_lock_wait_ms = self
            .metrics
            .max_sqlite_lock_wait_ms
            .max(metrics.max_sqlite_lock_wait_ms);
        self.metrics.max_sqlite_lock_hold_ms = self
            .metrics
            .max_sqlite_lock_hold_ms
            .max(metrics.max_sqlite_lock_hold_ms);
        self.warnings.extend(warnings);
        self.architecture_embedding_refresh_paths
            .extend(architecture_embedding_refresh_paths);
        self.architecture_embedding_cleanup_paths
            .extend(architecture_embedding_cleanup_paths.iter().cloned());
        for path in &architecture_embedding_cleanup_paths {
            self.removed_paths.insert(path.clone());
        }
        for request in adjudication_requests {
            if self.adjudication_scope_keys.insert(request.scope_key()) {
                self.adjudication_requests.push(request);
            }
        }
    }

    fn into_outcome(self) -> ArchitectureRoleReconcileOutcome {
        ArchitectureRoleReconcileOutcome {
            metrics: self.metrics,
            warnings: self.warnings,
            architecture_embedding_refresh_paths: self
                .architecture_embedding_refresh_paths
                .into_iter()
                .collect(),
            architecture_embedding_cleanup_paths: self
                .architecture_embedding_cleanup_paths
                .into_iter()
                .collect(),
            adjudication_requests: self.adjudication_requests,
        }
    }
}
