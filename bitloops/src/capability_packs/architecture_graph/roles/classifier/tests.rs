use super::super::contracts::AdjudicationReason;
use super::super::fact_extraction::SliceArchitectureRoleCurrentStateSource;
use super::super::taxonomy::{
    ArchitectureArtefactFact, ArchitectureRoleRuleSignal, AssignmentPriority, AssignmentSource,
    AssignmentStatus, RoleSignalPolarity, RoleTarget,
};
use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn empty_current_state() -> SliceArchitectureRoleCurrentStateSource<'static> {
    SliceArchitectureRoleCurrentStateSource::new(&[], &[])
}

fn classifier_storage() -> anyhow::Result<(tempfile::TempDir, crate::host::devql::RelationalStorage)>
{
    let temp = tempfile::TempDir::new()?;
    let sqlite_path = temp.path().join("devql.sqlite");
    let conn = rusqlite::Connection::open(&sqlite_path)?;
    conn.execute_batch(
        crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql(),
    )?;
    drop(conn);
    Ok((
        temp,
        crate::host::devql::RelationalStorage::local_only(sqlite_path),
    ))
}

fn file_fixture(path: &str) -> crate::models::CurrentCanonicalFileRecord {
    crate::models::CurrentCanonicalFileRecord {
        repo_id: "repo-1".to_string(),
        path: path.to_string(),
        analysis_mode: "code".to_string(),
        file_role: "source".to_string(),
        language: "rust".to_string(),
        resolved_language: "rust".to_string(),
        effective_content_id: format!("content:{path}"),
        parser_version: "parser".to_string(),
        extractor_version: "extractor".to_string(),
        exists_in_head: true,
        exists_in_index: true,
        exists_in_worktree: true,
    }
}

fn artefact_fixture(path: &str, name: &str) -> crate::models::CurrentCanonicalArtefactRecord {
    crate::models::CurrentCanonicalArtefactRecord {
        repo_id: "repo-1".to_string(),
        path: path.to_string(),
        content_id: format!("content:{path}"),
        symbol_id: format!("symbol-{name}"),
        artefact_id: format!("artefact-{name}"),
        language: "rust".to_string(),
        extraction_fingerprint: format!("fingerprint-{name}"),
        canonical_kind: Some("function".to_string()),
        language_kind: Some("function_item".to_string()),
        symbol_fqn: Some(format!("src/application/create_user.rs::{name}")),
        parent_symbol_id: None,
        parent_artefact_id: None,
        start_line: 1,
        end_line: 5,
        start_byte: 0,
        end_byte: 40,
        signature: Some(format!("pub fn {name}(name: &str) -> String")),
        modifiers: String::new(),
        docstring: None,
    }
}

async fn insert_active_rule_for_role(
    relational: &crate::host::devql::RelationalStorage,
    role_id: &str,
    candidate_selector: serde_json::Value,
    positive_conditions: serde_json::Value,
) -> anyhow::Result<()> {
    super::super::storage::upsert_detection_rule(
        relational,
        &super::super::taxonomy::ArchitectureRoleDetectionRule {
            repo_id: "repo-1".to_string(),
            rule_id: format!("rule-{role_id}"),
            role_id: role_id.to_string(),
            version: 1,
            lifecycle: super::super::taxonomy::RoleRuleLifecycle::Active,
            priority: 100,
            score: 0.90,
            min_positive_ratio: 1.0,
            candidate_selector,
            positive_conditions,
            negative_conditions: serde_json::json!([]),
            provenance: serde_json::json!({"source": "test"}),
        },
    )
    .await
}

#[tokio::test]
async fn classifier_skips_invalid_active_rule_and_uses_remaining_rules() -> anyhow::Result<()> {
    let (_temp, relational) = classifier_storage()?;
    insert_active_rule_for_role(
        &relational,
        "valid-role",
        serde_json::json!({ "targetKinds": ["file"] }),
        serde_json::json!([
            { "kind": "path", "key": "full", "op": "eq", "value": "src/main.rs", "score": 1.0 }
        ]),
    )
    .await?;
    insert_active_rule_for_role(
        &relational,
        "invalid-role",
        serde_json::json!({ "targetKinds": ["file"] }),
        serde_json::json!([
            { "kind": "signature", "key": "contains", "op": "eq", "value": "Result", "score": 1.0 }
        ]),
    )
    .await?;

    let files = vec![file_fixture("src/main.rs")];
    let current_state = empty_current_state();
    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 1,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: BTreeSet::from(["src/main.rs".to_string()]),
                removed_paths: BTreeSet::new(),
            },
            files: &files,
        },
    )
    .await?;

    assert_eq!(outcome.metrics.rules_loaded, 2);
    assert_eq!(outcome.metrics.signals_written, 1);
    assert!(outcome.warnings.iter().any(|warning| {
        warning.contains("skipped active architecture role rule")
            && warning.contains("invalid-role")
    }));
    Ok(())
}

fn positive_signal(role_id: &str, rule_id: &str, score: f64) -> ArchitectureRoleRuleSignal {
    ArchitectureRoleRuleSignal {
        repo_id: "repo-1".to_string(),
        signal_id: format!("signal-{role_id}-{rule_id}"),
        rule_id: rule_id.to_string(),
        rule_version: 1,
        role_id: role_id.to_string(),
        target: RoleTarget::file("src/main.rs"),
        polarity: RoleSignalPolarity::Positive,
        score,
        evidence: serde_json::json!([]),
        generation_seq: 1,
    }
}

