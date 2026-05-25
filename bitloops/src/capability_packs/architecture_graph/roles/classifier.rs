use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};

use serde_json::json;

use crate::host::devql::RelationalStorage;
use crate::models::CurrentCanonicalFileRecord;

use super::contracts::{
    AdjudicationReason, RoleAdjudicationRequest, RoleCurrentAssignmentSnapshot,
    placeholder_request_hash, role_adjudication_stable_request_key,
};
use super::fact_extraction::{
    ArchitectureRoleCurrentStateSource, ArchitectureRoleFactExtractionInput,
    extract_architecture_role_facts,
};
use super::rules::{compile_detection_rules, evaluate_rules_over_facts};
use super::storage::{
    AssignmentHistoryWrite, RoleClassificationStateReplacement,
    load_active_assignment_paths_not_in, load_active_detection_rules, load_assignments_for_paths,
    replace_role_classification_state,
};
use super::taxonomy::{
    ArchitectureRoleAssignment, ArchitectureRoleReconcileMetrics, ArchitectureRoleReconcileOutcome,
    ArchitectureRoleRuleSignal, AssignmentPriority, AssignmentSource, AssignmentStatus,
    RoleSignalPolarity, RoleTarget, assignment_id,
};

mod unknown_policy;
#[cfg(test)]
use unknown_policy::path_suppressed_for_roles;
use unknown_policy::{
    RoleTargetSummary, facts_by_target, select_unknown_target_policy, target_summaries_from_facts,
};

pub const ARCHITECTURE_ROLE_CLASSIFIER_VERSION: &str =
    "architecture_roles.deterministic.contract.v1";

#[derive(Debug, Clone)]
pub struct ArchitectureRoleClassificationInput<'a> {
    pub repo_id: &'a str,
    pub generation_seq: u64,
    pub scope: ArchitectureRoleClassificationScope,
    pub files: &'a [CurrentCanonicalFileRecord],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchitectureRoleClassificationScope {
    pub full_reconcile: bool,
    pub affected_paths: BTreeSet<String>,
    pub removed_paths: BTreeSet<String>,
}

pub fn role_classification_scope_from_request(
    request: &crate::host::capability_host::CurrentStateConsumerRequest,
) -> ArchitectureRoleClassificationScope {
    if request.reconcile_mode == crate::host::capability_host::ReconcileMode::FullReconcile {
        return ArchitectureRoleClassificationScope {
            full_reconcile: true,
            affected_paths: BTreeSet::new(),
            removed_paths: BTreeSet::new(),
        };
    }

    ArchitectureRoleClassificationScope {
        full_reconcile: false,
        affected_paths: affected_role_paths_from_request(request),
        removed_paths: removed_role_paths_from_request(request),
    }
}

pub fn affected_role_paths_from_request(
    request: &crate::host::capability_host::CurrentStateConsumerRequest,
) -> BTreeSet<String> {
    let mut affected = BTreeSet::new();
    affected.extend(request.affected_paths.iter().cloned());
    affected.extend(request.file_upserts.iter().map(|file| file.path.clone()));
    affected.extend(request.file_removals.iter().map(|file| file.path.clone()));
    affected.extend(
        request
            .artefact_upserts
            .iter()
            .map(|artefact| artefact.path.clone()),
    );
    affected.extend(
        request
            .artefact_removals
            .iter()
            .map(|artefact| artefact.path.clone()),
    );
    affected
}

pub fn removed_role_paths_from_request(
    request: &crate::host::capability_host::CurrentStateConsumerRequest,
) -> BTreeSet<String> {
    let mut removed = BTreeSet::new();
    removed.extend(request.file_removals.iter().map(|file| file.path.clone()));
    removed
}

#[derive(Debug, Clone)]
pub struct AssignmentAggregationConfig {
    pub active_threshold: f64,
    pub review_threshold: f64,
    pub conflict_margin: f64,
}

impl Default for AssignmentAggregationConfig {
    fn default() -> Self {
        Self {
            active_threshold: 0.80,
            review_threshold: 0.50,
            conflict_margin: 0.05,
        }
    }
}

