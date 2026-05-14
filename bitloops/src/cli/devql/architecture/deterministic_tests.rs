use super::*;
use crate::capability_packs::architecture_graph::roles::contracts::RoleTaxonomyReader;
use crate::capability_packs::architecture_graph::roles::fact_extraction::{
    ArchitectureRoleFactExtractionInput, SliceArchitectureRoleCurrentStateSource,
    extract_architecture_role_facts,
};
use crate::capability_packs::architecture_graph::roles::rules::{
    compile_detection_rules, evaluate_rules_over_facts,
};
use crate::capability_packs::architecture_graph::roles::storage::{
    ArchitectureRoleAliasRecord, ArchitectureRoleRecord, ArchitectureRoleRuleRecord,
    deterministic_alias_id, deterministic_role_id, deterministic_rule_id, insert_role_rule,
    list_roles, load_active_detection_rules, load_role_by_id, load_role_rules,
    next_role_rule_version, normalize_role_alias, update_role_rule_lifecycle, upsert_assignment,
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

fn test_scope() -> SlimCliRepoScope {
    SlimCliRepoScope {
        repo: crate::host::devql::RepoIdentity {
            repo_id: "repo-1".to_string(),
            provider: "git".to_string(),
            organization: "bitloops".to_string(),
            name: "demo".to_string(),
            identity: "git/bitloops/demo".to_string(),
        },
        repo_root: std::path::PathBuf::from("/tmp/demo"),
        branch_name: "main".to_string(),
        project_path: None,
        git_dir_relative_path: ".git".to_string(),
        config_fingerprint: "fingerprint".to_string(),
    }
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
fn architecture_roles_status_does_not_require_current_state_context() {
    let args = DevqlArchitectureRolesArgs {
        command: DevqlArchitectureRolesCommand::Status(DevqlArchitectureRolesStatusArgs {
            json: true,
            limit: 10,
        }),
    };

    assert!(
        !architecture_roles_command_requires_current_state_context(&args),
        "roles status must be routed before current-state consumer context construction"
    );
}

#[test]
fn role_adjudication_queue_item_maps_failed_job_payload_errors() {
    let job = WorkplaneJobRecord {
        job_id: "workplane-job-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: std::path::PathBuf::from("/tmp/repo"),
        config_root: std::path::PathBuf::from("/tmp/config"),
        capability_id: ARCHITECTURE_GRAPH_CAPABILITY_ID.to_string(),
        mailbox_name: ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX.to_string(),
        init_session_id: None,
        dedupe_key: Some("dedupe-1".to_string()),
        payload: serde_json::Value::Null,
        status: WorkplaneJobStatus::Failed,
        attempts: 2,
        available_at_unix: 1,
        submitted_at_unix: 1,
        started_at_unix: Some(2),
        updated_at_unix: 3,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: Some("database is locked".to_string()),
    };

    let item = role_adjudication_queue_item_from_job(job);

    assert_eq!(item.status, "failed");
    assert_eq!(item.attempts, 2);
    assert!(
        item.parse_error
            .as_deref()
            .unwrap_or_default()
            .contains("expected struct RoleAdjudicationMailboxPayload")
    );
    assert_eq!(item.last_error.as_deref(), Some("database is locked"));
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

#[test]
fn roles_classify_formats_human_metrics_without_json() -> Result<()> {
    let output = RolesClassifyOutput {
        roles: ArchitectureRoleReconcileMetrics {
            full_reconcile: true,
            affected_paths: 0,
            refreshed_paths: 2,
            removed_paths: 0,
            skipped_unchanged_paths: 0,
            facts_written: 4,
            facts_deleted: 1,
            rules_loaded: 3,
            signals_written: 5,
            signals_deleted: 2,
            assignments_written: 6,
            assignments_marked_stale: 0,
            assignment_history_rows: 6,
            adjudication_candidates: 1,
        },
        role_adjudication_selected: 1,
        role_adjudication_enqueued: 0,
        role_adjudication_deduped: 1,
        warnings: vec!["classification warning".to_string()],
    };

    let rendered = format_roles_classify_output(&output, false)?;

    assert!(rendered.contains("architecture roles classified"));
    assert!(rendered.contains("roles: full_reconcile=true"));
    assert!(rendered.contains("assignments: written=6"));
    assert!(rendered.contains("warning: classification warning"));
    Ok(())
}

#[test]
fn seed_command_output_includes_activation_and_classification_in_json() -> Result<()> {
    let summary = SeedCommandSummary {
        seed: SeedSummary {
            profile_name: "local_agent".to_string(),
            roles_total: 1,
            roles_created: 1,
            roles_reused: 0,
            rules_total: 1,
            rules_created: 1,
            rules_reused: 0,
        },
        rule_activation: Some(SeedRuleActivationSummary {
            seed_owned_draft_rules: 1,
            proposals_created: 1,
            proposals_applied: 1,
            activated_rule_ids: vec!["rule-1".to_string()],
            proposal_ids: vec!["proposal-1".to_string()],
        }),
        classification: Some(RolesClassifyOutput {
            roles: ArchitectureRoleReconcileMetrics {
                full_reconcile: true,
                affected_paths: 0,
                refreshed_paths: 2,
                removed_paths: 0,
                skipped_unchanged_paths: 0,
                facts_written: 4,
                facts_deleted: 0,
                rules_loaded: 1,
                signals_written: 2,
                signals_deleted: 0,
                assignments_written: 2,
                assignments_marked_stale: 0,
                assignment_history_rows: 2,
                adjudication_candidates: 0,
            },
            role_adjudication_selected: 0,
            role_adjudication_enqueued: 0,
            role_adjudication_deduped: 0,
            warnings: Vec::new(),
        }),
    };

    let rendered = format_seed_command_output(&summary, true)?;
    let value: serde_json::Value = serde_json::from_str(&rendered)?;

    assert_eq!(value["seed"]["profile_name"], "local_agent");
    assert_eq!(value["rule_activation"]["proposals_applied"], 1);
    assert_eq!(value["classification"]["roles"]["rules_loaded"], 1);
    Ok(())
}

#[test]
fn seed_command_output_keeps_seed_only_human_output_unchanged() -> Result<()> {
    let summary = SeedCommandSummary {
        seed: SeedSummary {
            profile_name: "local_agent".to_string(),
            roles_total: 1,
            roles_created: 1,
            roles_reused: 0,
            rules_total: 1,
            rules_created: 1,
            rules_reused: 0,
        },
        rule_activation: None,
        classification: None,
    };

    let rendered = format_seed_command_output(&summary, false)?;

    assert_eq!(
        rendered,
        "architecture roles seeded with profile `local_agent`\nroles: total=1 created=1 reused=0\nrules: total=1 created=1 reused=0"
    );
    Ok(())
}

#[test]
fn architecture_seed_diagnostics_count_evidence_and_prompt_bytes() {
    let evidence = json!({
        "canonical_files": [
            { "path": "src/main.rs" },
            { "path": "src/lib.rs" }
        ],
        "canonical_artefacts": [
            { "artefact_id": "a1" }
        ],
        "dependency_graph_hints": [],
        "existing_architecture_graph_facts": [
            { "label": "CLI" }
        ],
        "artefact_summaries": [
            { "artefact_id": "a1", "summary": "Routes commands." }
        ]
    });

    let request = crate::capability_packs::architecture_graph::roles::llm_adjudication
        ::architecture_roles_seed_roles_request(&test_scope(), &evidence);
    let diagnostics = architecture_seed_request_diagnostics(
        "role_discovery",
        "architecture_fact_synthesis_codex",
        Some("codex_exec"),
        Some("codex"),
        Some("gpt-5.4-mini"),
        &request,
        &evidence,
    );

    assert_eq!(
        diagnostics.profile_name,
        "architecture_fact_synthesis_codex"
    );
    assert_eq!(diagnostics.files, 2);
    assert_eq!(diagnostics.artefacts, 1);
    assert_eq!(diagnostics.edges, 0);
    assert_eq!(diagnostics.graph_facts, 1);
    assert_eq!(diagnostics.summaries, 1);
    assert!(diagnostics.user_prompt_bytes > 0);

    let rendered = diagnostics.human_summary();
    assert!(rendered.contains("phase=role_discovery"));
    assert!(rendered.contains("profile=architecture_fact_synthesis_codex"));
    assert!(rendered.contains("model=gpt-5.4-mini"));
    assert!(rendered.contains("files=2"));
    assert!(rendered.contains("artefacts=1"));
    assert!(rendered.contains("prompt_bytes(system="));
}

#[test]
fn bootstrap_skip_seed_formats_json_with_skipped_seed_flag() -> Result<()> {
    let summary = BootstrapCommandSummary {
        seed: None,
        rule_activation: SeedRuleActivationSummary {
            seed_owned_draft_rules: 1,
            proposals_created: 1,
            proposals_applied: 1,
            activated_rule_ids: vec!["rule-1".to_string()],
            proposal_ids: vec!["proposal-1".to_string()],
        },
        classification: RolesClassifyOutput {
            roles: ArchitectureRoleReconcileMetrics {
                full_reconcile: true,
                affected_paths: 0,
                refreshed_paths: 1,
                removed_paths: 0,
                skipped_unchanged_paths: 0,
                facts_written: 1,
                facts_deleted: 0,
                rules_loaded: 1,
                signals_written: 1,
                signals_deleted: 0,
                assignments_written: 1,
                assignments_marked_stale: 0,
                assignment_history_rows: 1,
                adjudication_candidates: 0,
            },
            role_adjudication_selected: 0,
            role_adjudication_enqueued: 0,
            role_adjudication_deduped: 0,
            warnings: Vec::new(),
        },
        skipped_seed: true,
    };

    let rendered = format_bootstrap_command_output(&summary, true)?;
    let value: serde_json::Value = serde_json::from_str(&rendered)?;
    assert!(rendered.contains("\"skipped_seed\": true"));
    assert!(rendered.contains("\"seed\": null"));
    assert!(rendered.contains("\"rule_activation\""));
    assert!(rendered.contains("\"classification\""));
    assert_eq!(value["skipped_seed"], true);
    assert!(value["seed"].is_null());
    assert_eq!(value["rule_activation"]["proposals_applied"], 1);
    assert_eq!(value["classification"]["roles"]["rules_loaded"], 1);
    Ok(())
}

#[test]
fn seed_classify_requires_activate_rules() {
    let err = validate_seed_automation_args(&DevqlArchitectureRolesSeedArgs {
        activate_rules: false,
        classify: true,
        enqueue_adjudication: true,
        json: false,
    })
    .expect_err("classify without activate-rules should fail");

    assert!(err.to_string().contains("requires `--activate-rules`"));
}

#[tokio::test]
async fn persist_seeded_taxonomy_defaults_missing_lifecycle_to_active() -> Result<()> {
    let relational = relational().await?;
    let mut taxonomy = seeded_taxonomy("command_dispatcher");
    taxonomy.roles[0].lifecycle_status = None;

    persist_seeded_taxonomy(&relational, "repo-1", "local_agent", taxonomy).await?;

    let roles = list_roles(&relational, "repo-1").await?;
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].lifecycle_status, "active");
    Ok(())
}

#[tokio::test]
async fn persist_seeded_taxonomy_rejects_stable_lifecycle() -> Result<()> {
    let relational = relational().await?;
    let mut taxonomy = seeded_taxonomy("command_dispatcher");
    taxonomy.roles[0].lifecycle_status = Some("stable".to_string());

    let err = persist_seeded_taxonomy(&relational, "repo-1", "local_agent", taxonomy)
        .await
        .expect_err("stable lifecycle should not persist");

    assert!(
        err.to_string()
            .contains("unsupported seeded role lifecycle_status `stable`")
    );
    assert!(list_roles(&relational, "repo-1").await?.is_empty());
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
async fn persisted_seed_roles_are_active_adjudication_candidates() -> Result<()> {
    let relational = relational().await?;
    persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        seeded_taxonomy("command_dispatcher"),
    )
    .await?;

    let candidates =
        crate::capability_packs::architecture_graph::roles::DbRoleTaxonomyReader::new(&relational)
            .load_active_roles("repo-1", 1)
            .await?;

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].canonical_key, "command_dispatcher");
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
    let files = vec![CurrentCanonicalFileRecord {
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
    }];
    let artefacts = Vec::new();
    let dependency_edges = Vec::new();
    let current_state = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &dependency_edges);
    let extraction = extract_architecture_role_facts(
        ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 7,
            affected_paths: &affected_paths,
            files: &files,
        },
        &current_state,
    )?;

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
async fn roles_status_reads_recent_adjudication_attempts() -> Result<()> {
    let relational = relational().await?;
    relational
        .exec(&format!(
            "INSERT INTO architecture_role_adjudication_attempts (
                repo_id, attempt_id, scope_key, generation_seq, target_kind, artefact_id,
                symbol_id, path, reason, deterministic_confidence, candidate_roles_json,
                current_assignment_json, request_json, evidence_packet_sha256,
                evidence_packet_json, model_descriptor, slot_name, outcome, raw_response_json,
                validated_result_json, failure_message, retryable, assignment_write_persisted,
                assignment_write_source, observed_at_unix
             ) VALUES (
                {repo_id}, 'attempt-1', 'repo-1:src/cli/run.rs', 7, 'artefact', 'artefact-1',
                'symbol-1', 'src/cli/run.rs', 'low_confidence', 0.55, '[]',
                NULL, '{{}}', 'sha-1', '{{}}', 'test-model', 'architecture-role-adjudication',
                'assigned', NULL, {validated_result_json}, NULL, 0, 1, 'llm', 1234
             );",
            repo_id = sql_text("repo-1"),
            validated_result_json =
                sql_text(&json!({"reasoning_summary": "Selected command dispatcher"}).to_string()),
        ))
        .await?;
    relational
        .exec(&format!(
            "INSERT INTO architecture_role_adjudication_attempts (
                repo_id, attempt_id, scope_key, generation_seq, target_kind, artefact_id,
                symbol_id, path, reason, deterministic_confidence, candidate_roles_json,
                current_assignment_json, request_json, evidence_packet_sha256,
                evidence_packet_json, model_descriptor, slot_name, outcome, raw_response_json,
                validated_result_json, failure_message, retryable, assignment_write_persisted,
                assignment_write_source, observed_at_unix
             ) VALUES (
                {repo_id}, 'attempt-2', 'repo-1:src/cli/run.rs:skipped', 8, 'artefact', 'artefact-1',
                'symbol-1', 'src/cli/run.rs', 'low_confidence', 0.55, '[]',
                NULL, '{{}}', 'sha-2', '{{}}', 'skipped', 'architecture-role-adjudication',
                'skipped_deterministic', NULL, NULL,
                'active deterministic rule assignment already exists for target', 0, 0,
                'skipped_deterministic_assignment', 1235
             );",
            repo_id = sql_text("repo-1"),
        ))
        .await?;

    let attempts = load_role_adjudication_attempt_items(&relational, "repo-1", 10).await?;

    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].attempt_id, "attempt-2");
    assert_eq!(attempts[0].outcome, "skipped_deterministic");
    assert_eq!(
        attempts[0].assignment_write_source.as_deref(),
        Some("skipped_deterministic_assignment")
    );
    assert!(!attempts[0].assignment_write_persisted);
    assert_eq!(attempts[1].attempt_id, "attempt-1");
    assert_eq!(attempts[1].outcome, "assigned");
    assert_eq!(attempts[1].path.as_deref(), Some("src/cli/run.rs"));
    assert_eq!(
        attempts[1].reasoning_summary.as_deref(),
        Some("Selected command dispatcher")
    );
    assert!(attempts[1].assignment_write_persisted);
    assert_eq!(attempts[1].assignment_write_source.as_deref(), Some("llm"));
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