fn assignment_for_target(
    target: RoleTarget,
    status: AssignmentStatus,
) -> super::super::taxonomy::ArchitectureRoleAssignment {
    super::super::taxonomy::ArchitectureRoleAssignment {
        repo_id: "repo-1".to_string(),
        assignment_id: super::super::taxonomy::assignment_id("repo-1", "role-1", &target),
        role_id: "role-1".to_string(),
        target,
        priority: AssignmentPriority::Primary,
        status,
        source: AssignmentSource::Rule,
        confidence: 0.9,
        evidence: serde_json::json!([]),
        provenance: serde_json::json!({}),
        classifier_version: "test".to_string(),
        rule_version: Some(1),
        generation_seq: 1,
    }
}

fn request_with_reason(reason: AdjudicationReason) -> RoleAdjudicationRequest {
    RoleAdjudicationRequest {
        repo_id: "repo-1".to_string(),
        generation: 1,
        stable_request_key: "target:file".to_string(),
        facts_hash: "facts".to_string(),
        rules_hash: "rules".to_string(),
        cluster_key: None,
        target_kind: Some("file".to_string()),
        artefact_id: None,
        symbol_id: None,
        path: Some(format!("src/{}.rs", reason.as_str())),
        language: Some("rust".to_string()),
        canonical_kind: None,
        reason,
        deterministic_confidence: None,
        candidate_role_ids: Vec::new(),
        current_assignment: None,
    }
}

#[test]
fn aggregate_role_assignments_combines_multiple_positive_rules_with_noisy_or() {
    let signals = vec![
        positive_signal("role-1", "rule-a", 0.60),
        positive_signal("role-1", "rule-b", 0.60),
    ];

    let assignments =
        aggregate_role_assignments("repo-1", &signals, AssignmentAggregationConfig::default());

    assert_eq!(assignments.len(), 1);
    assert!((assignments[0].confidence - 0.84).abs() < 0.0001);
}

#[test]
fn classification_reports_deterministic_coverage_and_review_rates() {
    let active_target = RoleTarget::file("src/active.rs");
    let review_target = RoleTarget::file("src/review.rs");
    let unknown_target = RoleTarget::file("src/unknown.rs");
    let target_summaries = BTreeMap::from([
        (
            active_target.clone(),
            RoleTargetSummary {
                target: active_target.clone(),
                language: Some("rust".to_string()),
                canonical_kind: None,
                language_kind: None,
                file_role: None,
                analysis_mode: None,
                symbol_name: None,
                symbol_suffix: None,
                dependency_kinds: std::collections::BTreeSet::new(),
                high_impact: false,
            },
        ),
        (
            review_target.clone(),
            RoleTargetSummary {
                target: review_target.clone(),
                language: Some("rust".to_string()),
                canonical_kind: None,
                language_kind: None,
                file_role: None,
                analysis_mode: None,
                symbol_name: None,
                symbol_suffix: None,
                dependency_kinds: std::collections::BTreeSet::new(),
                high_impact: false,
            },
        ),
        (
            unknown_target.clone(),
            RoleTargetSummary {
                target: unknown_target,
                language: Some("rust".to_string()),
                canonical_kind: None,
                language_kind: None,
                file_role: None,
                analysis_mode: None,
                symbol_name: None,
                symbol_suffix: None,
                dependency_kinds: std::collections::BTreeSet::new(),
                high_impact: false,
            },
        ),
    ]);
    let assignments = vec![
        assignment_for_target(active_target, AssignmentStatus::Active),
        assignment_for_target(review_target, AssignmentStatus::NeedsReview),
    ];

    let metrics = expanded_reconcile_metrics(&target_summaries, &assignments, &[], 0, 0);

    assert_eq!(metrics.target_count, 3);
    assert_eq!(metrics.deterministic_active_targets, 1);
    assert_eq!(metrics.deterministic_needs_review_targets, 1);
    assert_eq!(metrics.deterministic_unassigned_targets, 1);
    assert!((metrics.deterministic_coverage_ratio - 1.0 / 3.0).abs() < 0.0001);
    assert!((metrics.needs_review_ratio - 1.0 / 3.0).abs() < 0.0001);
    assert!((metrics.unknown_ratio - 1.0 / 3.0).abs() < 0.0001);
}

#[test]
fn classification_reports_adjudication_requests_by_reason() {
    let requests = vec![
        request_with_reason(AdjudicationReason::Unknown),
        request_with_reason(AdjudicationReason::HighImpact),
        request_with_reason(AdjudicationReason::LowConfidence),
        request_with_reason(AdjudicationReason::Conflict),
    ];

    let metrics = expanded_reconcile_metrics(&BTreeMap::new(), &[], &requests, 2, 1);

    assert_eq!(metrics.unknown_adjudication_candidates, 1);
    assert_eq!(metrics.high_impact_adjudication_candidates, 1);
    assert_eq!(metrics.low_confidence_adjudication_candidates, 1);
    assert_eq!(metrics.conflict_adjudication_candidates, 1);
    assert_eq!(metrics.repeated_adjudication_suppressed, 2);
    assert_eq!(metrics.deterministic_guard_skipped, 1);
}

