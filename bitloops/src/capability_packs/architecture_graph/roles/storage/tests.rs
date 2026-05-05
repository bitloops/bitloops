use super::rows::sql_text;
use super::*;
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    ArchitectureArtefactFact, ArchitectureRole, ArchitectureRoleAssignment,
    ArchitectureRoleChangeProposal, ArchitectureRoleDetectionRule, ArchitectureRoleRuleSignal,
    AssignmentPriority, AssignmentSource, AssignmentStatus, ProposalStatus, RoleLifecycle,
    RoleRuleLifecycle, RoleSignalPolarity, RoleTarget, proposal_id, stable_role_id,
};
use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;
use crate::host::devql::RelationalStorage;
use anyhow::Result;
use serde_json::Value;
use tempfile::TempDir;

fn test_relational() -> anyhow::Result<(TempDir, RelationalStorage)> {
    let temp = TempDir::new()?;
    let sqlite_path = temp.path().join("devql.sqlite");
    let conn = rusqlite::Connection::open(&sqlite_path)?;
    conn.execute_batch(architecture_graph_sqlite_schema_sql())?;
    drop(conn);
    Ok((temp, RelationalStorage::local_only(sqlite_path)))
}

fn role_fixture(repo_id: &str, family: &str, slug: &str, display_name: &str) -> ArchitectureRole {
    ArchitectureRole {
        repo_id: repo_id.to_string(),
        role_id: stable_role_id(repo_id, family, slug),
        family: family.to_string(),
        slug: slug.to_string(),
        display_name: display_name.to_string(),
        description: "Test role".to_string(),
        lifecycle: RoleLifecycle::Active,
        provenance: serde_json::json!({"source": "test"}),
    }
}

fn assignment_fixture(role: &ArchitectureRole, path: &str) -> ArchitectureRoleAssignment {
    let target = RoleTarget::artefact("art-1", "sym-1", path);
    ArchitectureRoleAssignment {
        repo_id: role.repo_id.clone(),
        assignment_id: super::super::taxonomy::assignment_id(&role.repo_id, &role.role_id, &target),
        role_id: role.role_id.clone(),
        target,
        priority: AssignmentPriority::Primary,
        status: AssignmentStatus::Active,
        source: AssignmentSource::Rule,
        confidence: 0.91,
        evidence: serde_json::json!([{ "fact": "path:suffix:main.rs" }]),
        provenance: serde_json::json!({ "classifier": "test" }),
        classifier_version: "test.classifier.v1".to_string(),
        rule_version: Some(3),
        generation_seq: 7,
    }
}

async fn assignment_history_count(
    relational: &RelationalStorage,
    repo_id: &str,
    assignment_id: &str,
) -> Result<usize> {
    let sql = format!(
        "SELECT COUNT(*) AS count
             FROM architecture_role_assignment_history
             WHERE repo_id = {} AND assignment_id = {}",
        sql_text(repo_id),
        sql_text(assignment_id)
    );
    let rows = relational.query_rows(&sql).await?;
    let count = rows
        .first()
        .and_then(|row| row.get("count"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Ok(usize::try_from(count).unwrap_or(0))
}

#[tokio::test]
async fn upsert_and_load_role_preserves_stable_id_and_display_name() -> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role_fixture("repo-1", "application", "entrypoint", "Entrypoint");

    upsert_classification_role(&relational, &role).await?;

    let roles = load_roles(&relational, "repo-1").await?;
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].role_id, role.role_id);
    assert_eq!(roles[0].display_name, "Entrypoint");
    assert_eq!(roles[0].lifecycle, RoleLifecycle::Active);
    Ok(())
}

#[tokio::test]
async fn rename_role_keeps_stable_role_id() -> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role_fixture("repo-1", "application", "entrypoint", "Entrypoint");
    upsert_classification_role(&relational, &role).await?;

    rename_role(
        &relational,
        "repo-1",
        &role.role_id,
        "Process Entrypoint",
        &serde_json::json!({"source": "rename-test"}),
    )
    .await?;

    let roles = load_roles(&relational, "repo-1").await?;
    assert_eq!(roles[0].role_id, role.role_id);
    assert_eq!(roles[0].display_name, "Process Entrypoint");
    Ok(())
}

