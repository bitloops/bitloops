use super::*;
use crate::capability_packs::architecture_graph::roles::storage::{
    load_current_assignment_by_id, upsert_assignment, upsert_classification_role,
    upsert_detection_rule,
};
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    ArchitectureRole, ArchitectureRoleAssignment, ArchitectureRoleDetectionRule,
    AssignmentPriority, AssignmentSource, AssignmentStatus, RoleLifecycle, RoleRuleLifecycle,
    RoleSignalPolarity, stable_role_id,
};
use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;
use tempfile::TempDir;

fn test_relational() -> Result<(TempDir, RelationalStorage)> {
    let temp = TempDir::new()?;
    let sqlite_path = temp.path().join("devql.sqlite");
    let conn = rusqlite::Connection::open(&sqlite_path)?;
    conn.execute_batch(architecture_graph_sqlite_schema_sql())?;
    drop(conn);
    Ok((temp, RelationalStorage::local_only(sqlite_path)))
}

fn role() -> ArchitectureRole {
    ArchitectureRole {
        repo_id: "repo-1".to_string(),
        role_id: stable_role_id("repo-1", "application", "entrypoint"),
        family: "application".to_string(),
        slug: "entrypoint".to_string(),
        display_name: "Entrypoint".to_string(),
        description: "Entrypoint role".to_string(),
        lifecycle: RoleLifecycle::Active,
        provenance: json!({"source": "test"}),
    }
}

#[tokio::test]
async fn db_taxonomy_reader_loads_active_roles() -> Result<()> {
    let (_temp, relational) = test_relational()?;
    upsert_classification_role(&relational, &role()).await?;

    let roles = DbRoleTaxonomyReader::new(&relational)
        .load_active_roles("repo-1", 1)
        .await?;

    assert_eq!(
        roles
            .iter()
            .map(|role| role.role_id.clone())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([role().role_id])
    );
    Ok(())
}

#[tokio::test]
async fn db_taxonomy_reader_ignores_non_active_role_rows() -> Result<()> {
    let (_temp, relational) = test_relational()?;
    let mut active = role();
    active.role_id = stable_role_id("repo-1", "application", "active_entrypoint");
    active.slug = "active_entrypoint".to_string();
    active.display_name = "Active Entrypoint".to_string();
    upsert_classification_role(&relational, &active).await?;

    relational
        .exec_serialized(
            "INSERT INTO architecture_roles (
                    repo_id, role_id, family, canonical_key, display_name, description,
                    lifecycle_status, provenance_json, evidence_json, metadata_json
                ) VALUES (
                    'repo-1', 'stable-role', 'application', 'stable_entrypoint',
                    'Stable Entrypoint', 'Bad legacy row', 'stable', '{}', '{}', '{}'
                );",
        )
        .await?;

    let roles = DbRoleTaxonomyReader::new(&relational)
        .load_active_roles("repo-1", 1)
        .await?;

    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].role_id, active.role_id);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn db_role_taxonomy_reader_loads_active_roles_without_blocking_bridge() -> Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = ArchitectureRole {
        repo_id: "repo-async-reader".to_string(),
        role_id: stable_role_id("repo-async-reader", "application", "entrypoint"),
        family: "application".to_string(),
        slug: "entrypoint".to_string(),
        display_name: "Entrypoint".to_string(),
        description: "Entrypoint role".to_string(),
        lifecycle: RoleLifecycle::Active,
        provenance: json!({"source": "test"}),
    };
    upsert_classification_role(&relational, &role).await?;

    let reader = DbRoleTaxonomyReader::new(&relational);
    let roles = reader.load_active_roles("repo-async-reader", 1).await?;

    assert_eq!(
        roles
            .iter()
            .map(|role| role.role_id.clone())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([role.role_id])
    );
    Ok(())
}