#[test]
fn target_summary_captures_role_bearing_facts() {
    let target = RoleTarget::artefact("artefact-1", "symbol-1", "src/main.rs");
    let facts = vec![
        ArchitectureArtefactFact {
            repo_id: "repo-1".to_string(),
            fact_id: "canonical-kind".to_string(),
            target: target.clone(),
            language: Some("rust".to_string()),
            fact_kind: "artefact".to_string(),
            fact_key: "canonical_kind".to_string(),
            fact_value: "function".to_string(),
            source: "test".to_string(),
            confidence: 1.0,
            evidence: serde_json::json!([]),
            generation_seq: 1,
        },
        ArchitectureArtefactFact {
            repo_id: "repo-1".to_string(),
            fact_id: "symbol-name".to_string(),
            target: target.clone(),
            language: Some("rust".to_string()),
            fact_kind: "symbol".to_string(),
            fact_key: "name".to_string(),
            fact_value: "main".to_string(),
            source: "test".to_string(),
            confidence: 1.0,
            evidence: serde_json::json!([]),
            generation_seq: 1,
        },
    ];

    let summaries = target_summaries_from_facts(&facts);
    let summary = summaries.get(&target).expect("summary");

    assert_eq!(summary.language.as_deref(), Some("rust"));
    assert_eq!(summary.canonical_kind.as_deref(), Some("function"));
    assert_eq!(summary.symbol_name.as_deref(), Some("main"));
    assert!(summary.high_impact);
}

#[test]
fn unknown_policy_does_not_escalate_helper_artefact_on_main_path() {
    let file_target = RoleTarget::file("src/main.rs");
    let helper_target = RoleTarget::artefact("artefact-helper", "symbol-helper", "src/main.rs");
    let facts = vec![
        ArchitectureArtefactFact {
            repo_id: "repo-1".to_string(),
            fact_id: "file-path".to_string(),
            target: file_target.clone(),
            language: Some("rust".to_string()),
            fact_kind: "path".to_string(),
            fact_key: "full".to_string(),
            fact_value: "src/main.rs".to_string(),
            source: "test".to_string(),
            confidence: 1.0,
            evidence: serde_json::json!([]),
            generation_seq: 1,
        },
        ArchitectureArtefactFact {
            repo_id: "repo-1".to_string(),
            fact_id: "helper-path".to_string(),
            target: helper_target.clone(),
            language: Some("rust".to_string()),
            fact_kind: "path".to_string(),
            fact_key: "full".to_string(),
            fact_value: "src/main.rs".to_string(),
            source: "test".to_string(),
            confidence: 1.0,
            evidence: serde_json::json!([]),
            generation_seq: 1,
        },
        ArchitectureArtefactFact {
            repo_id: "repo-1".to_string(),
            fact_id: "helper-kind".to_string(),
            target: helper_target.clone(),
            language: Some("rust".to_string()),
            fact_kind: "artefact".to_string(),
            fact_key: "canonical_kind".to_string(),
            fact_value: "function".to_string(),
            source: "test".to_string(),
            confidence: 1.0,
            evidence: serde_json::json!([]),
            generation_seq: 1,
        },
        ArchitectureArtefactFact {
            repo_id: "repo-1".to_string(),
            fact_id: "helper-name".to_string(),
            target: helper_target,
            language: Some("rust".to_string()),
            fact_kind: "symbol".to_string(),
            fact_key: "name".to_string(),
            fact_value: "helper".to_string(),
            source: "test".to_string(),
            confidence: 1.0,
            evidence: serde_json::json!([]),
            generation_seq: 1,
        },
    ];
    let summaries = target_summaries_from_facts(&facts);
    let grouped_facts = facts_by_target(&facts);

    let outcome = select_unknown_target_policy(
        "repo-1",
        1,
        &summaries,
        &grouped_facts,
        &[],
        &BTreeSet::new(),
        None,
    );

    assert_eq!(outcome.unknown_targets_total, 2);
    assert_eq!(outcome.adjudication_escalated, 1);
    assert_eq!(outcome.rule_mining_eligible, 1);
    assert_eq!(outcome.adjudication_requests.len(), 1);
    assert_eq!(
        outcome.adjudication_requests[0].target_kind.as_deref(),
        Some("file")
    );
}

#[test]
fn unknown_policy_counts_artefact_on_deterministically_assigned_file_path() {
    let file_target = RoleTarget::file("src/assigned.rs");
    let artefact_target =
        RoleTarget::artefact("artefact-assigned", "symbol-assigned", "src/assigned.rs");
    let summaries = BTreeMap::from([
        (
            file_target.clone(),
            RoleTargetSummary {
                target: file_target.clone(),
                language: Some("rust".to_string()),
                canonical_kind: None,
                language_kind: None,
                file_role: Some("source".to_string()),
                analysis_mode: Some("code".to_string()),
                symbol_name: None,
                symbol_suffix: None,
                dependency_kinds: BTreeSet::new(),
                high_impact: false,
            },
        ),
        (
            artefact_target.clone(),
            RoleTargetSummary {
                target: artefact_target.clone(),
                language: Some("rust".to_string()),
                canonical_kind: Some("function".to_string()),
                language_kind: Some("function_item".to_string()),
                file_role: None,
                analysis_mode: None,
                symbol_name: Some("assigned_helper".to_string()),
                symbol_suffix: None,
                dependency_kinds: BTreeSet::new(),
                high_impact: false,
            },
        ),
    ]);
    let assignments = vec![assignment_for_target(file_target, AssignmentStatus::Active)];

    let outcome = select_unknown_target_policy(
        "repo-1",
        1,
        &summaries,
        &BTreeMap::new(),
        &assignments,
        &BTreeSet::new(),
        None,
    );

    assert_eq!(outcome.unknown_targets_total, 1);
    assert_eq!(outcome.rule_mining_eligible, 1);
    assert_eq!(outcome.suppressed_non_role, 0);
    assert_eq!(outcome.adjudication_escalated, 0);
}