pub fn aggregate_role_assignments(
    repo_id: &str,
    signals: &[ArchitectureRoleRuleSignal],
    config: AssignmentAggregationConfig,
) -> Vec<ArchitectureRoleAssignment> {
    let mut scores: BTreeMap<(RoleTarget, String), AggregatedSignals> = BTreeMap::new();
    for signal in signals {
        let key = (signal.target.clone(), signal.role_id.clone());
        let entry = scores.entry(key).or_default();
        match signal.polarity {
            RoleSignalPolarity::Positive => entry.positive_scores.push(signal.score),
            RoleSignalPolarity::Negative => entry.negative_scores.push(signal.score),
        }
        entry.generation_seq = entry.generation_seq.max(signal.generation_seq);
        entry.evidence.push(json!({
            "signalId": signal.signal_id,
            "ruleId": signal.rule_id,
            "ruleVersion": signal.rule_version,
            "polarity": signal.polarity.as_db(),
            "score": signal.score,
            "evidence": signal.evidence
        }));
    }

    let mut by_target: BTreeMap<RoleTarget, Vec<(String, f64, AggregatedSignals)>> =
        BTreeMap::new();
    for ((target, role_id), aggregated) in scores {
        let confidence = aggregated.confidence();
        by_target
            .entry(target)
            .or_default()
            .push((role_id, confidence, aggregated));
    }

    let mut assignments = Vec::new();
    for (target, mut candidates) in by_target {
        candidates.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });
        candidates.retain(|(_, confidence, _)| *confidence >= config.review_threshold);
        let top_confidence = candidates.first().map(|(_, confidence, _)| *confidence);
        let has_top_conflict = match top_confidence {
            Some(top_confidence) => {
                candidates
                    .iter()
                    .filter(|(_, confidence, _)| {
                        top_confidence - *confidence <= config.conflict_margin
                    })
                    .count()
                    > 1
            }
            None => false,
        };
        for (index, (role_id, confidence, aggregated)) in candidates.into_iter().enumerate() {
            let priority = if index == 0 {
                AssignmentPriority::Primary
            } else {
                AssignmentPriority::Secondary
            };
            let in_top_conflict = top_confidence
                .map(|top_confidence| top_confidence - confidence <= config.conflict_margin)
                .unwrap_or(false);
            let status = if confidence >= config.active_threshold
                && !(has_top_conflict && in_top_conflict)
            {
                AssignmentStatus::Active
            } else {
                AssignmentStatus::NeedsReview
            };
            let generation_seq = aggregated.generation_seq;
            assignments.push(ArchitectureRoleAssignment {
                repo_id: repo_id.to_string(),
                assignment_id: assignment_id(repo_id, &role_id, &target),
                role_id,
                target: target.clone(),
                priority,
                status,
                source: AssignmentSource::Rule,
                confidence,
                evidence: aggregated.evidence_json(),
                provenance: json!({
                    "classifierVersion": ARCHITECTURE_ROLE_CLASSIFIER_VERSION,
                    "source": "deterministic_rules"
                }),
                classifier_version: ARCHITECTURE_ROLE_CLASSIFIER_VERSION.to_string(),
                rule_version: None,
                generation_seq,
            });
        }
    }
    assignments
}

fn assignment_meaningfully_changed(
    previous: &ArchitectureRoleAssignment,
    next: &ArchitectureRoleAssignment,
) -> bool {
    previous.role_id != next.role_id
        || previous.priority != next.priority
        || previous.status != next.status
        || (previous.confidence - next.confidence).abs() >= 0.05
        || previous.source != next.source
}