#[tokio::test]
async fn activate_seeded_draft_rules_only_applies_seed_owned_rules() -> Result<()> {
    use sha2::{Digest, Sha256};

    let relational = relational().await?;
    persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        seeded_taxonomy("command_dispatcher"),
    )
    .await?;
    let role = list_roles(&relational, "repo-1")
        .await?
        .into_iter()
        .next()
        .expect("seeded role");
    let manual_hash = hex::encode(Sha256::digest(b"manual-rule"));
    let manual_version = next_role_rule_version(&relational, "repo-1", &role.role_id).await?;
    let manual_rule = ArchitectureRoleRuleRecord {
        rule_id: deterministic_rule_id("repo-1", &role.role_id, manual_version, &manual_hash),
        repo_id: "repo-1".to_string(),
        role_id: role.role_id.clone(),
        version: manual_version,
        lifecycle_status: "draft".to_string(),
        canonical_hash: manual_hash,
        candidate_selector: json!({
            "targetKinds": [],
            "pathPrefixes": ["src/manual"],
            "pathSuffixes": [],
            "requiredFacts": []
        }),
        positive_conditions: json!([]),
        negative_conditions: json!([]),
        score: json!({"base_confidence": 0.8, "weight": 1.0}),
        provenance: json!({"source": "manual_test"}),
        evidence: json!([]),
        metadata: json!({}),
        supersedes_rule_id: None,
    };
    insert_role_rule(&relational, &manual_rule).await?;

    let activation = activate_seeded_draft_rules(
        &relational,
        "repo-1",
        "local_agent",
        cli_provenance("seed_activate_rules"),
    )
    .await?;

    assert_eq!(activation.seed_owned_draft_rules, 1);
    assert_eq!(activation.proposals_created, 1);
    assert_eq!(activation.proposals_applied, 1);
    assert_eq!(activation.activated_rule_ids.len(), 1);

    let rules = load_role_rules(&relational, "repo-1", &role.role_id).await?;
    let active_seed_rules = rules
        .iter()
        .filter(|rule| {
            rule.lifecycle_status == "active"
                && rule
                    .provenance
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    == Some("architecture_roles_seed")
        })
        .count();
    let draft_manual_rules = rules
        .iter()
        .filter(|rule| {
            rule.lifecycle_status == "draft"
                && rule
                    .provenance
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    == Some("manual_test")
        })
        .count();
    assert_eq!(active_seed_rules, 1);
    assert_eq!(draft_manual_rules, 1);
    Ok(())
}