#[test]
fn unknown_policy_suppresses_mypy_cache_paths() {
    assert!(path_suppressed_for_roles(".mypy_cache/module/data.json"));
    assert!(path_suppressed_for_roles(
        "src/.mypy_cache/module.meta.json"
    ));
}

#[test]
fn adjudication_scope_key_ignores_generation_when_facts_and_rules_unchanged() {
    let mut first = request_with_reason(AdjudicationReason::Unknown);
    first.stable_request_key = "file:src/main.rs".to_string();
    first.facts_hash = "facts-a".to_string();
    first.rules_hash = "rules-a".to_string();
    let mut second = first.clone();
    second.generation = 99;

    assert_eq!(first.scope_key(), second.scope_key());
}

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
fn adjudication_requests_encode_artefact_targets_without_symbol_target_promotion() {
    let target = RoleTarget::artefact(
        "artefact-1".to_string(),
        "symbol-1".to_string(),
        "src/main.rs".to_string(),
    );
    let assignments = vec![ArchitectureRoleAssignment {
        repo_id: "repo-1".to_string(),
        assignment_id: super::super::taxonomy::assignment_id("repo-1", "role-1", &target),
        role_id: "role-1".to_string(),
        target,
        priority: AssignmentPriority::Primary,
        status: AssignmentStatus::NeedsReview,
        source: AssignmentSource::Rule,
        confidence: 0.6,
        evidence: serde_json::json!([]),
        provenance: serde_json::json!({ "source": "test" }),
        classifier_version: "test".to_string(),
        rule_version: Some(1),
        generation_seq: 7,
    }];

    let requests = adjudication_requests_from_assignments(&assignments);

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].target_kind.as_deref(), Some("artefact"));
    assert_eq!(requests[0].artefact_id.as_deref(), Some("artefact-1"));
    assert_eq!(requests[0].symbol_id.as_deref(), Some("symbol-1"));
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

#[test]
fn full_reconcile_uses_all_live_role_paths() {
    let request = crate::host::capability_host::CurrentStateConsumerRequest {
        run_id: Some("run".to_string()),
        repo_id: "repo-1".to_string(),
        repo_root: std::path::PathBuf::from("/tmp/repo"),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("abc123".to_string()),
        from_generation_seq_exclusive: 1,
        to_generation_seq_inclusive: 2,
        reconcile_mode: crate::host::capability_host::ReconcileMode::FullReconcile,
        file_upserts: Vec::new(),
        file_removals: Vec::new(),
        affected_paths: vec!["src/changed.rs".to_string()],
        artefact_upserts: Vec::new(),
        artefact_removals: Vec::new(),
    };

    let scope = role_classification_scope_from_request(&request);

    assert!(scope.full_reconcile);
    assert!(scope.affected_paths.is_empty());
    assert!(scope.removed_paths.is_empty());
}

#[test]
fn merged_delta_scope_uses_file_and_artefact_changes() {
    let request = crate::host::capability_host::CurrentStateConsumerRequest {
        run_id: Some("run".to_string()),
        repo_id: "repo-1".to_string(),
        repo_root: std::path::PathBuf::from("/tmp/repo"),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("abc123".to_string()),
        from_generation_seq_exclusive: 1,
        to_generation_seq_inclusive: 2,
        reconcile_mode: crate::host::capability_host::ReconcileMode::MergedDelta,
        file_upserts: vec![crate::host::capability_host::ChangedFile {
            path: "src/file.rs".to_string(),
            language: "rust".to_string(),
            content_id: "content:file".to_string(),
        }],
        file_removals: vec![crate::host::capability_host::RemovedFile {
            path: "src/removed.rs".to_string(),
        }],
        affected_paths: vec!["src/affected.rs".to_string()],
        artefact_upserts: vec![crate::host::capability_host::ChangedArtefact {
            artefact_id: "artefact-1".to_string(),
            symbol_id: "symbol-1".to_string(),
            path: "src/artefact.rs".to_string(),
            canonical_kind: Some("function".to_string()),
            name: "run".to_string(),
        }],
        artefact_removals: vec![crate::host::capability_host::RemovedArtefact {
            artefact_id: "artefact-2".to_string(),
            symbol_id: "symbol-2".to_string(),
            path: "src/artefact_removed.rs".to_string(),
        }],
    };

    let scope = role_classification_scope_from_request(&request);

    assert!(!scope.full_reconcile);
    assert_eq!(
        scope.affected_paths,
        std::collections::BTreeSet::from([
            "src/affected.rs".to_string(),
            "src/artefact.rs".to_string(),
            "src/artefact_removed.rs".to_string(),
            "src/file.rs".to_string(),
            "src/removed.rs".to_string(),
        ])
    );
    assert_eq!(
        scope.removed_paths,
        std::collections::BTreeSet::from(["src/removed.rs".to_string()])
    );
}

