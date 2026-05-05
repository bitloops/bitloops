use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};

use serde_json::json;

use crate::host::devql::RelationalStorage;
use crate::models::{
    CurrentCanonicalArtefactRecord, CurrentCanonicalEdgeRecord, CurrentCanonicalFileRecord,
};

use super::fact_extraction::{
    ArchitectureRoleFactExtractionInput, extract_architecture_role_facts,
};
use super::llm_adjudication::queue_ambiguous_role_adjudication;
use super::rules::{compile_detection_rules, evaluate_rules_over_facts};
use super::storage::{
    AssignmentHistoryWrite, RoleClassificationStateReplacement, load_active_detection_rules,
    load_assignments_for_paths, replace_role_classification_state,
};
use super::taxonomy::{
    ArchitectureRoleAssignment, ArchitectureRoleReconcileMetrics, ArchitectureRoleReconcileOutcome,
    ArchitectureRoleRuleSignal, AssignmentPriority, AssignmentSource, AssignmentStatus,
    RoleSignalPolarity, RoleTarget, assignment_id,
};

pub const ARCHITECTURE_ROLE_CLASSIFIER_VERSION: &str =
    "architecture_roles.deterministic.contract.v1";

#[derive(Debug, Clone)]
pub struct ArchitectureRoleClassificationInput<'a> {
    pub repo_id: &'a str,
    pub generation_seq: u64,
    pub affected_paths: BTreeSet<String>,
    pub removed_paths: BTreeSet<String>,
    pub files: &'a [CurrentCanonicalFileRecord],
    pub artefacts: &'a [CurrentCanonicalArtefactRecord],
    pub dependency_edges: &'a [CurrentCanonicalEdgeRecord],
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

pub async fn classify_architecture_roles_for_current_state(
    relational: &RelationalStorage,
    input: ArchitectureRoleClassificationInput<'_>,
) -> Result<ArchitectureRoleReconcileOutcome> {
    let extraction = extract_architecture_role_facts(ArchitectureRoleFactExtractionInput {
        repo_id: input.repo_id,
        generation_seq: input.generation_seq,
        affected_paths: &input.affected_paths,
        files: input.files,
        artefacts: input.artefacts,
        dependency_edges: input.dependency_edges,
    });

    let fact_and_signal_paths =
        refreshed_paths_with_removals(&extraction.refreshed_paths, &input.removed_paths);
    let rules = load_active_detection_rules(relational, input.repo_id).await?;
    let compiled = compile_detection_rules(rules.clone())?;
    let rule_result = evaluate_rules_over_facts(&compiled, &extraction.facts)?;
    let assignments = aggregate_role_assignments(
        input.repo_id,
        &rule_result.signals,
        AssignmentAggregationConfig::default(),
    );
    let assignment_refresh_paths =
        assignment_refresh_paths(&extraction.refreshed_paths, &input.removed_paths);
    let previous_assignments =
        load_assignments_for_paths(relational, input.repo_id, &assignment_refresh_paths).await?;
    let previous_by_id = previous_assignments
        .iter()
        .map(|assignment| (assignment.assignment_id.clone(), assignment))
        .collect::<BTreeMap<_, _>>();
    let mut current_assignments = assignments.clone();
    let mut history_writes = Vec::new();
    for assignment in &assignments {
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
    let next_by_id = assignments
        .iter()
        .map(|assignment| assignment.assignment_id.as_str())
        .collect::<BTreeSet<_>>();
    for previous in previous_assignments
        .iter()
        .filter(|previous| previous.status == AssignmentStatus::Active)
        .filter(|previous| !next_by_id.contains(previous.assignment_id.as_str()))
    {
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
        current_assignments.push(previous.clone());
    }
    let adjudication_candidates = assignments
        .iter()
        .filter(|assignment| assignment.status == AssignmentStatus::NeedsReview)
        .cloned()
        .collect::<Vec<_>>();
    queue_ambiguous_role_adjudication(&adjudication_candidates)
        .await
        .with_context(|| {
            format!(
                "queueing architecture role adjudication candidates for repo {} generation {}",
                input.repo_id, input.generation_seq
            )
        })?;
    let removed_paths = input.removed_paths.iter().cloned().collect::<Vec<_>>();
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
            affected_paths: input.affected_paths.len(),
            facts_written: write_counts.facts_written,
            facts_deleted: 0,
            rules_loaded: rules.len(),
            signals_written: write_counts.signals_written,
            assignments_written: write_counts.assignments_written,
            assignments_marked_stale: write_counts.assignments_marked_stale,
            assignment_history_rows: write_counts.assignment_history_rows,
            adjudication_candidates: adjudication_candidates.len(),
        },
        warnings: Vec::new(),
    })
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
mod tests {
    use super::super::taxonomy::{
        ArchitectureRoleRuleSignal, AssignmentPriority, AssignmentStatus, RoleSignalPolarity,
        RoleTarget,
    };
    use super::*;

