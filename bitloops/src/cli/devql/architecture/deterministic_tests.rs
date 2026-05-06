use super::*;
use crate::capability_packs::architecture_graph::roles::fact_extraction::{
    ArchitectureRoleFactExtractionInput, extract_architecture_role_facts,
};
use crate::capability_packs::architecture_graph::roles::rules::{
    compile_detection_rules, evaluate_rules_over_facts,
};
use crate::capability_packs::architecture_graph::roles::storage::{
    ArchitectureRoleAliasRecord, ArchitectureRoleRecord, deterministic_alias_id,
    deterministic_role_id, list_roles, load_active_detection_rules, load_role_by_id,
    load_role_rules, normalize_role_alias, update_role_rule_lifecycle, upsert_assignment,
    upsert_role,
};
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    ArchitectureRoleAssignment, ArchitectureRoleReconcileMetrics, AssignmentPriority,
    AssignmentSource, AssignmentStatus, RoleRuleCandidateSelector, RoleRuleCondition,
    RoleRuleScore, RoleTarget, SeededArchitectureRole, SeededArchitectureRuleCandidate,
    SeededArchitectureTaxonomy, assignment_id,
};
use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;
use crate::host::devql::RelationalStorage;
use crate::host::runtime_store::{WorkplaneJobRecord, WorkplaneJobStatus};
use crate::models::CurrentCanonicalFileRecord;
use std::collections::BTreeSet;

async fn relational() -> Result<RelationalStorage> {
    let temp = tempfile::tempdir()?;
    let sqlite_path = temp.path().join("roles.sqlite");
    rusqlite::Connection::open(&sqlite_path)?;
    let relational = RelationalStorage::local_only(sqlite_path);
    relational
        .exec(architecture_graph_sqlite_schema_sql())
        .await?;
    std::mem::forget(temp);
    Ok(relational)
}

fn seeded_taxonomy(role_key: &str) -> SeededArchitectureTaxonomy {
    SeededArchitectureTaxonomy {
        roles: vec![SeededArchitectureRole {
            canonical_key: role_key.to_string(),
            display_name: "Command Dispatcher".to_string(),
            description: "Routes CLI commands".to_string(),
            family: Some("entrypoint".to_string()),
            lifecycle_status: Some("active".to_string()),
            provenance: json!({"source": "test"}),
            evidence: json!(["cli surface"]),
        }],
        rule_candidates: vec![SeededArchitectureRuleCandidate {
            target_role_key: role_key.to_string(),
            candidate_selector: RoleRuleCandidateSelector {
                path_prefixes: vec!["src/cli".to_string()],
                ..Default::default()
            },
            positive_conditions: vec![],
            negative_conditions: vec![],
            score: RoleRuleScore {
                base_confidence: Some(0.9),
                weight: Some(1.0),
            },
            evidence: json!(["path prefix"]),
            metadata: json!({"source": "test"}),
        }],
    }
}

#[test]
fn configured_seed_profile_name_requires_fact_synthesis_config() {
    let err = configured_seed_profile_name(Some(&json!({}))).expect_err("missing config");
    assert!(
        err.to_string()
            .contains("[architecture.inference].fact_synthesis")
    );

    let profile = configured_seed_profile_name(Some(
        &json!({"inference": {"fact_synthesis": "local_agent"}}),
    ))
    .expect("configured profile");
    assert_eq!(profile, "local_agent");
}

#[test]
fn role_adjudication_queue_item_parses_valid_payload() {
    let item = role_adjudication_queue_item_from_job(WorkplaneJobRecord {
        job_id: "job-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: std::path::PathBuf::from("/tmp/repo"),
        config_root: std::path::PathBuf::from("/tmp/config"),
        capability_id: "architecture_graph".to_string(),
        mailbox_name: ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX.to_string(),
        init_session_id: None,
        dedupe_key: Some("k1".to_string()),
        payload: json!({
            "request": {
                "repo_id": "repo-1",
                "generation": 8,
                "artefact_id": "a1",
                "symbol_id": "s1",
                "path": "src/main.rs",
                "language": "rust",
                "canonical_kind": "function",
                "reason": "high_impact",
                "deterministic_confidence": 0.72,
                "candidate_role_ids": [],
                "current_assignment": null
            }
        }),
        status: WorkplaneJobStatus::Pending,
        attempts: 0,
        available_at_unix: 1,
        submitted_at_unix: 1,
        started_at_unix: None,
        updated_at_unix: 1,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: None,
    });

    assert_eq!(item.reason.as_deref(), Some("high_impact"));
    assert_eq!(item.path.as_deref(), Some("src/main.rs"));
    assert!(item.parse_error.is_none());
}