#[tokio::test]
async fn classification_extracts_facts_runs_rules_and_writes_assignment() -> anyhow::Result<()> {
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
            rule_id: super::super::taxonomy::rule_id("repo-1", &role.role_id, "main-entrypoint"),
            role_id: role.role_id.clone(),
            version: 1,
            lifecycle: super::super::taxonomy::RoleRuleLifecycle::Active,
            priority: 10,
            score: 0.9,
            min_positive_ratio: 1.0,
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
                    "score": 1.0
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

    let current_state = empty_current_state();
    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 1,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/main.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
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
async fn classification_writes_rule_assignment_for_active_file_rule() -> anyhow::Result<()> {
    let (_temp, relational) = classifier_storage()?;
    let role = super::super::taxonomy::ArchitectureRole {
        repo_id: "repo-1".to_string(),
        role_id: "role-platform".to_string(),
        family: "entrypoint".to_string(),
        slug: "platform_bootstrapper".to_string(),
        display_name: "Platform Bootstrapper".to_string(),
        description: "Startup file role".to_string(),
        lifecycle: super::super::taxonomy::RoleLifecycle::Active,
        provenance: serde_json::json!({"source": "test"}),
    };
    super::super::storage::upsert_classification_role(&relational, &role).await?;
    insert_active_rule_for_role(
        &relational,
        &role.role_id,
        serde_json::json!({
            "targetKinds": ["file"],
            "pathPrefixes": ["src"],
            "pathSuffixes": [],
            "requiredFacts": [
                { "kind": "language", "key": "resolved", "op": "eq", "value": "rust", "score": 1.0 },
                { "kind": "path", "key": "full", "op": "eq", "value": "src/main.rs", "score": 1.0 }
            ],
            "requiredFactAnyGroups": []
        }),
        serde_json::json!([
            { "kind": "file", "key": "role", "op": "eq", "value": "source_code", "score": 1.0 }
        ]),
    )
    .await?;

    let mut main_file = file_fixture("src/main.rs");
    main_file.file_role = "source_code".to_string();
    let files = vec![main_file];
    let current_state = empty_current_state();
    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 1,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: true,
                affected_paths: BTreeSet::new(),
                removed_paths: BTreeSet::new(),
            },
            files: &files,
        },
    )
    .await?;

    assert_eq!(outcome.metrics.signals_written, 1);
    assert_eq!(outcome.metrics.assignments_written, 1);
    let assignment_paths = vec!["src/main.rs".to_string()];
    let assignments =
        super::super::storage::load_assignments_for_paths(&relational, "repo-1", &assignment_paths)
            .await?;
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].source, AssignmentSource::Rule);
    assert_eq!(assignments[0].status, AssignmentStatus::Active);
    assert_eq!(assignments[0].target, RoleTarget::file("src/main.rs"));
    Ok(())
}

#[tokio::test]
async fn classification_matches_seeded_artefact_rule_with_path_language_and_symbol_facts()
-> anyhow::Result<()> {
    let (_temp, relational) = classifier_storage()?;

    let role = super::super::taxonomy::ArchitectureRole {
        repo_id: "repo-1".to_string(),
        role_id: super::super::taxonomy::stable_role_id("repo-1", "application", "use_case"),
        family: "application".to_string(),
        slug: "use_case".to_string(),
        display_name: "Use Case".to_string(),
        description: "Application use case role".to_string(),
        lifecycle: super::super::taxonomy::RoleLifecycle::Active,
        provenance: serde_json::json!({ "source": "test" }),
    };
    super::super::storage::upsert_classification_role(&relational, &role).await?;
    super::super::storage::upsert_detection_rule(
        &relational,
        &super::super::taxonomy::ArchitectureRoleDetectionRule {
            repo_id: "repo-1".to_string(),
            rule_id: super::super::taxonomy::rule_id("repo-1", &role.role_id, "seeded-use-case"),
            role_id: role.role_id.clone(),
            version: 1,
            lifecycle: super::super::taxonomy::RoleRuleLifecycle::Active,
            priority: 10,
            score: 1.0,
            min_positive_ratio: 1.0,
            candidate_selector: serde_json::json!({
                "path_prefixes": ["src/application/"],
                "path_suffixes": [".rs"],
                "path_contains": ["create_user"],
                "languages": ["rust"],
                "canonical_kinds": ["function"],
                "symbol_fqn_contains": ["_use_case"]
            }),
            positive_conditions: serde_json::json!([
                { "kind": "path_contains", "value": "create_user.rs" }
            ]),
            negative_conditions: serde_json::json!([]),
            provenance: serde_json::json!({ "source": "test" }),
        },
    )
    .await?;

    let files = vec![file_fixture("src/application/create_user.rs")];
    let artefacts = vec![artefact_fixture(
        "src/application/create_user.rs",
        "create_user_use_case",
    )];
    let dependency_edges = Vec::new();
    let current_state = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &dependency_edges);

    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 1,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from([
                    "src/application/create_user.rs".to_string(),
                ]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
        },
    )
    .await?;

    let assignments = super::super::storage::load_assignments_for_path(
        &relational,
        "repo-1",
        "src/application/create_user.rs",
    )
    .await?;
    let signal_rows = relational
        .query_rows(
            "SELECT target_kind, path, role_id, polarity, score
             FROM architecture_role_rule_signals_current
             WHERE repo_id = 'repo-1'",
        )
        .await?;

    assert_eq!(outcome.metrics.rules_loaded, 1);
    assert_eq!(outcome.metrics.signals_written, 1);
    assert_eq!(outcome.metrics.assignments_written, 1);
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].role_id, role.role_id);
    assert_eq!(assignments[0].status, AssignmentStatus::Active);
    assert_eq!(
        assignments[0].target.target_kind,
        super::super::taxonomy::TargetKind::Artefact
    );
    assert_eq!(signal_rows.len(), 1);
    assert_eq!(signal_rows[0]["target_kind"], "artefact");
    assert_eq!(signal_rows[0]["path"], "src/application/create_user.rs");
    assert_eq!(signal_rows[0]["role_id"], assignments[0].role_id);
    assert_eq!(signal_rows[0]["polarity"], "positive");
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
            assignment_id: super::super::taxonomy::assignment_id("repo-1", &role.role_id, &target),
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

    let current_state = empty_current_state();
    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 2,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/removed.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::from(["src/removed.rs".to_string()]),
            },
            files: &[],
        },
    )
    .await?;

    let assignments =
        super::super::storage::load_assignments_for_path(&relational, "repo-1", "src/removed.rs")
            .await?;
    assert_eq!(outcome.metrics.assignments_marked_stale, 1);
    assert_eq!(outcome.metrics.assignment_history_rows, 1);
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].status, AssignmentStatus::Stale);
    assert_eq!(assignments[0].generation_seq, 2);
    Ok(())
}