#[tokio::test]
async fn db_assignment_writer_persists_llm_assignment() -> Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role();
    upsert_classification_role(&relational, &role).await?;
    let request = request(vec![role.role_id.clone()]);
    let provenance = provenance();

    let outcome = DbRoleAssignmentWriter::new(&relational)
            .apply_llm_assignment(RoleAssignmentWriteEvent {
                request: request.clone(),
                result: crate::capability_packs::architecture_graph::roles::contracts::RoleAdjudicationResult {
                        outcome: AdjudicationOutcome::Assigned,
                        assignments: vec![crate::capability_packs::architecture_graph::roles::contracts::RoleAssignmentDecision {
                            role_id: role.role_id.clone(),
                            primary: true,
                            confidence: 0.91,
                            evidence: json!(["main.rs"]),
                        }],
                        confidence: 0.91,
                        evidence: json!(["signal"]),
                        reasoning_summary: "clear role".to_string(),
                        rule_suggestions: vec![],
                    },
                provenance,
            })
            .await?;

    let target = target_from_request(&request)?;
    let assignment_id = assignment_id("repo-1", &role.role_id, &target);
    let assignment = load_current_assignment_by_id(&relational, "repo-1", &assignment_id)
        .await?
        .expect("assignment");
    assert!(outcome.persisted);
    assert_eq!(assignment.source, AssignmentSource::Llm);
    assert_eq!(assignment.status, AssignmentStatus::Active);
    assert_eq!(assignment.confidence, 0.91);
    Ok(())
}

#[tokio::test]
async fn db_assignment_writer_marks_needs_review() -> Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role();
    upsert_classification_role(&relational, &role).await?;
    let request = request(vec![role.role_id.clone()]);

    let outcome = DbRoleAssignmentWriter::new(&relational)
        .mark_needs_review(
            &request,
            &RoleAdjudicationFailure {
                message: "invalid response".to_string(),
                retryable: false,
            },
            &provenance(),
        )
        .await?;

    let target = target_from_request(&request)?;
    let assignment_id = assignment_id("repo-1", &role.role_id, &target);
    let assignment = load_current_assignment_by_id(&relational, "repo-1", &assignment_id)
        .await?
        .expect("assignment");
    assert!(outcome.persisted);
    assert_eq!(assignment.status, AssignmentStatus::NeedsReview);
    assert_eq!(assignment.source, AssignmentSource::Llm);
    Ok(())
}

#[tokio::test]
async fn db_assignment_state_reader_finds_active_rule_assignment_for_request_target() -> Result<()>
{
    let (_temp, relational) = test_relational()?;
    let role = role();
    upsert_classification_role(&relational, &role).await?;
    let request = request(vec![role.role_id.clone()]);
    let target = target_from_request(&request)?;
    let deterministic = ArchitectureRoleAssignment {
        repo_id: "repo-1".to_string(),
        assignment_id: assignment_id("repo-1", &role.role_id, &target),
        role_id: role.role_id.clone(),
        target,
        priority: AssignmentPriority::Primary,
        status: AssignmentStatus::Active,
        source: AssignmentSource::Rule,
        confidence: 1.0,
        evidence: json!([{ "source": "rule_signal_aggregation" }]),
        provenance: json!({ "source": "deterministic_rules" }),
        classifier_version: "architecture_roles.deterministic.contract.v1".to_string(),
        rule_version: Some(1),
        generation_seq: 8,
    };
    upsert_assignment(&relational, &deterministic).await?;

    let current = DbRoleAssignmentWriter::new(&relational)
        .active_rule_assignment_for_request(&request)
        .await?
        .expect("active rule assignment");

    assert_eq!(current.assignment_id, deterministic.assignment_id);
    assert_eq!(current.role_id, role.role_id);
    assert_eq!(current.source, "rule");
    assert_eq!(current.status, "active");
    assert_eq!(current.generation_seq, 8);
    Ok(())
}