#[test]
fn role_adjudication_queue_item_keeps_malformed_payload_as_parse_error() {
    let item = role_adjudication_queue_item_from_job(WorkplaneJobRecord {
        job_id: "job-2".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: std::path::PathBuf::from("/tmp/repo"),
        config_root: std::path::PathBuf::from("/tmp/config"),
        capability_id: "architecture_graph".to_string(),
        mailbox_name: ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX.to_string(),
        init_session_id: None,
        dedupe_key: Some("k2".to_string()),
        payload: json!({"bad": "shape"}),
        status: WorkplaneJobStatus::Failed,
        attempts: 2,
        available_at_unix: 1,
        submitted_at_unix: 1,
        started_at_unix: None,
        updated_at_unix: 2,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: Some("schema mismatch".to_string()),
    });

    assert!(item.reason.is_none());
    assert!(item.parse_error.is_some());
    assert_eq!(item.last_error.as_deref(), Some("schema mismatch"));
}

#[test]
fn roles_classify_formats_json_metrics() -> Result<()> {
    let output = RolesClassifyOutput {
        roles: ArchitectureRoleReconcileMetrics {
            full_reconcile: true,
            affected_paths: 0,
            refreshed_paths: 2,
            removed_paths: 0,
            skipped_unchanged_paths: 0,
            facts_written: 4,
            facts_deleted: 0,
            rules_loaded: 1,
            signals_written: 0,
            signals_deleted: 0,
            assignments_written: 0,
            assignments_marked_stale: 0,
            assignment_history_rows: 0,
            adjudication_candidates: 0,
        },
        role_adjudication_selected: 0,
        role_adjudication_enqueued: 0,
        role_adjudication_deduped: 0,
        warnings: Vec::new(),
    };

    let rendered = format_roles_classify_output(&output, true)?;
    let parsed: serde_json::Value = serde_json::from_str(&rendered)?;

    assert_eq!(parsed["roles"]["full_reconcile"], true);
    assert_eq!(parsed["roles"]["adjudication_candidates"], 0);
    Ok(())
}

#[tokio::test]
async fn persist_seeded_taxonomy_is_idempotent_for_repeated_runs() -> Result<()> {
    let relational = relational().await?;

    let first = persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        seeded_taxonomy("command_dispatcher"),
    )
    .await?;
    assert_eq!(first.roles_created, 1);
    assert_eq!(first.rules_created, 1);

    let second = persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        seeded_taxonomy("command_dispatcher"),
    )
    .await?;
    assert_eq!(second.roles_reused, 1);
    assert_eq!(second.rules_reused, 1);

    let roles = list_roles(&relational, "repo-1").await?;
    assert_eq!(roles.len(), 1);
    let rules = load_role_rules(&relational, "repo-1", &roles[0].role_id).await?;
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].lifecycle_status, "draft");
    Ok(())
}

#[tokio::test]
async fn seeded_rule_candidate_persists_loads_compiles_and_evaluates_over_facts() -> Result<()> {
    let relational = relational().await?;
    let mut taxonomy = seeded_taxonomy("command_dispatcher");
    taxonomy.rule_candidates[0].positive_conditions = vec![RoleRuleCondition {
        kind: "path_contains".to_string(),
        value: json!("commands"),
    }];

    persist_seeded_taxonomy(&relational, "repo-1", "local_agent", taxonomy).await?;
    let role = list_roles(&relational, "repo-1")
        .await?
        .into_iter()
        .next()
        .expect("seeded role");
    let rule = load_role_rules(&relational, "repo-1", &role.role_id)
        .await?
        .into_iter()
        .next()
        .expect("seeded rule");
    assert_eq!(
        rule.candidate_selector,
        json!({
            "targetKinds": [],
            "pathPrefixes": ["src/cli"],
            "pathSuffixes": [],
            "requiredFacts": []
        })
    );
    assert_eq!(
        rule.positive_conditions,
        json!([
            { "kind": "path", "key": "full", "op": "contains", "value": "commands", "score": 1.0 }
        ])
    );
    assert!(
        load_active_detection_rules(&relational, "repo-1")
            .await?
            .is_empty()
    );

    update_role_rule_lifecycle(&relational, "repo-1", &rule.rule_id, "active").await?;
    let active_rules = load_active_detection_rules(&relational, "repo-1").await?;
    let compiled = compile_detection_rules(active_rules)?;
    let affected_paths = BTreeSet::from(["src/cli/commands/run.rs".to_string()]);
    let extraction = extract_architecture_role_facts(ArchitectureRoleFactExtractionInput {
        repo_id: "repo-1",
        generation_seq: 7,
        affected_paths: &affected_paths,
        files: &[CurrentCanonicalFileRecord {
            repo_id: "repo-1".to_string(),
            path: "src/cli/commands/run.rs".to_string(),
            analysis_mode: "parsed".to_string(),
            file_role: "source".to_string(),
            language: "rust".to_string(),
            resolved_language: "rust".to_string(),
            effective_content_id: "content-1".to_string(),
            parser_version: "test".to_string(),
            extractor_version: "test".to_string(),
            exists_in_head: true,
            exists_in_index: true,
            exists_in_worktree: true,
        }],
        artefacts: &[],
        dependency_edges: &[],
    });

    let result = evaluate_rules_over_facts(&compiled, &extraction.facts)?;

    assert_eq!(result.signals.len(), 1);
    assert_eq!(result.signals[0].role_id, role.role_id);
    assert_eq!(result.signals[0].score, 0.9);
    assert_eq!(result.signals[0].generation_seq, 7);
    Ok(())
}