#[tokio::test]
async fn activate_seeded_draft_rules_is_idempotent_after_first_activation() -> Result<()> {
    let relational = relational().await?;
    persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        seeded_taxonomy("command_dispatcher"),
    )
    .await?;

    let first = activate_seeded_draft_rules(
        &relational,
        "repo-1",
        "local_agent",
        cli_provenance("seed_activate_rules"),
    )
    .await?;
    let second = activate_seeded_draft_rules(
        &relational,
        "repo-1",
        "local_agent",
        cli_provenance("seed_activate_rules"),
    )
    .await?;

    assert_eq!(first.seed_owned_draft_rules, 1);
    assert_eq!(first.proposals_applied, 1);
    assert_eq!(second.seed_owned_draft_rules, 0);
    assert_eq!(second.proposals_applied, 0);
    assert!(second.activated_rule_ids.is_empty());
    Ok(())
}

#[tokio::test]
async fn seed_activation_enables_full_classification() -> Result<()> {
    let relational = relational().await?;
    let mut taxonomy = seeded_taxonomy("command_dispatcher");
    taxonomy.rule_candidates[0].candidate_selector = RoleRuleCandidateSelector {
        path_prefixes: vec!["src/cli".to_string()],
        ..Default::default()
    };
    taxonomy.rule_candidates[0].score = RoleRuleScore {
        base_confidence: Some(0.95),
        weight: Some(1.0),
    };
    taxonomy.rule_candidates[0].positive_conditions = vec![RoleRuleCondition {
        kind: "path_contains".to_string(),
        value: json!("commands"),
    }];

    persist_seeded_taxonomy(&relational, "repo-1", "local_agent", taxonomy).await?;
    activate_seeded_draft_rules(
        &relational,
        "repo-1",
        "local_agent",
        cli_provenance("seed_activate_rules"),
    )
    .await?;

    let files = vec![CurrentCanonicalFileRecord {
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
    }];
    let artefacts = Vec::new();
    let dependency_edges = Vec::new();
    let current_state = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &dependency_edges);

    let outcome =
        crate::capability_packs::architecture_graph::roles::classifier::classify_architecture_roles_for_current_state(
            &relational,
            &current_state,
            crate::capability_packs::architecture_graph::roles::classifier::ArchitectureRoleClassificationInput {
                repo_id: "repo-1",
                generation_seq: 7,
                scope: crate::capability_packs::architecture_graph::roles::classifier::ArchitectureRoleClassificationScope {
                    full_reconcile: true,
                    affected_paths: BTreeSet::new(),
                    removed_paths: BTreeSet::new(),
                },
                files: &files,
            },
        )
        .await?;

    assert!(outcome.metrics.full_reconcile);
    assert_eq!(outcome.metrics.rules_loaded, 1);
    assert!(outcome.metrics.signals_written >= 1);
    assert!(outcome.metrics.assignments_written >= 1);
    assert_eq!(outcome.metrics.adjudication_candidates, 0);
    Ok(())
}