#[tokio::test]
async fn full_reconcile_marks_missing_role_assignments_stale() -> anyhow::Result<()> {
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
    let target = RoleTarget::file("src/deleted.rs");
    let assignment_id = super::super::taxonomy::assignment_id("repo-1", &role.role_id, &target);
    super::super::storage::upsert_assignment(
        &relational,
        &ArchitectureRoleAssignment {
            repo_id: "repo-1".to_string(),
            assignment_id: assignment_id.clone(),
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

    let files = vec![crate::models::CurrentCanonicalFileRecord {
        repo_id: "repo-1".to_string(),
        path: "src/live.rs".to_string(),
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

    let current_state = empty_current_state();
    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 2,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: true,
                affected_paths: std::collections::BTreeSet::new(),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
        },
    )
    .await?;

    let assignments =
        super::super::storage::load_assignments_for_path(&relational, "repo-1", "src/deleted.rs")
            .await?;
    let history = relational
        .query_rows(&format!(
            "SELECT change_kind
             FROM architecture_role_assignment_history
             WHERE repo_id = 'repo-1' AND assignment_id = '{}'",
            assignment_id
        ))
        .await?;
    assert_eq!(outcome.metrics.assignments_marked_stale, 1);
    assert_eq!(outcome.metrics.assignment_history_rows, 1);
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].status, AssignmentStatus::Stale);
    assert_eq!(assignments[0].generation_seq, 2);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["change_kind"], "path_removed");
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
            rule_id: super::super::taxonomy::rule_id("repo-1", &role.role_id, "main-entrypoint"),
            role_id: role.role_id.clone(),
            version: 1,
            lifecycle: super::super::taxonomy::RoleRuleLifecycle::Active,
            priority: 10,
            score: 0.6,
            min_positive_ratio: 1.0,
            candidate_selector: serde_json::json!({ "targetKinds": ["file"] }),
            positive_conditions: serde_json::json!([
                {
                    "kind": "path",
                    "key": "full",
                    "op": "suffix",
                    "value": "main.rs",
                    "score": 1.0
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

    let current_state = empty_current_state();
    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 1,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/main.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
        },
    )
    .await?;

    assert_eq!(outcome.metrics.assignments_written, 1);
    assert_eq!(outcome.metrics.adjudication_candidates, 1);
    assert_eq!(outcome.adjudication_requests.len(), 1);
    let request = &outcome.adjudication_requests[0];
    assert_eq!(request.reason, AdjudicationReason::LowConfidence);
    assert_eq!(request.deterministic_confidence, Some(0.6));
    assert_eq!(request.candidate_role_ids, vec![role.role_id.clone()]);
    assert_eq!(request.path.as_deref(), Some("src/main.rs"));
    assert_eq!(
        request
            .current_assignment
            .as_ref()
            .map(|assignment| assignment.role_id.as_str()),
        Some(role.role_id.as_str())
    );
    Ok(())
}

#[tokio::test]
async fn classification_does_not_queue_ordinary_unknown_source_file() -> anyhow::Result<()> {
    let (_temp, relational) = classifier_storage()?;
    let files = vec![file_fixture("src/ordinary.rs")];
    let current_state = empty_current_state();

    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 3,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/ordinary.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
        },
    )
    .await?;

    assert_eq!(outcome.metrics.unknown_targets_total, 1);
    assert_eq!(outcome.metrics.unknown_targets_rule_mining_eligible, 1);
    assert_eq!(outcome.metrics.unknown_targets_suppressed_non_role, 0);
    assert_eq!(outcome.metrics.unknown_targets_adjudication_escalated, 0);
    assert_eq!(outcome.metrics.unknown_adjudication_candidates, 0);
    assert_eq!(outcome.metrics.adjudication_candidates, 0);
    assert!(outcome.adjudication_requests.is_empty());
    Ok(())
}

#[tokio::test]
async fn classification_queues_high_impact_main_when_not_confidently_classified()
-> anyhow::Result<()> {
    let (_temp, relational) = classifier_storage()?;
    let files = vec![file_fixture("src/main.rs")];

    let current_state = empty_current_state();
    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 4,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/main.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
        },
    )
    .await?;

    assert_eq!(outcome.adjudication_requests.len(), 1);
    assert_eq!(
        outcome.adjudication_requests[0].reason,
        AdjudicationReason::HighImpact
    );
    Ok(())
}