#[tokio::test]
async fn roles_status_reads_review_items_from_current_assignments() -> Result<()> {
    let relational = relational().await?;
    let target = RoleTarget::artefact("artefact-1", "symbol-1", "src/cli/run.rs");
    let assignment = ArchitectureRoleAssignment {
        repo_id: "repo-1".to_string(),
        assignment_id: assignment_id("repo-1", "role-1", &target),
        role_id: "role-1".to_string(),
        target,
        priority: AssignmentPriority::Primary,
        status: AssignmentStatus::NeedsReview,
        source: AssignmentSource::Rule,
        confidence: 0.55,
        evidence: json!([]),
        provenance: json!({"statusReason": "low confidence"}),
        classifier_version: "test".to_string(),
        rule_version: Some(1),
        generation_seq: 3,
    };
    upsert_assignment(&relational, &assignment).await?;

    let review_items = load_role_review_items(&relational, "repo-1", 10).await?;

    assert_eq!(review_items.len(), 1);
    assert_eq!(review_items[0].assignment_id, assignment.assignment_id);
    assert_eq!(review_items[0].artefact_id, "artefact-1");
    assert_eq!(review_items[0].source_kind, "rule");
    assert_eq!(review_items[0].status, "needs_review");
    assert_eq!(review_items[0].status_reason, "low confidence");
    assert_eq!(review_items[0].path.as_deref(), Some("src/cli/run.rs"));
    Ok(())
}

#[tokio::test]
async fn persist_seeded_taxonomy_reuses_alias_equivalent_roles() -> Result<()> {
    let relational = relational().await?;
    let existing = ArchitectureRoleRecord {
        role_id: deterministic_role_id("repo-1", "command_dispatcher"),
        repo_id: "repo-1".to_string(),
        canonical_key: "command_dispatcher".to_string(),
        display_name: "Command Dispatcher".to_string(),
        description: "Routes CLI commands".to_string(),
        family: Some("entrypoint".to_string()),
        lifecycle_status: "active".to_string(),
        provenance: json!({"source": "test"}),
        evidence: json!([]),
        metadata: json!({}),
    };
    let existing = upsert_role(&relational, &existing).await?;
    ensure_seed_alias(
        &relational,
        &ArchitectureRoleAliasRecord {
            alias_id: deterministic_alias_id("repo-1", "cli_command_dispatcher"),
            repo_id: "repo-1".to_string(),
            role_id: existing.role_id.clone(),
            alias_key: "cli_command_dispatcher".to_string(),
            alias_normalized: normalize_role_alias("cli_command_dispatcher"),
            source_kind: "manual".to_string(),
            metadata: json!({}),
        },
    )
    .await?;

    let summary = persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        seeded_taxonomy("cli_command_dispatcher"),
    )
    .await?;
    assert_eq!(summary.roles_created, 0);
    assert_eq!(summary.roles_reused, 1);

    let roles = list_roles(&relational, "repo-1").await?;
    assert_eq!(roles.len(), 1);
    let loaded = load_role_by_id(&relational, "repo-1", &existing.role_id)
        .await?
        .expect("existing role");
    assert_eq!(loaded.canonical_key, "command_dispatcher");
    let rules = load_role_rules(&relational, "repo-1", &existing.role_id).await?;
    assert_eq!(rules.len(), 1);
    Ok(())
}