fn expanded_reconcile_metrics(
    target_summaries: &BTreeMap<RoleTarget, RoleTargetSummary>,
    assignments: &[ArchitectureRoleAssignment],
    adjudication_requests: &[RoleAdjudicationRequest],
    repeated_adjudication_suppressed: usize,
    deterministic_guard_skipped: usize,
) -> ArchitectureRoleReconcileMetrics {
    let target_count = target_summaries.len();
    let active_targets = assignments
        .iter()
        .filter(|assignment| assignment.status == AssignmentStatus::Active)
        .map(|assignment| assignment.target.clone())
        .collect::<BTreeSet<_>>();
    let review_targets = assignments
        .iter()
        .filter(|assignment| assignment.status == AssignmentStatus::NeedsReview)
        .map(|assignment| assignment.target.clone())
        .collect::<BTreeSet<_>>();
    let conflict_targets = assignments
        .iter()
        .filter(|assignment| assignment.status == AssignmentStatus::NeedsReview)
        .filter(|assignment| {
            adjudication_requests.iter().any(|request| {
                request.reason == AdjudicationReason::Conflict
                    && request.path.as_deref() == Some(assignment.target.path.as_str())
            })
        })
        .map(|assignment| assignment.target.clone())
        .collect::<BTreeSet<_>>();
    let assigned_paths = assignments
        .iter()
        .filter(|assignment| assignment.status != AssignmentStatus::Stale)
        .map(|assignment| assignment.target.path.clone())
        .collect::<BTreeSet<_>>();
    let deterministic_unassigned_targets = target_summaries
        .values()
        .filter(|summary| !assigned_paths.contains(&summary.target.path))
        .count();

    let mut metrics = ArchitectureRoleReconcileMetrics {
        target_count,
        deterministic_active_targets: active_targets.len(),
        deterministic_needs_review_targets: review_targets.len(),
        deterministic_conflict_targets: conflict_targets.len(),
        deterministic_unassigned_targets,
        repeated_adjudication_suppressed,
        deterministic_guard_skipped,
        ..Default::default()
    };
    for request in adjudication_requests {
        match request.reason {
            AdjudicationReason::Unknown => metrics.unknown_adjudication_candidates += 1,
            AdjudicationReason::HighImpact => {
                metrics.high_impact_adjudication_candidates += 1;
            }
            AdjudicationReason::LowConfidence => {
                metrics.low_confidence_adjudication_candidates += 1;
            }
            AdjudicationReason::Conflict => metrics.conflict_adjudication_candidates += 1,
            AdjudicationReason::NovelPattern | AdjudicationReason::ManualReview => {}
        }
    }
    metrics.deterministic_coverage_ratio =
        ratio(metrics.deterministic_active_targets, target_count);
    metrics.needs_review_ratio = ratio(metrics.deterministic_needs_review_targets, target_count);
    metrics.unknown_ratio = ratio(metrics.deterministic_unassigned_targets, target_count);
    metrics
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn authoritative_current_assignment(assignment: &ArchitectureRoleAssignment) -> bool {
    assignment.status == AssignmentStatus::Active && assignment.source != AssignmentSource::Rule
}

pub async fn classify_architecture_roles_for_current_state(
    relational: &RelationalStorage,
    current_state: &dyn ArchitectureRoleCurrentStateSource,
    input: ArchitectureRoleClassificationInput<'_>,
) -> Result<ArchitectureRoleReconcileOutcome> {
    let extraction = extract_architecture_role_facts(
        ArchitectureRoleFactExtractionInput {
            repo_id: input.repo_id,
            generation_seq: input.generation_seq,
            affected_paths: &input.scope.affected_paths,
            files: input.files,
        },
        current_state,
    )?;
    let target_summaries = target_summaries_from_facts(&extraction.facts);
    let facts_by_target = facts_by_target(&extraction.facts);

    let live_paths = extraction.live_paths.clone();
    let refreshed_path_set = extraction
        .refreshed_paths
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let skipped_unchanged_paths = if input.scope.full_reconcile {
        0
    } else {
        live_paths
            .iter()
            .filter(|path| !refreshed_path_set.contains(*path))
            .count()
    };
    let mut removed_assignment_paths = input.scope.removed_paths.clone();
    if input.scope.full_reconcile {
        for path in
            load_active_assignment_paths_not_in(relational, input.repo_id, &live_paths).await?
        {
            removed_assignment_paths.insert(path);
        }
    }
    let fact_and_signal_paths =
        refreshed_paths_with_removals(&extraction.refreshed_paths, &removed_assignment_paths);
    let rules = load_active_detection_rules(relational, input.repo_id).await?;
    let compiled = compile_detection_rules(rules.clone())?;
    let rule_result = evaluate_rules_over_facts(&compiled, &extraction.facts)?;
    let assignments = aggregate_role_assignments(
        input.repo_id,
        &rule_result.signals,
        AssignmentAggregationConfig::default(),
    );
    let assignment_refresh_paths =
        assignment_refresh_paths(&extraction.refreshed_paths, &removed_assignment_paths);
    let previous_assignments =
        load_assignments_for_paths(relational, input.repo_id, &assignment_refresh_paths).await?;
    let previous_by_id = previous_assignments
        .iter()
        .map(|assignment| (assignment.assignment_id.clone(), assignment))
        .collect::<BTreeMap<_, _>>();
    let adjudicated_targets = previous_assignments
        .iter()
        .filter(|assignment| authoritative_current_assignment(assignment))
        .map(|assignment| assignment.target.clone())
        .collect::<BTreeSet<_>>();
    let mut current_assignments = Vec::new();
    let mut history_writes = Vec::new();
    for assignment in &assignments {
        if let Some(previous) = previous_by_id
            .get(&assignment.assignment_id)
            .filter(|previous| authoritative_current_assignment(previous))
        {
            current_assignments.push((*previous).clone());
            continue;
        }
        current_assignments.push(assignment.clone());
        match previous_by_id.get(&assignment.assignment_id) {
            Some(previous) if assignment_meaningfully_changed(previous, assignment) => {
                history_writes.push(AssignmentHistoryWrite {
                    previous: Some((*previous).clone()),
                    next: assignment.clone(),
                    change_kind: "deterministic_reclassified".to_string(),
                });
            }
            None => {
                history_writes.push(AssignmentHistoryWrite {
                    previous: None,
                    next: assignment.clone(),
                    change_kind: "deterministic_reclassified".to_string(),
                });
            }
            Some(_) => {}
        }
    }
    let mut current_by_id = current_assignments
        .iter()
        .map(|assignment| assignment.assignment_id.clone())
        .collect::<BTreeSet<_>>();
    let next_by_id = assignments
        .iter()
        .map(|assignment| assignment.assignment_id.as_str())
        .collect::<BTreeSet<_>>();
    for previous in previous_assignments
        .iter()
        .filter(|previous| previous.status == AssignmentStatus::Active)
        .filter(|previous| !next_by_id.contains(previous.assignment_id.as_str()))
    {
        if authoritative_current_assignment(previous) {
            if current_by_id.insert(previous.assignment_id.clone()) {
                current_assignments.push(previous.clone());
            }
            continue;
        }
        let mut stale = previous.clone();
        stale.status = AssignmentStatus::Stale;
        stale.generation_seq = input.generation_seq;
        history_writes.push(AssignmentHistoryWrite {
            previous: Some(previous.clone()),
            next: stale.clone(),
            change_kind: "deterministic_reclassified".to_string(),
        });
        current_assignments.push(stale);
    }
    for previous in previous_assignments
        .iter()
        .filter(|previous| previous.status != AssignmentStatus::Active)
        .filter(|previous| !next_by_id.contains(previous.assignment_id.as_str()))
    {
        if current_by_id.insert(previous.assignment_id.clone()) {
            current_assignments.push(previous.clone());
        }
    }
    let adjudication_candidates = assignments
        .iter()
        .filter(|assignment| assignment.status == AssignmentStatus::NeedsReview)
        .filter(|assignment| !adjudicated_targets.contains(&assignment.target))
        .cloned()
        .collect::<Vec<_>>();
    let mut adjudication_requests =
        adjudication_requests_from_assignments(&adjudication_candidates);
    let mut seen_adjudication_scopes = adjudication_requests
        .iter()
        .map(RoleAdjudicationRequest::scope_key)
        .collect::<BTreeSet<_>>();
    let mut unknown_or_high_impact_candidates = 0usize;
    let unknown_request_paths =
        if input.scope.full_reconcile || input.scope.affected_paths.is_empty() {
            None
        } else {
            Some(&input.scope.affected_paths)
        };
    let unknown_policy = select_unknown_target_policy(
        input.repo_id,
        input.generation_seq,
        &target_summaries,
        &facts_by_target,
        &assignments,
        &adjudicated_targets,
        unknown_request_paths,
    );
    let role_mining_clusters = super::rule_mining::build_rule_mining_clusters(
        input.repo_id,
        unknown_policy.rule_mining_inputs.clone(),
        3,
    );
    for request in unknown_policy.adjudication_requests.iter().cloned() {
        if seen_adjudication_scopes.insert(request.scope_key()) {
            unknown_or_high_impact_candidates += 1;
            adjudication_requests.push(request);
        }
    }
    let removed_paths = removed_assignment_paths.iter().cloned().collect::<Vec<_>>();
    let apply_outcome = replace_role_classification_state(
        relational,
        RoleClassificationStateReplacement {
            repo_id: input.repo_id,
            fact_and_signal_paths: &fact_and_signal_paths,
            facts: &extraction.facts,
            signals: &rule_result.signals,
            assignment_paths: &assignment_refresh_paths,
            assignments: &current_assignments,
            assignment_history_writes: &history_writes,
            removed_assignment_paths: &removed_paths,
            generation_seq: input.generation_seq,
        },
    )
    .await
    .context("replacing architecture role classification state for current state")?;
    let write_counts = apply_outcome.write_counts;
    let mut expanded_metrics = expanded_reconcile_metrics(
        &target_summaries,
        &current_assignments,
        &adjudication_requests,
        0,
        0,
    );
    expanded_metrics.unknown_targets_total = unknown_policy.unknown_targets_total;
    expanded_metrics.unknown_targets_suppressed_non_role = unknown_policy.suppressed_non_role;
    expanded_metrics.unknown_targets_rule_mining_eligible = unknown_policy.rule_mining_eligible;
    expanded_metrics.unknown_targets_adjudication_escalated = unknown_policy.adjudication_escalated;
    expanded_metrics.role_mining_clusters = role_mining_clusters.len();
    expanded_metrics.role_mining_representative_targets = role_mining_clusters
        .iter()
        .map(|cluster| cluster.representative_target_ids.len())
        .sum();

    Ok(ArchitectureRoleReconcileOutcome {
        metrics: ArchitectureRoleReconcileMetrics {
            phase_name: Some(
                crate::capability_packs::architecture_graph::types::ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID.to_string(),
            ),
            reconcile_mode: Some(if input.scope.full_reconcile {
                "full_reconcile".to_string()
            } else {
                "merged_delta".to_string()
            }),
            skipped_inactive: false,
            full_reconcile: input.scope.full_reconcile,
            affected_paths: input.scope.affected_paths.len(),
            refreshed_paths: extraction.refreshed_paths.len(),
            removed_paths: removed_assignment_paths.len(),
            skipped_unchanged_paths,
            facts_written: write_counts.facts_written,
            facts_deleted: write_counts.facts_deleted,
            rules_loaded: rules.len(),
            signals_written: write_counts.signals_written,
            signals_deleted: write_counts.signals_deleted,
            assignments_written: write_counts.assignments_written,
            assignments_marked_stale: write_counts.assignments_marked_stale,
            assignment_history_rows: write_counts.assignment_history_rows,
            adjudication_candidates: adjudication_candidates.len()
                + unknown_or_high_impact_candidates,
            target_count: expanded_metrics.target_count,
            deterministic_active_targets: expanded_metrics.deterministic_active_targets,
            deterministic_needs_review_targets: expanded_metrics
                .deterministic_needs_review_targets,
            deterministic_conflict_targets: expanded_metrics.deterministic_conflict_targets,
            deterministic_unassigned_targets: expanded_metrics.deterministic_unassigned_targets,
            unknown_targets_total: expanded_metrics.unknown_targets_total,
            unknown_targets_suppressed_non_role: expanded_metrics
                .unknown_targets_suppressed_non_role,
            unknown_targets_rule_mining_eligible: expanded_metrics
                .unknown_targets_rule_mining_eligible,
            unknown_targets_adjudication_escalated: expanded_metrics
                .unknown_targets_adjudication_escalated,
            role_mining_clusters: expanded_metrics.role_mining_clusters,
            role_mining_representative_targets: expanded_metrics
                .role_mining_representative_targets,
            unknown_adjudication_candidates: expanded_metrics.unknown_adjudication_candidates,
            high_impact_adjudication_candidates: expanded_metrics
                .high_impact_adjudication_candidates,
            low_confidence_adjudication_candidates: expanded_metrics
                .low_confidence_adjudication_candidates,
            conflict_adjudication_candidates: expanded_metrics.conflict_adjudication_candidates,
            repeated_adjudication_suppressed: expanded_metrics.repeated_adjudication_suppressed,
            deterministic_guard_skipped: expanded_metrics.deterministic_guard_skipped,
            deterministic_coverage_ratio: expanded_metrics.deterministic_coverage_ratio,
            needs_review_ratio: expanded_metrics.needs_review_ratio,
            unknown_ratio: expanded_metrics.unknown_ratio,
            transaction_count: apply_outcome.sqlite_phase_metrics.transaction_count,
            max_rss_kb: apply_outcome.max_rss_kb,
            max_sqlite_lock_wait_ms: apply_outcome.sqlite_phase_metrics.max_wait_ms,
            max_sqlite_lock_hold_ms: apply_outcome.sqlite_phase_metrics.max_hold_ms,
            file_batches: usize::from(input.scope.full_reconcile),
        },
        warnings: Vec::new(),
        architecture_embedding_refresh_paths: assignment_refresh_paths,
        architecture_embedding_cleanup_paths: removed_paths,
        adjudication_requests,
    })
}

pub fn adjudication_requests_from_assignments(
    assignments: &[ArchitectureRoleAssignment],
) -> Vec<RoleAdjudicationRequest> {
    let mut grouped: BTreeMap<RoleTarget, Vec<ArchitectureRoleAssignment>> = BTreeMap::new();
    for assignment in assignments
        .iter()
        .filter(|assignment| assignment.status == AssignmentStatus::NeedsReview)
    {
        grouped
            .entry(assignment.target.clone())
            .or_default()
            .push(assignment.clone());
    }

    let config = AssignmentAggregationConfig::default();
    grouped
        .into_iter()
        .filter_map(|(target, mut target_assignments)| {
            target_assignments.sort_by(|left, right| {
                right
                    .confidence
                    .partial_cmp(&left.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.role_id.cmp(&right.role_id))
            });
            let primary = target_assignments.first()?;
            let top_confidence = primary.confidence;
            let reason =
                if target_assignments.iter().skip(1).any(|assignment| {
                    top_confidence - assignment.confidence <= config.conflict_margin
                }) {
                    AdjudicationReason::Conflict
                } else {
                    AdjudicationReason::LowConfidence
                };
            let candidate_role_ids = target_assignments
                .iter()
                .map(|assignment| assignment.role_id.clone())
                .collect::<Vec<_>>();
            let (target_kind, artefact_id, symbol_id) = request_target_fields(&target);
            let stable_request_key = role_adjudication_stable_request_key(
                target_kind.as_deref(),
                artefact_id.as_deref(),
                symbol_id.as_deref(),
                Some(&target.path),
            );
            Some(RoleAdjudicationRequest {
                repo_id: primary.repo_id.clone(),
                generation: primary.generation_seq,
                stable_request_key,
                facts_hash: placeholder_request_hash(),
                rules_hash: placeholder_request_hash(),
                cluster_key: None,
                target_kind,
                artefact_id,
                symbol_id,
                path: Some(target.path.clone()),
                language: None,
                canonical_kind: None,
                reason,
                deterministic_confidence: Some(top_confidence),
                candidate_role_ids,
                current_assignment: Some(RoleCurrentAssignmentSnapshot {
                    role_id: primary.role_id.clone(),
                    confidence: Some(primary.confidence),
                    source: Some(primary.source.as_db().to_string()),
                }),
            })
        })
        .collect()
}

fn request_target_fields(target: &RoleTarget) -> (Option<String>, Option<String>, Option<String>) {
    (
        Some(target.target_kind.as_db().to_string()),
        target.artefact_id.clone(),
        target.symbol_id.clone(),
    )
}

fn refreshed_paths_with_removals(
    refreshed_paths: &[String],
    removed_paths: &BTreeSet<String>,
) -> Vec<String> {
    refreshed_paths
        .iter()
        .cloned()
        .chain(removed_paths.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn assignment_refresh_paths(
    refreshed_paths: &[String],
    removed_paths: &BTreeSet<String>,
) -> Vec<String> {
    refreshed_paths
        .iter()
        .filter(|path| !removed_paths.contains(path.as_str()))
        .cloned()
        .collect()
}

#[derive(Debug, Default)]
struct AggregatedSignals {
    positive_scores: Vec<f64>,
    negative_scores: Vec<f64>,
    generation_seq: u64,
    evidence: Vec<serde_json::Value>,
}

impl AggregatedSignals {
    fn confidence(&self) -> f64 {
        let positive_confidence = noisy_or(&self.positive_scores);
        let negative_confidence = noisy_or(&self.negative_scores);
        positive_confidence * (1.0 - negative_confidence)
    }

    fn evidence_json(self) -> serde_json::Value {
        let mut evidence = Vec::with_capacity(self.evidence.len() + 1);
        evidence.push(json!({ "source": "rule_signal_aggregation" }));
        evidence.extend(self.evidence);
        json!(evidence)
    }
}

fn noisy_or(scores: &[f64]) -> f64 {
    1.0 - scores
        .iter()
        .map(|score| 1.0 - score.clamp(0.0, 1.0))
        .product::<f64>()
}

#[cfg(test)]
mod tests;