#[tokio::test]
async fn upsert_role_does_not_mutate_identity_fields() -> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let mut role = role_fixture("repo-1", "runtime", "consumer", "Consumer");
    upsert_classification_role(&relational, &role).await?;

    role.family = "sync".to_string();
    role.slug = "canonical_consumer".to_string();
    role.display_name = "Renamed Consumer".to_string();
    upsert_classification_role(&relational, &role).await?;

    let roles = load_roles(&relational, "repo-1").await?;
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].family, "runtime");
    assert_eq!(roles[0].slug, "consumer");
    assert_eq!(roles[0].display_name, "Renamed Consumer");
    Ok(())
}

#[tokio::test]
async fn replace_facts_for_paths_removes_stale_facts() -> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let target = RoleTarget::file("src/main.rs");
    let first = ArchitectureArtefactFact {
        repo_id: "repo-1".to_string(),
        fact_id: super::super::taxonomy::fact_id("repo-1", &target, "path", "suffix", ".rs"),
        target: target.clone(),
        language: Some("rust".to_string()),
        fact_kind: "path".to_string(),
        fact_key: "suffix".to_string(),
        fact_value: ".rs".to_string(),
        source: "canonical_file".to_string(),
        confidence: 1.0,
        evidence: serde_json::json!([{ "path": "src/main.rs" }]),
        generation_seq: 1,
    };
    replace_facts_for_paths(
        &relational,
        "repo-1",
        std::slice::from_ref(&target.path),
        &[first],
    )
    .await?;

    let second = ArchitectureArtefactFact {
        fact_id: super::super::taxonomy::fact_id("repo-1", &target, "language", "resolved", "rust"),
        fact_kind: "language".to_string(),
        fact_key: "resolved".to_string(),
        fact_value: "rust".to_string(),
        generation_seq: 2,
        ..load_facts_for_paths(&relational, "repo-1", std::slice::from_ref(&target.path)).await?[0]
            .clone()
    };
    replace_facts_for_paths(
        &relational,
        "repo-1",
        std::slice::from_ref(&target.path),
        &[second],
    )
    .await?;

    let loaded = load_facts_for_paths(&relational, "repo-1", &[target.path]).await?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].fact_kind, "language");
    assert_eq!(loaded[0].generation_seq, 2);
    Ok(())
}

#[tokio::test]
async fn load_active_detection_rules_ignores_draft_rules() -> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role_fixture("repo-1", "runtime", "consumer", "Consumer");
    upsert_classification_role(&relational, &role).await?;

    let active = ArchitectureRoleDetectionRule {
        repo_id: "repo-1".to_string(),
        rule_id: super::super::taxonomy::rule_id("repo-1", &role.role_id, "consumer-rule"),
        role_id: role.role_id.clone(),
        version: 1,
        lifecycle: RoleRuleLifecycle::Active,
        priority: 10,
        score: 0.8,
        candidate_selector: serde_json::json!({ "targetKinds": ["artefact"] }),
        positive_conditions: serde_json::json!([{ "kind": "path", "key": "segment", "op": "eq", "value": "consumer", "score": 0.3 }]),
        negative_conditions: serde_json::json!([]),
        provenance: serde_json::json!({ "source": "test" }),
    };
    let mut draft = active.clone();
    draft.rule_id = super::super::taxonomy::rule_id("repo-1", &role.role_id, "draft-rule");
    draft.lifecycle = RoleRuleLifecycle::Draft;

    upsert_detection_rule(&relational, &active).await?;
    upsert_detection_rule(&relational, &draft).await?;

    let loaded = load_active_detection_rules(&relational, "repo-1").await?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].rule_id, active.rule_id);
    Ok(())
}

#[tokio::test]
async fn load_active_detection_rules_returns_latest_active_version() -> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role_fixture("repo-1", "runtime", "consumer", "Consumer");
    upsert_classification_role(&relational, &role).await?;

    let version_one = ArchitectureRoleDetectionRule {
        repo_id: "repo-1".to_string(),
        rule_id: super::super::taxonomy::rule_id("repo-1", &role.role_id, "consumer-rule"),
        role_id: role.role_id.clone(),
        version: 1,
        lifecycle: RoleRuleLifecycle::Active,
        priority: 10,
        score: 0.5,
        candidate_selector: serde_json::json!({ "targetKinds": ["file"] }),
        positive_conditions: serde_json::json!([{ "kind": "path", "key": "segment", "op": "eq", "value": "consumer", "score": 0.5 }]),
        negative_conditions: serde_json::json!([]),
        provenance: serde_json::json!({ "source": "test" }),
    };
    let mut version_two = version_one.clone();
    version_two.version = 2;
    version_two.score = 0.8;

    upsert_detection_rule(&relational, &version_one).await?;
    upsert_detection_rule(&relational, &version_two).await?;

    let loaded = load_active_detection_rules(&relational, "repo-1").await?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].rule_id, version_one.rule_id);
    assert_eq!(loaded[0].version, 2);
    assert_eq!(loaded[0].score, 0.8);
    Ok(())
}