#[tokio::test]
async fn classification_preserves_high_impact_main_file_adjudication() -> anyhow::Result<()> {
    let (_temp, relational) = classifier_storage()?;
    let files = vec![file_fixture("src/main.rs")];
    let current_state = empty_current_state();

    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 4,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/main.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
        },
    )
    .await?;

    assert_eq!(outcome.metrics.unknown_targets_total, 1);
    assert_eq!(outcome.metrics.unknown_targets_adjudication_escalated, 1);
    assert_eq!(outcome.metrics.high_impact_adjudication_candidates, 1);
    assert_eq!(outcome.metrics.adjudication_candidates, 1);
    assert_eq!(outcome.adjudication_requests.len(), 1);
    assert_eq!(
        outcome.adjudication_requests[0].reason,
        AdjudicationReason::HighImpact
    );
    assert_eq!(
        outcome.adjudication_requests[0].path.as_deref(),
        Some("src/main.rs")
    );
    Ok(())
}

#[tokio::test]
async fn classification_suppresses_file_like_main_artefact_duplicate() -> anyhow::Result<()> {
    let (_temp, relational) = classifier_storage()?;
    let files = vec![file_fixture("src/main.rs")];
    let file_like_artefact = crate::models::CurrentCanonicalArtefactRecord {
        repo_id: "repo-1".to_string(),
        path: "src/main.rs".to_string(),
        content_id: "content-main".to_string(),
        symbol_id: "symbol-main-file".to_string(),
        artefact_id: "artefact-main-file".to_string(),
        language: "rust".to_string(),
        extraction_fingerprint: "fingerprint-main-file".to_string(),
        canonical_kind: Some("file".to_string()),
        language_kind: Some("file".to_string()),
        symbol_fqn: Some("src/main.rs".to_string()),
        parent_symbol_id: None,
        parent_artefact_id: None,
        start_line: 1,
        end_line: 10,
        start_byte: 0,
        end_byte: 100,
        signature: None,
        modifiers: String::new(),
        docstring: None,
    };
    let artefacts = [file_like_artefact];
    let current_state = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &[]);

    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 4,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/main.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
        },
    )
    .await?;

    assert_eq!(outcome.metrics.unknown_targets_total, 2);
    assert_eq!(outcome.metrics.unknown_targets_adjudication_escalated, 1);
    assert_eq!(outcome.metrics.unknown_targets_suppressed_non_role, 1);
    assert_eq!(outcome.metrics.high_impact_adjudication_candidates, 1);
    assert_eq!(outcome.adjudication_requests.len(), 1);
    assert_eq!(
        outcome.adjudication_requests[0].target_kind.as_deref(),
        Some("file")
    );
    Ok(())
}

#[tokio::test]
async fn classification_preserves_main_function_artefact_high_impact() -> anyhow::Result<()> {
    let (_temp, relational) = classifier_storage()?;
    let files = vec![file_fixture("src/main.rs")];
    let main_artefact = crate::models::CurrentCanonicalArtefactRecord {
        repo_id: "repo-1".to_string(),
        path: "src/main.rs".to_string(),
        content_id: "content-main".to_string(),
        symbol_id: "symbol-main-function".to_string(),
        artefact_id: "artefact-main-function".to_string(),
        language: "rust".to_string(),
        extraction_fingerprint: "fingerprint-main-function".to_string(),
        canonical_kind: Some("function".to_string()),
        language_kind: Some("function_item".to_string()),
        symbol_fqn: Some("src/main.rs::main".to_string()),
        parent_symbol_id: None,
        parent_artefact_id: None,
        start_line: 1,
        end_line: 5,
        start_byte: 0,
        end_byte: 40,
        signature: Some("fn main()".to_string()),
        modifiers: String::new(),
        docstring: None,
    };
    let artefacts = [main_artefact];
    let current_state = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &[]);

    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 4,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/main.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
        },
    )
    .await?;

    assert_eq!(outcome.metrics.unknown_targets_total, 2);
    assert_eq!(outcome.metrics.unknown_targets_adjudication_escalated, 2);
    assert_eq!(outcome.metrics.high_impact_adjudication_candidates, 2);
    assert_eq!(outcome.adjudication_requests.len(), 2);
    assert!(outcome.adjudication_requests.iter().any(|request| {
        request.target_kind.as_deref() == Some("artefact")
            && request.reason == AdjudicationReason::HighImpact
    }));
    Ok(())
}

#[tokio::test]
async fn classification_suppresses_non_role_file_unknowns() -> anyhow::Result<()> {
    let (_temp, relational) = classifier_storage()?;
    let mut readme = file_fixture("README.md");
    readme.analysis_mode = "text".to_string();
    readme.file_role = "documentation".to_string();
    readme.language = "plaintext".to_string();
    readme.resolved_language = "plaintext".to_string();
    let files = vec![readme];
    let current_state = empty_current_state();

    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 5,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["README.md".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
        },
    )
    .await?;

    assert_eq!(outcome.metrics.unknown_targets_total, 1);
    assert_eq!(outcome.metrics.unknown_targets_suppressed_non_role, 1);
    assert_eq!(outcome.metrics.unknown_targets_rule_mining_eligible, 0);
    assert_eq!(outcome.metrics.unknown_targets_adjudication_escalated, 0);
    assert!(outcome.adjudication_requests.is_empty());
    Ok(())
}

