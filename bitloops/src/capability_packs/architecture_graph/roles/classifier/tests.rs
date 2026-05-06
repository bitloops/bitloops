use super::super::taxonomy::{
    ArchitectureArtefactFact, ArchitectureRoleRuleSignal, AssignmentPriority, AssignmentStatus,
    RoleSignalPolarity, RoleTarget,
};
use super::*;

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
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/main.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
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

    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 2,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/removed.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::from(["src/removed.rs".to_string()]),
            },
            files: &[],
            artefacts: &[],
            dependency_edges: &[],
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

    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 2,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: true,
                affected_paths: std::collections::BTreeSet::new(),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
            artefacts: &[],
            dependency_edges: &[],
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
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/main.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
            artefacts: &[],
            dependency_edges: &[],
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
async fn classification_queues_unknown_when_changed_target_has_no_assignment() -> anyhow::Result<()>
{
    let (_temp, relational) = classifier_storage()?;
    let files = vec![file_fixture("src/unknown.rs")];

    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 3,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/unknown.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
            artefacts: &[],
            dependency_edges: &[],
        },
    )
    .await?;

    assert_eq!(outcome.metrics.adjudication_candidates, 1);
    assert_eq!(outcome.adjudication_requests.len(), 1);
    assert_eq!(
        outcome.adjudication_requests[0].reason,
        AdjudicationReason::Unknown
    );
    assert_eq!(
        outcome.adjudication_requests[0].path.as_deref(),
        Some("src/unknown.rs")
    );
    Ok(())
}

#[tokio::test]
async fn classification_queues_high_impact_main_when_not_confidently_classified()
-> anyhow::Result<()> {
    let (_temp, relational) = classifier_storage()?;
    let files = vec![file_fixture("src/main.rs")];

    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 4,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/main.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
            artefacts: &[],
            dependency_edges: &[],
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

    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-1",
            generation_seq: 2,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from([path]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &[],
            artefacts: &[],
            dependency_edges: &[],
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
                score: 1.0,
                candidate_selector: serde_json::json!({ "targetKinds": ["file"] }),
                positive_conditions: serde_json::json!([
                    {
                        "kind": "path",
                        "key": "full",
                        "op": "suffix",
                        "value": "api.rs",
                        "score": score
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

    let outcome = classify_architecture_roles_for_current_state(
        &relational,
        ArchitectureRoleClassificationInput {
            repo_id: "repo-conflict",
            generation_seq: 4,
            scope: ArchitectureRoleClassificationScope {
                full_reconcile: false,
                affected_paths: std::collections::BTreeSet::from(["src/api.rs".to_string()]),
                removed_paths: std::collections::BTreeSet::new(),
            },
            files: &files,
            artefacts: &[],
            dependency_edges: &[],
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