#[tokio::test]
async fn replace_signals_for_paths_removes_stale_signals() -> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let target = RoleTarget::file("src/main.rs");
    let path = target.path.clone();
    let first = ArchitectureRoleRuleSignal {
        repo_id: "repo-1".to_string(),
        signal_id: "signal-1".to_string(),
        rule_id: "rule-1".to_string(),
        rule_version: 1,
        role_id: "role-1".to_string(),
        target: target.clone(),
        polarity: RoleSignalPolarity::Positive,
        score: 0.4,
        evidence: serde_json::json!([{ "phase": "first" }]),
        generation_seq: 1,
    };
    let second = ArchitectureRoleRuleSignal {
        signal_id: "signal-2".to_string(),
        rule_id: "rule-2".to_string(),
        role_id: "role-2".to_string(),
        score: 0.6,
        evidence: serde_json::json!([{ "phase": "second" }]),
        ..first.clone()
    };
    replace_signals_for_paths(
        &relational,
        "repo-1",
        std::slice::from_ref(&path),
        &[first, second],
    )
    .await?;

    let replacement = ArchitectureRoleRuleSignal {
        signal_id: "signal-3".to_string(),
        rule_id: "rule-3".to_string(),
        role_id: "role-3".to_string(),
        score: 0.8,
        evidence: serde_json::json!([{ "phase": "replacement" }]),
        generation_seq: 3,
        ..ArchitectureRoleRuleSignal {
            repo_id: "repo-1".to_string(),
            signal_id: "unused".to_string(),
            rule_id: "unused".to_string(),
            rule_version: 1,
            role_id: "unused".to_string(),
            target: target.clone(),
            polarity: RoleSignalPolarity::Positive,
            score: 0.0,
            evidence: serde_json::json!([]),
            generation_seq: 0,
        }
    };
    replace_signals_for_paths(
        &relational,
        "repo-1",
        std::slice::from_ref(&path),
        &[replacement],
    )
    .await?;

    let rows = relational
        .query_rows(
            "SELECT signal_id, generation_seq, evidence_json
                 FROM architecture_role_rule_signals_current
                 WHERE repo_id = 'repo-1' AND path = 'src/main.rs'
                 ORDER BY signal_id ASC",
        )
        .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["signal_id"], "signal-3");
    assert_eq!(rows[0]["generation_seq"], 3);
    let evidence: serde_json::Value = serde_json::from_str(
        rows[0]["evidence_json"]
            .as_str()
            .expect("evidence JSON should be stored as text"),
    )?;
    assert_eq!(evidence[0]["phase"], "replacement");
    Ok(())
}

#[tokio::test]
async fn assignment_persistence_preserves_contract_fields() -> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role_fixture("repo-1", "application", "entrypoint", "Entrypoint");
    upsert_classification_role(&relational, &role).await?;
    let assignment = assignment_fixture(&role, "src/main.rs");

    upsert_assignment(&relational, &assignment).await?;

    let loaded = load_assignments_for_path(&relational, "repo-1", "src/main.rs").await?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].role_id, role.role_id);
    assert_eq!(loaded[0].source, AssignmentSource::Rule);
    assert_eq!(loaded[0].confidence, 0.91);
    assert_eq!(loaded[0].classifier_version, "test.classifier.v1");
    assert_eq!(loaded[0].rule_version, Some(3));
    assert_eq!(loaded[0].generation_seq, 7);
    assert_eq!(loaded[0].evidence[0]["fact"], "path:suffix:main.rs");
    Ok(())
}

#[tokio::test]
async fn replace_assignments_for_paths_removes_stale_assignments() -> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role_fixture("repo-1", "application", "entrypoint", "Entrypoint");
    let replacement_role = role_fixture("repo-1", "runtime", "consumer", "Consumer");
    upsert_classification_role(&relational, &role).await?;
    upsert_classification_role(&relational, &replacement_role).await?;

    let stale = assignment_fixture(&role, "src/main.rs");
    upsert_assignment(&relational, &stale).await?;

    let mut replacement = assignment_fixture(&replacement_role, "src/main.rs");
    replacement.evidence = serde_json::json!([{ "phase": "replacement" }]);
    replacement.generation_seq = 8;
    let count = replace_assignments_for_paths(
        &relational,
        "repo-1",
        &[String::from("src/main.rs")],
        &[replacement.clone()],
    )
    .await?;

    assert_eq!(count, 1);
    let loaded = load_assignments_for_path(&relational, "repo-1", "src/main.rs").await?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].assignment_id, replacement.assignment_id);
    assert_eq!(loaded[0].role_id, replacement_role.role_id);
    assert_eq!(loaded[0].evidence[0]["phase"], "replacement");
    Ok(())
}