#[tokio::test]
async fn role_metrics_count_deleted_facts_and_signals() -> anyhow::Result<()> {
    let (_temp, relational) = classifier_storage()?;
    let target = RoleTarget::file("src/orphan.rs");
    let path = target.path.clone();
    super::super::storage::replace_facts_for_paths(
        &relational,
        "repo-1",
        std::slice::from_ref(&path),
        &[ArchitectureArtefactFact {
            repo_id: "repo-1".to_string(),
            fact_id: super::super::taxonomy::fact_id(
                "repo-1",
                &target,
                "path",
                "full",
                "src/orphan.rs",
            ),
            target: target.clone(),
            language: Some("rust".to_string()),
            fact_kind: "path".to_string(),
            fact_key: "full".to_string(),
            fact_value: "src/orphan.rs".to_string(),
            source: "test".to_string(),
            confidence: 1.0,
            evidence: serde_json::json!([]),
            generation_seq: 1,
        }],
    )
    .await?;
    super::super::storage::replace_signals_for_paths(
        &relational,
        "repo-1",
        std::slice::from_ref(&path),
        &[ArchitectureRoleRuleSignal {
            repo_id: "repo-1".to_string(),
            signal_id: "signal-orphan".to_string(),
            rule_id: "rule-orphan".to_string(),
            rule_version: 1,
            role_id: "role-orphan".to_string(),
            target: target.clone(),
            polarity: RoleSignalPolarity::Positive,
            score: 0.5,
            evidence: serde_json::json!([]),
            generation_seq: 1,
        }],
    )
    .await?;

    let current_state = empty_current_state();
    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 2,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from([path]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &[],
        },
    )
    .await?;

    assert_eq!(outcome.metrics.facts_deleted, 1);
    assert_eq!(outcome.metrics.signals_deleted, 1);
    Ok(())
}

#[tokio::test]
async fn classification_returns_conflict_adjudication_request_for_top_conflicting_assignments()
-> anyhow::Result<()> {
    let temp = tempfile::TempDir::new()?;
    let sqlite_path = temp.path().join("devql.sqlite");
    let conn = rusqlite::Connection::open(&sqlite_path)?;
    conn.execute_batch(
        crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql(),
    )?;
    drop(conn);
    let relational = crate::host::devql::RelationalStorage::local_only(sqlite_path);

    let api_role = super::super::taxonomy::ArchitectureRole {
        repo_id: "repo-conflict".to_string(),
        role_id: super::super::taxonomy::stable_role_id("repo-conflict", "layer", "api"),
        family: "layer".to_string(),
        slug: "api".to_string(),
        display_name: "API".to_string(),
        description: "API layer".to_string(),
        lifecycle: super::super::taxonomy::RoleLifecycle::Active,
        provenance: serde_json::json!({ "source": "test" }),
    };
    let adapter_role = super::super::taxonomy::ArchitectureRole {
        repo_id: "repo-conflict".to_string(),
        role_id: super::super::taxonomy::stable_role_id("repo-conflict", "layer", "adapter"),
        family: "layer".to_string(),
        slug: "adapter".to_string(),
        display_name: "Adapter".to_string(),
        description: "Adapter layer".to_string(),
        lifecycle: super::super::taxonomy::RoleLifecycle::Active,
        provenance: serde_json::json!({ "source": "test" }),
    };
    super::super::storage::upsert_classification_role(&relational, &api_role).await?;
    super::super::storage::upsert_classification_role(&relational, &adapter_role).await?;
    for (role, score) in [(&api_role, 0.86), (&adapter_role, 0.84)] {
        super::super::storage::upsert_detection_rule(
            &relational,
            &super::super::taxonomy::ArchitectureRoleDetectionRule {
                repo_id: "repo-conflict".to_string(),
                rule_id: super::super::taxonomy::rule_id(
                    "repo-conflict",
                    &role.role_id,
                    "conflicting-api",
                ),
                role_id: role.role_id.clone(),
                version: 1,
                lifecycle: super::super::taxonomy::RoleRuleLifecycle::Active,
                priority: 10,
                score,
                min_positive_ratio: 1.0,
                candidate_selector: serde_json::json!({ "targetKinds": ["file"] }),
                positive_conditions: serde_json::json!([
                    {
                        "kind": "path",
                        "key": "full",
                        "op": "suffix",
                        "value": "api.rs",
                        "score": 1.0
                    }
                ]),
                negative_conditions: serde_json::json!([]),
                provenance: serde_json::json!({ "source": "test" }),
            },
        )
        .await?;
    }

    let files = vec![crate::models::CurrentCanonicalFileRecord {
        repo_id: "repo-conflict".to_string(),
        path: "src/api.rs".to_string(),
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

    let current_state = empty_current_state();
    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        &current_state,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-conflict",
            generation_seq: 4,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/api.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
        },
    )
    .await?;

    let assignments = super::super::storage::load_assignments_for_path(
        &relational,
        "repo-conflict",
        "src/api.rs",
    )
    .await?;
    assert_eq!(assignments.len(), 2);
    assert!(
        assignments
            .iter()
            .all(|assignment| assignment.status == AssignmentStatus::NeedsReview)
    );
    assert_eq!(outcome.metrics.adjudication_candidates, 2);
    assert_eq!(outcome.adjudication_requests.len(), 1);
    let request = &outcome.adjudication_requests[0];
    assert_eq!(request.reason, AdjudicationReason::Conflict);
    assert_eq!(request.deterministic_confidence, Some(0.86));
    assert_eq!(
        request.candidate_role_ids,
        vec![api_role.role_id.clone(), adapter_role.role_id.clone()]
    );
    assert_eq!(request.path.as_deref(), Some("src/api.rs"));
    Ok(())
}