#[tokio::test]
async fn db_assignment_writer_skips_llm_assignment_when_active_rule_assignment_exists() -> Result<()>
{
    let (_temp, relational) = test_relational()?;
    let role = role();
    upsert_classification_role(&relational, &role).await?;
    let request = request(vec![role.role_id.clone()]);
    let target = target_from_request(&request)?;
    let deterministic = ArchitectureRoleAssignment {
        repo_id: "repo-1".to_string(),
        assignment_id: assignment_id("repo-1", &role.role_id, &target),
        role_id: role.role_id.clone(),
        target: target.clone(),
        priority: AssignmentPriority::Primary,
        status: AssignmentStatus::Active,
        source: AssignmentSource::Rule,
        confidence: 1.0,
        evidence: json!([{ "source": "rule_signal_aggregation" }]),
        provenance: json!({ "source": "deterministic_rules" }),
        classifier_version: "architecture_roles.deterministic.contract.v1".to_string(),
        rule_version: Some(1),
        generation_seq: 8,
    };
    upsert_assignment(&relational, &deterministic).await?;

    let outcome = DbRoleAssignmentWriter::new(&relational)
            .apply_llm_assignment(RoleAssignmentWriteEvent {
                request: request.clone(),
                result: crate::capability_packs::architecture_graph::roles::contracts::RoleAdjudicationResult {
                    outcome: AdjudicationOutcome::Assigned,
                    assignments: vec![crate::capability_packs::architecture_graph::roles::contracts::RoleAssignmentDecision {
                        role_id: role.role_id.clone(),
                        primary: true,
                        confidence: 0.91,
                        evidence: json!(["main.rs"]),
                    }],
                    confidence: 0.91,
                    evidence: json!(["signal"]),
                    reasoning_summary: "clear role".to_string(),
                    rule_suggestions: vec![],
                },
                provenance: provenance(),
            })
            .await?;

    let loaded = load_current_assignment_by_id(&relational, "repo-1", &deterministic.assignment_id)
        .await?
        .expect("assignment");
    assert!(!outcome.persisted);
    assert_eq!(outcome.source, "skipped_deterministic_assignment");
    assert_eq!(loaded.source, AssignmentSource::Rule);
    assert_eq!(loaded.confidence, 1.0);
    assert_eq!(
        loaded.classifier_version,
        "architecture_roles.deterministic.contract.v1"
    );
    Ok(())
}

#[tokio::test]
async fn db_assignment_writer_skips_needs_review_when_active_rule_assignment_exists() -> Result<()>
{
    let (_temp, relational) = test_relational()?;
    let role = role();
    upsert_classification_role(&relational, &role).await?;
    let request = request(vec![role.role_id.clone()]);
    let target = target_from_request(&request)?;
    let deterministic = ArchitectureRoleAssignment {
        repo_id: "repo-1".to_string(),
        assignment_id: assignment_id("repo-1", &role.role_id, &target),
        role_id: role.role_id.clone(),
        target,
        priority: AssignmentPriority::Primary,
        status: AssignmentStatus::Active,
        source: AssignmentSource::Rule,
        confidence: 1.0,
        evidence: json!([{ "source": "rule_signal_aggregation" }]),
        provenance: json!({ "source": "deterministic_rules" }),
        classifier_version: "architecture_roles.deterministic.contract.v1".to_string(),
        rule_version: Some(1),
        generation_seq: 8,
    };
    upsert_assignment(&relational, &deterministic).await?;

    let outcome = DbRoleAssignmentWriter::new(&relational)
        .mark_needs_review(
            &request,
            &RoleAdjudicationFailure {
                message: "invalid response".to_string(),
                retryable: false,
            },
            &provenance(),
        )
        .await?;

    let loaded = load_current_assignment_by_id(&relational, "repo-1", &deterministic.assignment_id)
        .await?
        .expect("assignment");
    assert!(!outcome.persisted);
    assert_eq!(outcome.source, "skipped_deterministic_assignment");
    assert_eq!(loaded.status, AssignmentStatus::Active);
    assert_eq!(loaded.source, AssignmentSource::Rule);
    Ok(())
}

