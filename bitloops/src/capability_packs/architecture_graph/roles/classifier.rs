use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};

use serde_json::json;

use crate::host::devql::RelationalStorage;
use crate::models::CurrentCanonicalFileRecord;

use super::adjudication_selector::{DeterministicRoleOutcomeInput, select_adjudication_reason};
use super::contracts::{
    AdjudicationReason, RoleAdjudicationRequest, RoleCurrentAssignmentSnapshot,
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
    ArchitectureArtefactFact, ArchitectureRoleAssignment, ArchitectureRoleReconcileMetrics,
    ArchitectureRoleReconcileOutcome, ArchitectureRoleRuleSignal, AssignmentPriority,
    AssignmentSource, AssignmentStatus, RoleSignalPolarity, RoleTarget, assignment_id,
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
            RoleSignalPolarity::Positive => entry.score += signal.score,
            RoleSignalPolarity::Negative => entry.score -= signal.score,
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
    for ((target, role_id), mut aggregated) in scores {
        let confidence = aggregated.score.clamp(0.0, 1.0);
        aggregated.score = confidence;
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
    for request in unknown_or_high_impact_requests(
        input.repo_id,
        input.generation_seq,
        target_summaries,
        &assignments,
        &adjudicated_targets,
        unknown_request_paths,
    ) {
        if seen_adjudication_scopes.insert(request.scope_key()) {
            unknown_or_high_impact_candidates += 1;
            adjudication_requests.push(request);
        }
    }
    let removed_paths = removed_assignment_paths.iter().cloned().collect::<Vec<_>>();
    let write_counts = replace_role_classification_state(
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

    Ok(ArchitectureRoleReconcileOutcome {
        metrics: ArchitectureRoleReconcileMetrics {
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
        },
        warnings: Vec::new(),
        adjudication_requests,
    })
}

#[derive(Debug, Clone)]
struct RoleTargetSummary {
    target: RoleTarget,
    language: Option<String>,
    canonical_kind: Option<String>,
    high_impact: bool,
}

fn target_summaries_from_facts(
    facts: &[ArchitectureArtefactFact],
) -> BTreeMap<RoleTarget, RoleTargetSummary> {
    let mut summaries = BTreeMap::new();
    for fact in facts {
        let entry = summaries
            .entry(fact.target.clone())
            .or_insert_with(|| RoleTargetSummary {
                target: fact.target.clone(),
                language: fact.language.clone(),
                canonical_kind: None,
                high_impact: false,
            });
        if entry.language.is_none() {
            entry.language = fact.language.clone();
        }
        if fact.fact_kind == "artefact" && fact.fact_key == "canonical_kind" {
            entry.canonical_kind = Some(fact.fact_value.clone());
        }
        if fact.fact_kind == "path"
            && fact.fact_key == "full"
            && (fact.fact_value == "main.rs" || fact.fact_value.ends_with("/main.rs"))
        {
            entry.high_impact = true;
        }
        if fact.fact_kind == "symbol" && fact.fact_key == "name" && fact.fact_value == "main" {
            entry.high_impact = true;
        }
    }
    summaries
}

fn unknown_or_high_impact_requests(
    repo_id: &str,
    generation_seq: u64,
    target_summaries: BTreeMap<RoleTarget, RoleTargetSummary>,
    deterministic_assignments: &[ArchitectureRoleAssignment],
    authoritative_targets: &BTreeSet<RoleTarget>,
    eligible_paths: Option<&BTreeSet<String>>,
) -> Vec<RoleAdjudicationRequest> {
    let assigned_targets = deterministic_assignments
        .iter()
        .map(|assignment| assignment.target.clone())
        .collect::<BTreeSet<_>>();
    let assigned_paths = deterministic_assignments
        .iter()
        .map(|assignment| assignment.target.path.clone())
        .collect::<BTreeSet<_>>();

    target_summaries
        .into_values()
        .filter(|summary| {
            eligible_paths
                .map(|paths| paths.contains(&summary.target.path))
                .unwrap_or(true)
        })
        .filter(|summary| !assigned_targets.contains(&summary.target))
        .filter(|summary| !assigned_paths.contains(&summary.target.path))
        .filter(|summary| !authoritative_targets.contains(&summary.target))
        .map(|summary| {
            let reason = select_adjudication_reason(&DeterministicRoleOutcomeInput {
                classification_known: false,
                best_confidence: None,
                has_conflict: false,
                high_impact: summary.high_impact,
                novel_pattern: false,
                manual_review_requested: false,
            })
            .unwrap_or(AdjudicationReason::Unknown);
            let target = summary.target;
            let (target_kind, artefact_id, symbol_id) = request_target_fields(&target);
            RoleAdjudicationRequest {
                repo_id: repo_id.to_string(),
                generation: generation_seq,
                target_kind,
                artefact_id,
                symbol_id,
                path: Some(target.path),
                language: summary.language,
                canonical_kind: summary.canonical_kind,
                reason,
                deterministic_confidence: None,
                candidate_role_ids: Vec::new(),
                current_assignment: None,
            }
        })
        .collect()
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
            Some(RoleAdjudicationRequest {
                repo_id: primary.repo_id.clone(),
                generation: primary.generation_seq,
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
    score: f64,
    generation_seq: u64,
    evidence: Vec<serde_json::Value>,
}

impl AggregatedSignals {
    fn evidence_json(self) -> serde_json::Value {
        let mut evidence = Vec::with_capacity(self.evidence.len() + 1);
        evidence.push(json!({ "source": "rule_signal_aggregation" }));
        evidence.extend(self.evidence);
        json!(evidence)
    }
}

#[cfg(test)]
mod tests;