#[tokio::test]
async fn mark_assignments_for_paths_stale_preserves_assignment_with_stale_status()
-> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role_fixture("repo-1", "application", "entrypoint", "Entrypoint");
    upsert_classification_role(&relational, &role).await?;
    let assignment = assignment_fixture(&role, "src/main.rs");
    upsert_assignment(&relational, &assignment).await?;

    let marked =
        mark_assignments_for_paths_stale(&relational, "repo-1", &[String::from("src/main.rs")], 11)
            .await?;

    assert_eq!(marked, 1);
    let loaded = load_assignments_for_path(&relational, "repo-1", "src/main.rs").await?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].assignment_id, assignment.assignment_id);
    assert_eq!(loaded[0].status, AssignmentStatus::Stale);
    assert_eq!(loaded[0].generation_seq, 11);
    assert_eq!(
        assignment_history_count(&relational, "repo-1", &assignment.assignment_id).await?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn removing_role_marks_active_assignments_needs_review() -> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role_fixture("repo-1", "application", "entrypoint", "Entrypoint");
    upsert_classification_role(&relational, &role).await?;
    upsert_assignment(&relational, &assignment_fixture(&role, "src/main.rs")).await?;

    retire_role_and_mark_assignments(
        &relational,
        "repo-1",
        &role.role_id,
        RoleLifecycle::Removed,
        AssignmentStatus::NeedsReview,
    )
    .await?;

    let roles = load_roles(&relational, "repo-1").await?;
    assert_eq!(roles[0].lifecycle, RoleLifecycle::Removed);

    let assignments = load_assignments_for_path(&relational, "repo-1", "src/main.rs").await?;
    assert_eq!(assignments[0].status, AssignmentStatus::NeedsReview);
    assert_eq!(assignments[0].role_id, role.role_id);
    assert_eq!(
        assignment_history_count(&relational, "repo-1", &assignments[0].assignment_id).await?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn assignment_history_records_meaningful_status_change() -> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let role = role_fixture("repo-1", "application", "entrypoint", "Entrypoint");
    upsert_classification_role(&relational, &role).await?;
    let previous = assignment_fixture(&role, "src/main.rs");
    upsert_assignment(&relational, &previous).await?;

    let mut next = previous.clone();
    next.status = AssignmentStatus::NeedsReview;
    next.confidence = 0.62;
    next.generation_seq = 8;
    record_assignment_history(&relational, Some(&previous), &next, "status_changed").await?;

    assert_eq!(
        assignment_history_count(&relational, "repo-1", &next.assignment_id).await?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn change_proposal_persists_payload_and_preview() -> anyhow::Result<()> {
    let (_temp, relational) = test_relational()?;
    let payload = serde_json::json!({
        "roleId": "role-1",
        "displayName": "New Name"
    });
    let proposal = ArchitectureRoleChangeProposal {
        repo_id: "repo-1".to_string(),
        proposal_id: proposal_id("repo-1", "role_rename", &payload),
        proposal_kind: "role_rename".to_string(),
        status: ProposalStatus::Previewed,
        payload,
        impact_preview: serde_json::json!({ "affectedAssignments": 0 }),
        provenance: serde_json::json!({ "source": "test" }),
    };

    insert_change_proposal(&relational, &proposal).await?;

    let rows = relational
            .query_rows(
                "SELECT proposal_kind, status, payload_json, impact_preview_json FROM architecture_role_change_proposals",
            )
            .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["proposal_kind"], "role_rename");
    assert_eq!(rows[0]["status"], "previewed");
    let payload: serde_json::Value = serde_json::from_str(
        rows[0]["payload_json"]
            .as_str()
            .expect("payload JSON should be stored as text"),
    )?;
    let impact_preview: serde_json::Value = serde_json::from_str(
        rows[0]["impact_preview_json"]
            .as_str()
            .expect("impact preview JSON should be stored as text"),
    )?;
    assert_eq!(payload["displayName"], "New Name");
    assert_eq!(impact_preview["affectedAssignments"], 0);
    Ok(())
}