#[tokio::test]
async fn stale_llm_assignment_from_older_generation_does_not_replace_current_rule_assignment()
-> Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role();
    upsert_classification_role(&relational, &role).await?;
    let mut stale_request = request(vec![role.role_id.clone()]);
    stale_request.generation = 2;
    let target = target_from_request(&stale_request)?;
    let deterministic = ArchitectureRoleAssignment {
        repo_id: "repo-1".to_string(),
        assignment_id: assignment_id("repo-1", &role.role_id, &target),
        role_id: role.role_id.clone(),
        target,
        priority: AssignmentPriority::Primary,
        status: AssignmentStatus::Active,
        source: AssignmentSource::Rule,
        confidence: 1.0,
        evidence: json!([{ "source": "rule_signal_aggregation" }]),
        provenance: json!({ "source": "deterministic_rules" }),
        classifier_version: "architecture_roles.deterministic.contract.v1".to_string(),
        rule_version: Some(1),
        generation_seq: 3,
    };
    upsert_assignment(&relational, &deterministic).await?;

    let outcome = DbRoleAssignmentWriter::new(&relational)
            .apply_llm_assignment(RoleAssignmentWriteEvent {
                request: stale_request,
                result: crate::capability_packs::architecture_graph::roles::contracts::RoleAdjudicationResult {
                    outcome: AdjudicationOutcome::Assigned,
                    assignments: vec![crate::capability_packs::architecture_graph::roles::contracts::RoleAssignmentDecision {
                        role_id: role.role_id.clone(),
                        primary: true,
                        confidence: 0.91,
                        evidence: json!(["main.rs"]),
                    }],
                    confidence: 0.91,
                    evidence: json!(["signal"]),
                    reasoning_summary: "clear role".to_string(),
                    rule_suggestions: vec![],
                },
                provenance: provenance(),
            })
            .await?;

    let loaded = load_current_assignment_by_id(&relational, "repo-1", &deterministic.assignment_id)
        .await?
        .expect("assignment");
    assert!(!outcome.persisted);
    assert_eq!(loaded.source, AssignmentSource::Rule);
    assert_eq!(loaded.generation_seq, 3);
    assert_eq!(
        loaded.classifier_version,
        "architecture_roles.deterministic.contract.v1"
    );
    Ok(())
}

#[tokio::test]
async fn active_rule_assignment_for_different_exact_target_does_not_skip_llm_write() -> Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role();
    upsert_classification_role(&relational, &role).await?;
    let request = request(vec![role.role_id.clone()]);
    let different_target = RoleTarget::file("src/main.rs");
    let deterministic = ArchitectureRoleAssignment {
        repo_id: "repo-1".to_string(),
        assignment_id: assignment_id("repo-1", &role.role_id, &different_target),
        role_id: role.role_id.clone(),
        target: different_target,
        priority: AssignmentPriority::Primary,
        status: AssignmentStatus::Active,
        source: AssignmentSource::Rule,
        confidence: 1.0,
        evidence: json!([{ "source": "rule_signal_aggregation" }]),
        provenance: json!({ "source": "deterministic_rules" }),
        classifier_version: "architecture_roles.deterministic.contract.v1".to_string(),
        rule_version: Some(1),
        generation_seq: 8,
    };
    upsert_assignment(&relational, &deterministic).await?;

    let outcome = DbRoleAssignmentWriter::new(&relational)
            .apply_llm_assignment(RoleAssignmentWriteEvent {
                request: request.clone(),
                result: crate::capability_packs::architecture_graph::roles::contracts::RoleAdjudicationResult {
                    outcome: AdjudicationOutcome::Assigned,
                    assignments: vec![crate::capability_packs::architecture_graph::roles::contracts::RoleAssignmentDecision {
                        role_id: role.role_id.clone(),
                        primary: true,
                        confidence: 0.91,
                        evidence: json!(["main.rs"]),
                    }],
                    confidence: 0.91,
                    evidence: json!(["signal"]),
                    reasoning_summary: "clear role".to_string(),
                    rule_suggestions: vec![],
                },
                provenance: provenance(),
            })
            .await?;

    let request_target = target_from_request(&request)?;
    let request_assignment_id = assignment_id("repo-1", &role.role_id, &request_target);
    let loaded = load_current_assignment_by_id(&relational, "repo-1", &request_assignment_id)
        .await?
        .expect("assignment");
    assert!(outcome.persisted);
    assert_eq!(outcome.source, "db");
    assert_eq!(loaded.source, AssignmentSource::Llm);
    assert_eq!(loaded.target, request_target);
    Ok(())
}