    #[test]
    fn aggregate_role_assignments_marks_low_confidence_needs_review() {
        let target = RoleTarget::file("src/main.rs");
        let signals = vec![ArchitectureRoleRuleSignal {
            repo_id: "repo-1".to_string(),
            signal_id: "signal-1".to_string(),
            rule_id: "rule-1".to_string(),
            rule_version: 1,
            role_id: "role-1".to_string(),
            target,
            polarity: RoleSignalPolarity::Positive,
            score: 0.5,
            evidence: serde_json::json!([]),
            generation_seq: 10,
        }];

        let assignments = aggregate_role_assignments(
            "repo-1",
            &signals,
            AssignmentAggregationConfig {
                active_threshold: 0.65,
                review_threshold: 0.35,
                conflict_margin: 0.05,
            },
        );

        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].status, AssignmentStatus::NeedsReview);
        assert_eq!(assignments[0].priority, AssignmentPriority::Primary);
        assert_eq!(assignments[0].confidence, 0.5);
        assert_eq!(assignments[0].generation_seq, 10);
    }

    #[test]
    fn aggregate_role_assignments_uses_generation_seq_from_contributing_signals() {
        let signals = vec![
            ArchitectureRoleRuleSignal {
                repo_id: "repo-1".to_string(),
                signal_id: "signal-1".to_string(),
                rule_id: "rule-1".to_string(),
                rule_version: 1,
                role_id: "role-1".to_string(),
                target: RoleTarget::file("src/main.rs"),
                polarity: RoleSignalPolarity::Positive,
                score: 0.7,
                evidence: serde_json::json!([]),
                generation_seq: 2,
            },
            ArchitectureRoleRuleSignal {
                repo_id: "repo-1".to_string(),
                signal_id: "signal-2".to_string(),
                rule_id: "rule-2".to_string(),
                rule_version: 1,
                role_id: "role-2".to_string(),
                target: RoleTarget::file("src/lib.rs"),
                polarity: RoleSignalPolarity::Positive,
                score: 0.9,
                evidence: serde_json::json!([]),
                generation_seq: 99,
            },
        ];

        let assignments =
            aggregate_role_assignments("repo-1", &signals, AssignmentAggregationConfig::default());
        let assignment = assignments
            .iter()
            .find(|assignment| assignment.role_id == "role-1")
            .expect("role-1 assignment should be produced");

        assert_eq!(assignment.generation_seq, 2);
        assert_eq!(assignment.evidence[0]["source"], "rule_signal_aggregation");
        assert_eq!(assignment.evidence[1]["signalId"], "signal-1");
        assert_eq!(assignment.evidence[1]["ruleId"], "rule-1");
        assert_eq!(assignment.evidence[1]["ruleVersion"], 1);
        assert_eq!(assignment.evidence[1]["polarity"], "positive");
        assert_eq!(assignment.evidence[1]["score"], 0.7);
        assert!(
            assignment
                .evidence
                .as_array()
                .expect("evidence should be an array")
                .iter()
                .all(|entry| entry["signalId"] != "signal-2")
        );
    }

    #[test]
    fn aggregate_role_assignments_marks_top_role_conflicts_needs_review() {
        let target = RoleTarget::file("src/main.rs");
        let signals = vec![
            ArchitectureRoleRuleSignal {
                repo_id: "repo-1".to_string(),
                signal_id: "signal-1".to_string(),
                rule_id: "rule-1".to_string(),
                rule_version: 1,
                role_id: "role-b".to_string(),
                target: target.clone(),
                polarity: RoleSignalPolarity::Positive,
                score: 0.86,
                evidence: serde_json::json!([]),
                generation_seq: 1,
            },
            ArchitectureRoleRuleSignal {
                repo_id: "repo-1".to_string(),
                signal_id: "signal-2".to_string(),
                rule_id: "rule-2".to_string(),
                rule_version: 1,
                role_id: "role-a".to_string(),
                target,
                polarity: RoleSignalPolarity::Positive,
                score: 0.84,
                evidence: serde_json::json!([]),
                generation_seq: 1,
            },
        ];

        let assignments =
            aggregate_role_assignments("repo-1", &signals, AssignmentAggregationConfig::default());

        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments[0].role_id, "role-b");
        assert_eq!(assignments[0].status, AssignmentStatus::NeedsReview);
        assert_eq!(assignments[1].role_id, "role-a");
        assert_eq!(assignments[1].status, AssignmentStatus::NeedsReview);
    }

    #[test]
    fn aggregate_role_assignments_sorts_equal_confidence_by_role_id() {
        let target = RoleTarget::file("src/main.rs");
        let signals = vec![
            ArchitectureRoleRuleSignal {
                repo_id: "repo-1".to_string(),
                signal_id: "signal-1".to_string(),
                rule_id: "rule-1".to_string(),
                rule_version: 1,
                role_id: "role-b".to_string(),
                target: target.clone(),
                polarity: RoleSignalPolarity::Positive,
                score: 0.7,
                evidence: serde_json::json!([]),
                generation_seq: 1,
            },
            ArchitectureRoleRuleSignal {
                repo_id: "repo-1".to_string(),
                signal_id: "signal-2".to_string(),
                rule_id: "rule-2".to_string(),
                rule_version: 1,
                role_id: "role-a".to_string(),
                target,
                polarity: RoleSignalPolarity::Positive,
                score: 0.7,
                evidence: serde_json::json!([]),
                generation_seq: 1,
            },
        ];

        let assignments =
            aggregate_role_assignments("repo-1", &signals, AssignmentAggregationConfig::default());

        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments[0].role_id, "role-a");
        assert_eq!(assignments[1].role_id, "role-b");
    }

    #[test]
    fn aggregate_role_assignments_carries_signal_evidence_payload() {
        let signals = vec![ArchitectureRoleRuleSignal {
            repo_id: "repo-1".to_string(),
            signal_id: "signal-1".to_string(),
            rule_id: "rule-1".to_string(),
            rule_version: 1,
            role_id: "role-1".to_string(),
            target: RoleTarget::file("src/main.rs"),
            polarity: RoleSignalPolarity::Positive,
            score: 0.9,
            evidence: serde_json::json!([{ "factId": "fact-1" }]),
            generation_seq: 1,
        }];

        let assignments =
            aggregate_role_assignments("repo-1", &signals, AssignmentAggregationConfig::default());

        assert_eq!(assignments.len(), 1);
        assert_eq!(
            assignments[0].evidence[1]["evidence"][0]["factId"],
            "fact-1"
        );
    }

    #[test]
    fn artefact_removals_affect_but_do_not_remove_role_paths() {
        let request = crate::host::capability_host::CurrentStateConsumerRequest {
            run_id: Some("run".to_string()),
            repo_id: "repo-1".to_string(),
            repo_root: std::path::PathBuf::from("/tmp/repo"),
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("abc123".to_string()),
            from_generation_seq_exclusive: 1,
            to_generation_seq_inclusive: 2,
            reconcile_mode: crate::host::capability_host::ReconcileMode::MergedDelta,
            file_upserts: Vec::new(),
            file_removals: Vec::new(),
            affected_paths: Vec::new(),
            artefact_upserts: Vec::new(),
            artefact_removals: vec![crate::host::capability_host::RemovedArtefact {
                artefact_id: "artefact-1".to_string(),
                symbol_id: "symbol-1".to_string(),
                path: "src/api.rs".to_string(),
            }],
        };

        assert!(affected_role_paths_from_request(&request).contains("src/api.rs"));
        assert!(!removed_role_paths_from_request(&request).contains("src/api.rs"));
    }

    #[tokio::test]
    async fn classification_extracts_facts_runs_rules_and_writes_assignment() -> anyhow::Result<()>
    {
        let temp = tempfile::TempDir::new()?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let conn = rusqlite::Connection::open(&sqlite_path)?;
        conn.execute_batch(
            crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql(),
        )?;
        drop(conn);
        let relational = crate::host::devql::RelationalStorage::local_only(sqlite_path);

        let role = super::super::taxonomy::ArchitectureRole {
            repo_id: "repo-1".to_string(),
            role_id: super::super::taxonomy::stable_role_id("repo-1", "application", "entrypoint"),
            family: "application".to_string(),
            slug: "entrypoint".to_string(),
            display_name: "Entrypoint".to_string(),
            description: "Entrypoint role".to_string(),
            lifecycle: super::super::taxonomy::RoleLifecycle::Active,
            provenance: serde_json::json!({ "source": "test" }),
        };
        super::super::storage::upsert_classification_role(&relational, &role).await?;
        super::super::storage::upsert_detection_rule(
            &relational,
            &super::super::taxonomy::ArchitectureRoleDetectionRule {
                repo_id: "repo-1".to_string(),
                rule_id: super::super::taxonomy::rule_id(
                    "repo-1",
                    &role.role_id,
                    "main-entrypoint",
                ),
                role_id: role.role_id.clone(),
                version: 1,
                lifecycle: super::super::taxonomy::RoleRuleLifecycle::Active,
                priority: 10,
                score: 1.0,
                candidate_selector: serde_json::json!({
                    "targetKinds": ["file"],
                    "pathSuffixes": [".rs"]
                }),
                positive_conditions: serde_json::json!([
                    {
                        "kind": "path",
                        "key": "full",
                        "op": "suffix",
                        "value": "main.rs",
                        "score": 0.9
                    }
                ]),
                negative_conditions: serde_json::json!([]),
                provenance: serde_json::json!({ "source": "test" }),
            },
        )
        .await?;

        let previous_role = super::super::taxonomy::ArchitectureRole {
            repo_id: "repo-1".to_string(),
            role_id: super::super::taxonomy::stable_role_id("repo-1", "application", "previous"),
            family: "application".to_string(),
            slug: "previous".to_string(),
            display_name: "Previous".to_string(),
            description: "Previous role".to_string(),
            lifecycle: super::super::taxonomy::RoleLifecycle::Active,
            provenance: serde_json::json!({ "source": "test" }),
        };
        super::super::storage::upsert_classification_role(&relational, &previous_role).await?;
        let previous_target = RoleTarget::file("src/main.rs");
        let previous_role_id = previous_role.role_id.clone();
        super::super::storage::upsert_assignment(
            &relational,
            &ArchitectureRoleAssignment {
                repo_id: "repo-1".to_string(),
                assignment_id: super::super::taxonomy::assignment_id(
                    "repo-1",
                    &previous_role.role_id,
                    &previous_target,
                ),
                role_id: previous_role_id.clone(),
                target: previous_target,
                priority: AssignmentPriority::Primary,
                status: AssignmentStatus::Active,
                source: AssignmentSource::Rule,
                confidence: 0.91,
                evidence: serde_json::json!([]),
                provenance: serde_json::json!({ "source": "test" }),
                classifier_version: "previous".to_string(),
                rule_version: Some(1),
                generation_seq: 0,
            },
        )
        .await?;
        let stale_role = super::super::taxonomy::ArchitectureRole {
            repo_id: "repo-1".to_string(),
            role_id: super::super::taxonomy::stable_role_id("repo-1", "application", "stale"),
            family: "application".to_string(),
            slug: "stale".to_string(),
            display_name: "Stale".to_string(),
            description: "Stale role".to_string(),
            lifecycle: super::super::taxonomy::RoleLifecycle::Active,
            provenance: serde_json::json!({ "source": "test" }),
        };
        super::super::storage::upsert_classification_role(&relational, &stale_role).await?;
        let stale_target = RoleTarget::file("src/main.rs");
        super::super::storage::upsert_assignment(
            &relational,
            &ArchitectureRoleAssignment {
                repo_id: "repo-1".to_string(),
                assignment_id: super::super::taxonomy::assignment_id(
                    "repo-1",
                    &stale_role.role_id,
                    &stale_target,
                ),
                role_id: stale_role.role_id.clone(),
                target: stale_target,
                priority: AssignmentPriority::Secondary,
                status: AssignmentStatus::Stale,
                source: AssignmentSource::Rule,
                confidence: 0.51,
                evidence: serde_json::json!([]),
                provenance: serde_json::json!({ "source": "test" }),
                classifier_version: "previous".to_string(),
                rule_version: Some(1),
                generation_seq: 0,
            },
        )
        .await?;

        let files = vec![crate::models::CurrentCanonicalFileRecord {
            repo_id: "repo-1".to_string(),
            path: "src/main.rs".to_string(),
            analysis_mode: "code".to_string(),
            file_role: "source".to_string(),
            language: "rust".to_string(),
            resolved_language: "rust".to_string(),
            effective_content_id: "content-1".to_string(),
            parser_version: "parser".to_string(),
            extractor_version: "extractor".to_string(),
            exists_in_head: true,
            exists_in_index: true,
            exists_in_worktree: true,
        }];

        let outcome = classify_architecture_roles_for_current_state(
            &relational,
            ArchitectureRoleClassificationInput {
                repo_id: "repo-1",
                generation_seq: 1,
                affected_paths: std::collections::BTreeSet::from(["src/main.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
                files: &files,
                artefacts: &[],
                dependency_edges: &[],
            },
        )
        .await?;

        let assignments =
            super::super::storage::load_assignments_for_path(&relational, "repo-1", "src/main.rs")
                .await?;
        assert!(outcome.metrics.facts_written > 0);
        assert_eq!(outcome.metrics.rules_loaded, 1);
        assert_eq!(outcome.metrics.signals_written, 1);
        assert_eq!(outcome.metrics.assignments_written, 3);
        assert_eq!(outcome.metrics.assignment_history_rows, 2);
        assert_eq!(assignments.len(), 3);
        assert!(
            assignments
                .iter()
                .any(|assignment| assignment.role_id == role.role_id
                    && assignment.status == AssignmentStatus::Active)
        );
        assert!(assignments.iter().any(|assignment| {
            assignment.role_id == previous_role_id
                && assignment.status == AssignmentStatus::Stale
                && assignment.generation_seq == 1
        }));
        assert!(assignments.iter().any(|assignment| {
            assignment.role_id == stale_role.role_id
                && assignment.status == AssignmentStatus::Stale
                && assignment.generation_seq == 0
        }));
        Ok(())
    }

    #[tokio::test]
    async fn classification_marks_removed_path_assignment_stale_and_counts_history()
    -> anyhow::Result<()> {
        let temp = tempfile::TempDir::new()?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let conn = rusqlite::Connection::open(&sqlite_path)?;
        conn.execute_batch(
            crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql(),
        )?;
        drop(conn);
        let relational = crate::host::devql::RelationalStorage::local_only(sqlite_path);

        let role = super::super::taxonomy::ArchitectureRole {
            repo_id: "repo-1".to_string(),
            role_id: super::super::taxonomy::stable_role_id("repo-1", "application", "entrypoint"),
            family: "application".to_string(),
            slug: "entrypoint".to_string(),
            display_name: "Entrypoint".to_string(),
            description: "Entrypoint role".to_string(),
            lifecycle: super::super::taxonomy::RoleLifecycle::Active,
            provenance: serde_json::json!({ "source": "test" }),
        };
        super::super::storage::upsert_classification_role(&relational, &role).await?;
        let target = RoleTarget::file("src/removed.rs");
        super::super::storage::upsert_assignment(
            &relational,
            &ArchitectureRoleAssignment {
                repo_id: "repo-1".to_string(),
                assignment_id: super::super::taxonomy::assignment_id(
                    "repo-1",
                    &role.role_id,
                    &target,
                ),
                role_id: role.role_id,
                target,
                priority: AssignmentPriority::Primary,
                status: AssignmentStatus::Active,
                source: AssignmentSource::Rule,
                confidence: 0.91,
                evidence: serde_json::json!([]),
                provenance: serde_json::json!({ "source": "test" }),
                classifier_version: "previous".to_string(),
                rule_version: Some(1),
                generation_seq: 1,
            },
        )
        .await?;

        let outcome = classify_architecture_roles_for_current_state(
            &relational,
            ArchitectureRoleClassificationInput {
                repo_id: "repo-1",
                generation_seq: 2,
                affected_paths: std::collections::BTreeSet::from(["src/removed.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::from(["src/removed.rs".to_string()]),
                files: &[],
                artefacts: &[],
                dependency_edges: &[],
            },
        )
        .await?;

        let assignments = super::super::storage::load_assignments_for_path(
            &relational,
            "repo-1",
            "src/removed.rs",
        )
        .await?;
        assert_eq!(outcome.metrics.assignments_marked_stale, 1);
        assert_eq!(outcome.metrics.assignment_history_rows, 1);
        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].status, AssignmentStatus::Stale);
        assert_eq!(assignments[0].generation_seq, 2);
        Ok(())
    }

    #[tokio::test]
    async fn classification_counts_needs_review_adjudication_candidates() -> anyhow::Result<()> {
        let temp = tempfile::TempDir::new()?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let conn = rusqlite::Connection::open(&sqlite_path)?;
        conn.execute_batch(
            crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql(),
        )?;
        drop(conn);
        let relational = crate::host::devql::RelationalStorage::local_only(sqlite_path);

        let role = super::super::taxonomy::ArchitectureRole {
            repo_id: "repo-1".to_string(),
            role_id: super::super::taxonomy::stable_role_id("repo-1", "application", "entrypoint"),
            family: "application".to_string(),
            slug: "entrypoint".to_string(),
            display_name: "Entrypoint".to_string(),
            description: "Entrypoint role".to_string(),
            lifecycle: super::super::taxonomy::RoleLifecycle::Active,
            provenance: serde_json::json!({ "source": "test" }),
        };
        super::super::storage::upsert_classification_role(&relational, &role).await?;
        super::super::storage::upsert_detection_rule(
            &relational,
            &super::super::taxonomy::ArchitectureRoleDetectionRule {
                repo_id: "repo-1".to_string(),
                rule_id: super::super::taxonomy::rule_id(
                    "repo-1",
                    &role.role_id,
                    "main-entrypoint",
                ),
                role_id: role.role_id.clone(),
                version: 1,
                lifecycle: super::super::taxonomy::RoleRuleLifecycle::Active,
                priority: 10,
                score: 1.0,
                candidate_selector: serde_json::json!({ "targetKinds": ["file"] }),
                positive_conditions: serde_json::json!([
                    {
                        "kind": "path",
                        "key": "full",
                        "op": "suffix",
                        "value": "main.rs",
                        "score": 0.6
                    }
                ]),
                negative_conditions: serde_json::json!([]),
                provenance: serde_json::json!({ "source": "test" }),
            },
        )
        .await?;

        let files = vec![crate::models::CurrentCanonicalFileRecord {
            repo_id: "repo-1".to_string(),
            path: "src/main.rs".to_string(),
            analysis_mode: "code".to_string(),
            file_role: "source".to_string(),
            language: "rust".to_string(),
            resolved_language: "rust".to_string(),
            effective_content_id: "content-1".to_string(),
            parser_version: "parser".to_string(),
            extractor_version: "extractor".to_string(),
            exists_in_head: true,
            exists_in_index: true,
            exists_in_worktree: true,
        }];

        let outcome = classify_architecture_roles_for_current_state(
            &relational,
            ArchitectureRoleClassificationInput {
                repo_id: "repo-1",
                generation_seq: 1,
                affected_paths: std::collections::BTreeSet::from(["src/main.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
                files: &files,
                artefacts: &[],
                dependency_edges: &[],
            },
        )
        .await?;

        assert_eq!(outcome.metrics.assignments_written, 1);
        assert_eq!(outcome.metrics.adjudication_candidates, 1);
        Ok(())
    }
}