#[tokio::test]
async fn db_facts_reader_loads_facts_and_rule_signals() -> Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role();
    upsert_classification_role(&relational, &role).await?;
    let rule = ArchitectureRoleDetectionRule {
        repo_id: "repo-1".to_string(),
        rule_id: "rule-1".to_string(),
        role_id: role.role_id,
        version: 1,
        lifecycle: RoleRuleLifecycle::Active,
        priority: 10,
        score: 1.0,
        candidate_selector: json!({"targetKinds": ["file"]}),
        positive_conditions: json!([]),
        negative_conditions: json!([]),
        provenance: json!({"source": "test"}),
    };
    upsert_detection_rule(&relational, &rule).await?;
    let target = RoleTarget::file("src/main.rs");
    let fact =
        crate::capability_packs::architecture_graph::roles::taxonomy::ArchitectureArtefactFact {
            repo_id: "repo-1".to_string(),
            fact_id: "fact-1".to_string(),
            target: target.clone(),
            language: Some("rust".to_string()),
            fact_kind: "path".to_string(),
            fact_key: "segment".to_string(),
            fact_value: "main.rs".to_string(),
            source: "test".to_string(),
            confidence: 1.0,
            evidence: json!([]),
            generation_seq: 1,
        };
    let signal =
        crate::capability_packs::architecture_graph::roles::taxonomy::ArchitectureRoleRuleSignal {
            repo_id: "repo-1".to_string(),
            signal_id: "signal-1".to_string(),
            rule_id: "rule-1".to_string(),
            rule_version: 1,
            role_id: rule.role_id.clone(),
            target,
            polarity: RoleSignalPolarity::Positive,
            score: 0.8,
            evidence: json!([]),
            generation_seq: 1,
        };
    super::super::facts::replace_facts_for_paths(
        &relational,
        "repo-1",
        &[String::from("src/main.rs")],
        &[fact],
    )
    .await?;
    super::super::signals::replace_signals_for_paths(
        &relational,
        "repo-1",
        &[String::from("src/main.rs")],
        &[signal],
    )
    .await?;

    let bundle = DbRoleFactsReader::new(&relational)
        .load_facts(&request(Vec::new()))
        .await?;

    assert_eq!(bundle.facts.len(), 1);
    assert_eq!(bundle.rule_signals.len(), 1);
    assert_eq!(bundle.rule_signals[0].rule_id, "rule-1");
    Ok(())
}

fn request(candidate_role_ids: Vec<String>) -> RoleAdjudicationRequest {
    RoleAdjudicationRequest {
            repo_id: "repo-1".to_string(),
            generation: 7,
            target_kind: Some("artefact".to_string()),
            artefact_id: Some("artefact-1".to_string()),
            symbol_id: Some("symbol-1".to_string()),
            path: Some("src/main.rs".to_string()),
            language: Some("rust".to_string()),
            canonical_kind: Some("function".to_string()),
            reason: crate::capability_packs::architecture_graph::roles::contracts::AdjudicationReason::LowConfidence,
            deterministic_confidence: Some(0.5),
            candidate_role_ids,
            current_assignment: None,
        }
}

fn provenance() -> RoleAdjudicationProvenance {
    RoleAdjudicationProvenance {
            source: "llm".to_string(),
            model_descriptor: "fake:model".to_string(),
            slot_name: "role_adjudication".to_string(),
            packet_sha256: "packet".to_string(),
            adjudication_reason: crate::capability_packs::architecture_graph::roles::contracts::AdjudicationReason::LowConfidence,
            adjudicated_at_unix: 1,
        }
}
