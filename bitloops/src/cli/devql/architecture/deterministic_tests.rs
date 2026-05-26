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
    SeededArchitectureTaxonomy, TargetKind, assignment_id,
    decode_seeded_rule_candidates_with_recovery,
};
use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;
use crate::cli::devql::architecture::roles_seed::ArchitectureSeedProfileDiagnostics;
use crate::host::capability_host::gateways::RelationalGateway;
use crate::host::devql::RelationalStorage;
use crate::host::runtime_store::{WorkplaneJobRecord, WorkplaneJobStatus};
use crate::models::{
    CurrentCanonicalArtefactRecord, CurrentCanonicalFileRecord, ProductionArtefact,
};
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

struct PreviewGateway {
    artefacts: Vec<CurrentCanonicalArtefactRecord>,
}

impl PreviewGateway {
    fn with_cli_artefact() -> Self {
        Self {
            artefacts: vec![CurrentCanonicalArtefactRecord {
                repo_id: "repo-1".to_string(),
                path: "src/cli/commands/run.rs".to_string(),
                content_id: "content-1".to_string(),
                symbol_id: "symbol-1".to_string(),
                artefact_id: "artefact-1".to_string(),
                language: "rust".to_string(),
                extraction_fingerprint: "fingerprint".to_string(),
                canonical_kind: Some("function".to_string()),
                language_kind: Some("function".to_string()),
                symbol_fqn: Some("crate::cli::commands::run".to_string()),
                parent_symbol_id: None,
                parent_artefact_id: None,
                start_line: 1,
                end_line: 10,
                start_byte: 0,
                end_byte: 50,
                signature: Some("fn run()".to_string()),
                modifiers: "[]".to_string(),
                docstring: None,
            }],
        }
    }

    fn empty() -> Self {
        Self {
            artefacts: Vec::new(),
        }
    }
}

impl RelationalGateway for PreviewGateway {
    fn resolve_checkpoint_id(&self, _repo_id: &str, _checkpoint_ref: &str) -> Result<String> {
        anyhow::bail!("not used")
    }

    fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
        anyhow::bail!("not used")
    }

    fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
        anyhow::bail!("not used")
    }

    fn load_current_canonical_artefacts(
        &self,
        _repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalArtefactRecord>> {
        Ok(self.artefacts.clone())
    }

    fn load_current_production_artefacts(&self, _repo_id: &str) -> Result<Vec<ProductionArtefact>> {
        anyhow::bail!("not used")
    }

    fn load_production_artefacts(&self, _commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
        anyhow::bail!("not used")
    }

    fn load_artefacts_for_file_lines(
        &self,
        _commit_sha: &str,
        _file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>> {
        anyhow::bail!("not used")
    }
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
            positive_conditions: vec![RoleRuleCondition {
                kind: "path_contains".to_string(),
                value: json!("commands"),
                ..Default::default()
            }],
            negative_conditions: vec![],
            score: RoleRuleScore {
                base_confidence: Some(0.9),
                priority_hint: Some(100),
                min_positive_ratio: Some(1.0),
            },
            evidence: json!(["path prefix"]),
            metadata: json!({"source": "test"}),
        }],
    }
}

fn seeded_taxonomy_with_artefact_target(role_key: &str) -> SeededArchitectureTaxonomy {
    let mut taxonomy = seeded_taxonomy(role_key);
    taxonomy.rule_candidates[0].candidate_selector.target_kinds = vec![TargetKind::Artefact];
    taxonomy
}

fn seeded_taxonomy_with_file_target(canonical_key: &str) -> SeededArchitectureTaxonomy {
    SeededArchitectureTaxonomy {
        roles: vec![SeededArchitectureRole {
            canonical_key: canonical_key.to_string(),
            display_name: "Platform Bootstrapper".to_string(),
            description: "Startup file role".to_string(),
            family: Some("entrypoint".to_string()),
            lifecycle_status: Some("active".to_string()),
            provenance: json!({}),
            evidence: json!({}),
        }],
        rule_candidates: vec![SeededArchitectureRuleCandidate {
            target_role_key: canonical_key.to_string(),
            candidate_selector: RoleRuleCandidateSelector {
                target_kinds: vec![TargetKind::File],
                path_prefixes: vec!["src".to_string()],
                required_facts: vec![
                    RoleRuleCondition {
                        kind: "language".to_string(),
                        key: Some("resolved".to_string()),
                        op: Some(crate::capability_packs::architecture_graph::roles::taxonomy::RoleFactConditionOp::Eq),
                        value: json!("rust"),
                        score: Some(1.0),
                    },
                    RoleRuleCondition {
                        kind: "path".to_string(),
                        key: Some("full".to_string()),
                        op: Some(crate::capability_packs::architecture_graph::roles::taxonomy::RoleFactConditionOp::Eq),
                        value: json!("src/main.rs"),
                        score: Some(1.0),
                    },
                ],
                ..Default::default()
            },
            positive_conditions: vec![RoleRuleCondition {
                kind: "file".to_string(),
                key: Some("role".to_string()),
                op: Some(crate::capability_packs::architecture_graph::roles::taxonomy::RoleFactConditionOp::Eq),
                value: json!("source_code"),
                score: Some(1.0),
            }],
            negative_conditions: Vec::new(),
            score: RoleRuleScore {
                base_confidence: Some(0.90),
                priority_hint: Some(100),
                min_positive_ratio: Some(1.0),
            },
            evidence: json!({}),
            metadata: json!({}),
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
                "stable_request_key": "file:src/main.rs",
                "facts_hash": "facts",
                "rules_hash": "rules",
                "cluster_key": null,
                "target_kind": "file",
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
            ..ArchitectureRoleReconcileMetrics::default()
        },
        architecture_embedding_selected: 0,
        architecture_embedding_enqueued: 0,
        architecture_embedding_deduped: 0,
        role_adjudication_selected: 0,
        role_adjudication_enqueued: 0,
        role_adjudication_deduped: 0,
        warnings: Vec::new(),
    };

    let rendered = format_roles_classify_output(&output, true)?;
    let parsed: serde_json::Value = serde_json::from_str(&rendered)?;

    assert_eq!(parsed["roles"]["full_reconcile"], true);
    assert_eq!(parsed["roles"]["adjudication_candidates"], 0);
    assert_eq!(parsed["roles"]["unknown_targets_total"], 0);
    assert_eq!(parsed["roles"]["unknown_targets_suppressed_non_role"], 0);
    assert_eq!(parsed["roles"]["unknown_targets_rule_mining_eligible"], 0);
    assert_eq!(parsed["roles"]["unknown_targets_adjudication_escalated"], 0);
    assert_eq!(parsed["roles"]["role_mining_clusters"], 0);
    assert_eq!(parsed["roles"]["role_mining_representative_targets"], 0);
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
            target_count: 10,
            deterministic_active_targets: 6,
            deterministic_needs_review_targets: 2,
            deterministic_conflict_targets: 1,
            deterministic_unassigned_targets: 2,
            deterministic_coverage_ratio: 0.6,
            unknown_targets_total: 4,
            unknown_targets_suppressed_non_role: 2,
            unknown_targets_rule_mining_eligible: 1,
            unknown_targets_adjudication_escalated: 1,
            role_mining_clusters: 1,
            role_mining_representative_targets: 1,
            unknown_adjudication_candidates: 1,
            high_impact_adjudication_candidates: 0,
            low_confidence_adjudication_candidates: 1,
            conflict_adjudication_candidates: 1,
            repeated_adjudication_suppressed: 3,
            ..ArchitectureRoleReconcileMetrics::default()
        },
        architecture_embedding_selected: 2,
        architecture_embedding_enqueued: 1,
        architecture_embedding_deduped: 1,
        role_adjudication_selected: 1,
        role_adjudication_enqueued: 0,
        role_adjudication_deduped: 1,
        warnings: vec!["classification warning".to_string()],
    };

    let rendered = format_roles_classify_output(&output, false)?;

    assert!(rendered.contains("architecture roles classified"));
    let facts_index = rendered
        .find("facts: written=4 deleted=1")
        .expect("facts line present");
    let signals_index = rendered
        .find("signals: written=5 deleted=2")
        .expect("signals line present");
    let assignments_index = rendered
        .find("assignments: written=6 marked_stale=0 history_rows=6")
        .expect("assignments line present");
    let roles_index = rendered
        .find("roles: full_reconcile=true")
        .expect("roles summary line present");
    assert!(facts_index < signals_index);
    assert!(signals_index < assignments_index);
    assert!(assignments_index < roles_index);
    assert!(rendered.contains("assignments: written=6"));
    assert!(rendered.contains(
        "coverage: targets=10 active=6 review=2 conflict=1 unknown=2 deterministic_ratio=0.600"
    ));
    assert!(rendered.contains(
        "unknown policy: total=4 suppressed_non_role=2 rule_mining_eligible=1 adjudication_escalated=1"
    ));
    assert!(rendered.contains("rule mining: clusters=1 representative_targets=1"));
    assert!(rendered.contains(
        "adjudication reasons: unknown=1 high_impact=0 low_confidence=1 conflict=1 repeated_suppressed=3"
    ));
    assert!(rendered.contains("architecture embeddings: selected=2 enqueued=1 deduped=1"));
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
            recovery: SeedRecoverySummary {
                rule_candidates_accepted: 1,
                rule_candidates_repaired: 1,
                rule_candidates_rejected: 0,
                rule_batches_retried: 0,
                rule_batches_skipped: 0,
                warnings: Vec::new(),
            },
        },
        rule_activation: Some(SeedRuleActivationSummary {
            seed_owned_draft_rules: 1,
            proposals_created: 1,
            proposals_applied: 1,
            activated_rule_ids: vec!["rule-1".to_string()],
            proposal_ids: vec!["proposal-1".to_string()],
            blocked_rule_ids: Vec::new(),
            blocked_reasons: std::collections::BTreeMap::new(),
            blocked_diagnostics: std::collections::BTreeMap::new(),
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
                ..ArchitectureRoleReconcileMetrics::default()
            },
            architecture_embedding_selected: 0,
            architecture_embedding_enqueued: 0,
            architecture_embedding_deduped: 0,
            role_adjudication_selected: 0,
            role_adjudication_enqueued: 0,
            role_adjudication_deduped: 0,
            warnings: Vec::new(),
        }),
    };

    let rendered = format_seed_command_output(&summary, true)?;
    let value: serde_json::Value = serde_json::from_str(&rendered)?;

    assert_eq!(value["seed"]["profile_name"], "local_agent");
    assert_eq!(value["seed"]["recovery"]["rule_candidates_repaired"], 1);
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
            recovery: SeedRecoverySummary::default(),
        },
        rule_activation: None,
        classification: None,
    };

    let rendered = format_seed_command_output(&summary, false)?;

    assert_eq!(
        rendered,
        "architecture roles seeded with profile `local_agent`\nroles: total=1 created=1 reused=0\nrules: total=1 created=1 reused=0\nrecovery: rule_candidates_accepted=0 rule_candidates_repaired=0 rule_candidates_rejected=0 rule_batches_retried=0 rule_batches_skipped=0"
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
        ArchitectureSeedProfileDiagnostics {
            profile_name: "architecture_fact_synthesis_codex",
            driver: Some("codex_exec"),
            runtime: Some("codex"),
            model: Some("gpt-5.4-mini"),
            thinking_level: Some("xhigh"),
        },
        &request,
        &evidence,
        None,
    );

    assert_eq!(
        diagnostics.profile_name,
        "architecture_fact_synthesis_codex"
    );
    assert_eq!(diagnostics.files, 2);
    assert_eq!(diagnostics.signals, 0);
    assert_eq!(diagnostics.artefacts, 1);
    assert_eq!(diagnostics.edges, 0);
    assert_eq!(diagnostics.graph_facts, 1);
    assert_eq!(diagnostics.summaries, 1);
    assert_eq!(diagnostics.prompt_budget_bytes, None);
    assert_eq!(diagnostics.omitted_signals, 0);
    assert_eq!(diagnostics.omitted_files, 0);
    assert_eq!(diagnostics.thinking_level.as_deref(), Some("xhigh"));
    assert!(diagnostics.user_prompt_bytes > 0);

    let rendered = diagnostics.human_summary();
    assert!(rendered.contains("phase=role_discovery"));
    assert!(rendered.contains("profile=architecture_fact_synthesis_codex"));
    assert!(rendered.contains("model=gpt-5.4-mini"));
    assert!(rendered.contains("thinking_level=xhigh"));
    assert!(rendered.contains("files=2"));
    assert!(rendered.contains("artefacts=1"));
    assert!(rendered.contains("prompt_bytes(system="));
    assert!(rendered.contains("budget=none"));
}

#[test]
fn architecture_seed_diagnostics_include_budget_and_omitted_counts() {
    let evidence = json!({
        "repository": {},
        "language_framework_signals": [],
        "canonical_files": [
            { "path": "src/runtime.rs" }
        ],
        "canonical_artefacts": [],
        "dependency_graph_hints": [],
        "existing_architecture_graph_facts": [],
        "artefact_summaries": [],
        "generic_role_family_examples": []
    });
    let budgeted = crate::capability_packs::architecture_graph::roles::seed_evidence
        ::budget_role_discovery_evidence(&test_scope(), &evidence)
        .expect("budget evidence");
    let request = crate::capability_packs::architecture_graph::roles::llm_adjudication
        ::architecture_roles_seed_roles_request(&test_scope(), budgeted.evidence());

    let diagnostics = architecture_seed_request_diagnostics(
        "role_discovery",
        ArchitectureSeedProfileDiagnostics {
            profile_name: "architecture_fact_synthesis_codex",
            driver: Some("codex_exec"),
            runtime: Some("codex"),
            model: Some("gpt-5.4-mini"),
            thinking_level: Some("low"),
        },
        &request,
        budgeted.evidence(),
        Some(&budgeted),
    );

    assert_eq!(
        diagnostics.prompt_budget_bytes,
        Some(
            crate::capability_packs::architecture_graph::roles::seed_evidence
                ::ROLE_DISCOVERY_USER_PROMPT_BUDGET_BYTES
        )
    );
    assert_eq!(diagnostics.files, 1);
    assert_eq!(diagnostics.signals, 0);
    assert_eq!(diagnostics.omitted_signals, 0);
    assert_eq!(diagnostics.omitted_files, 0);
    assert!(diagnostics.human_summary().contains("budget=65536"));
    assert!(diagnostics.human_summary().contains("omitted(files=0"));
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
            blocked_rule_ids: Vec::new(),
            blocked_reasons: std::collections::BTreeMap::new(),
            blocked_diagnostics: std::collections::BTreeMap::new(),
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
                ..ArchitectureRoleReconcileMetrics::default()
            },
            architecture_embedding_selected: 0,
            architecture_embedding_enqueued: 0,
            architecture_embedding_deduped: 0,
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

    assert!(err.chain().any(|cause| {
        cause
            .to_string()
            .contains("unsupported seeded role lifecycle_status `stable`")
    }));
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
        ..Default::default()
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
async fn persist_seeded_taxonomy_rolls_back_when_accepted_rule_is_invalid() -> Result<()> {
    let relational = relational().await?;
    let mut taxonomy = seeded_taxonomy("command_dispatcher");
    taxonomy.rule_candidates[0].target_role_key = "missing_role".to_string();

    let err = persist_seeded_taxonomy(&relational, "repo-1", "local_agent", taxonomy)
        .await
        .expect_err("invalid accepted rule should fail before writes");

    assert!(!err.to_string().trim().is_empty());
    assert!(list_roles(&relational, "repo-1").await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn persist_seeded_taxonomy_records_alias_conflict_warning_without_losing_rule() -> Result<()>
{
    let relational = relational().await?;
    let seed_existing = ArchitectureRoleRecord {
        role_id: deterministic_role_id("repo-1", "command_dispatcher"),
        repo_id: "repo-1".to_string(),
        canonical_key: "command_dispatcher".to_string(),
        display_name: "Command Dispatcher".to_string(),
        description: "Seed role".to_string(),
        family: Some("entrypoint".to_string()),
        lifecycle_status: "active".to_string(),
        provenance: json!({"source": "test"}),
        evidence: json!([]),
        metadata: json!({}),
    };
    upsert_role(&relational, &seed_existing).await?;
    let existing = ArchitectureRoleRecord {
        role_id: deterministic_role_id("repo-1", "existing_role"),
        repo_id: "repo-1".to_string(),
        canonical_key: "existing_role".to_string(),
        display_name: "Existing Role".to_string(),
        description: "Existing role".to_string(),
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
            alias_id: deterministic_alias_id("repo-1", "Command Dispatcher"),
            repo_id: "repo-1".to_string(),
            role_id: existing.role_id.clone(),
            alias_key: "Command Dispatcher".to_string(),
            alias_normalized: normalize_role_alias("Command Dispatcher"),
            source_kind: "manual".to_string(),
            metadata: json!({}),
        },
    )
    .await?;

    let summary = persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        seeded_taxonomy("command_dispatcher"),
    )
    .await?;

    assert_eq!(summary.rules_created, 1);
    assert!(summary.recovery.warnings.iter().any(|warning| {
        warning.contains("conflicts with existing role") && warning.contains("skipped alias")
    }));
    let rules = load_role_rules(&relational, "repo-1", &seed_existing.role_id).await?;
    assert_eq!(rules.len(), 1);
    Ok(())
}

#[tokio::test]
async fn architecture_roles_bootstrap_repairs_signature_contains_contains_candidate() -> Result<()>
{
    let relational = relational().await?;
    let roles = vec![SeededArchitectureRole {
        canonical_key: "command_dispatcher".to_string(),
        display_name: "Command Dispatcher".to_string(),
        description: "Routes CLI commands".to_string(),
        family: Some("entrypoint".to_string()),
        lifecycle_status: Some("active".to_string()),
        provenance: json!({}),
        evidence: json!({}),
    }];
    let decoded = decode_seeded_rule_candidates_with_recovery(json!({
        "rule_candidates": [{
            "target_role_key": "command_dispatcher",
            "candidate_selector": {
                "target_kinds": ["artefact"],
                "path_prefixes": [],
                "path_suffixes": [],
                "path_contains": [],
                "languages": [],
                "canonical_kinds": [],
                "symbol_fqn_contains": [],
                "required_facts": [
                    { "kind": "signature", "key": "contains", "op": "contains", "value": "Result", "score": 0.5 }
                ],
                "required_fact_any_groups": []
            },
            "positive_conditions": [
                { "predicate": "path.full:contains", "value": "commands", "score": 1.0 }
            ],
            "negative_conditions": [],
            "score": { "base_confidence": 0.8, "priority_hint": 100, "min_positive_ratio": 1.0 },
            "evidence": {},
            "metadata": {}
        }]
    }));
    assert_eq!(decoded.repaired.len(), 1);
    assert_eq!(decoded.accepted.len(), 1);

    let summary = persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        SeededArchitectureTaxonomy {
            roles,
            rule_candidates: decoded.accepted,
        },
    )
    .await?;

    assert_eq!(summary.rules_created, 1);
    Ok(())
}

#[tokio::test]
async fn architecture_roles_bootstrap_drops_unrepairable_candidate_and_continues() -> Result<()> {
    let relational = relational().await?;
    let roles = vec![SeededArchitectureRole {
        canonical_key: "command_dispatcher".to_string(),
        display_name: "Command Dispatcher".to_string(),
        description: "Routes CLI commands".to_string(),
        family: Some("entrypoint".to_string()),
        lifecycle_status: Some("active".to_string()),
        provenance: json!({}),
        evidence: json!({}),
    }];
    let decoded = decode_seeded_rule_candidates_with_recovery(json!({
        "rule_candidates": [
            {
                "target_role_key": "command_dispatcher",
                "candidate_selector": {
                    "target_kinds": ["file"],
                    "path_prefixes": [],
                    "path_suffixes": [],
                    "path_contains": [],
                    "languages": [],
                    "canonical_kinds": [],
                    "symbol_fqn_contains": [],
                    "required_facts": [],
                    "required_fact_any_groups": []
                },
                "positive_conditions": [
                    { "predicate": "path.full:prefix", "value": "src/cli", "score": 1.0 }
                ],
                "negative_conditions": [],
                "score": { "base_confidence": 0.8, "priority_hint": 100, "min_positive_ratio": 1.0 },
                "evidence": {},
                "metadata": {}
            },
            {
                "target_role_key": "command_dispatcher",
                "candidate_selector": {
                    "target_kinds": ["file"],
                    "path_prefixes": [],
                    "path_suffixes": [],
                    "path_contains": [],
                    "languages": [],
                    "canonical_kinds": [],
                    "symbol_fqn_contains": [],
                    "required_facts": [],
                    "required_fact_any_groups": []
                },
                "positive_conditions": [
                    { "predicate": "signature.contains:contains", "value": "Result", "score": 1.0 }
                ],
                "negative_conditions": [],
                "score": { "base_confidence": 0.8, "priority_hint": 100, "min_positive_ratio": 1.0 },
                "evidence": {},
                "metadata": {}
            }
        ]
    }));
    assert_eq!(decoded.accepted.len(), 1);
    assert_eq!(decoded.rejected.len(), 1);

    let summary = persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        SeededArchitectureTaxonomy {
            roles,
            rule_candidates: decoded.accepted,
        },
    )
    .await?;

    assert_eq!(summary.rules_created, 1);
    Ok(())
}

#[tokio::test]
async fn seed_rule_activation_leaves_zero_match_rule_draft() -> Result<()> {
    let relational = relational().await?;
    persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        seeded_taxonomy_with_artefact_target("command_dispatcher"),
    )
    .await?;

    let activation = activate_seeded_draft_rules(
        &relational,
        &PreviewGateway::empty(),
        "repo-1",
        "local_agent",
        cli_provenance("seed_activate_rules"),
    )
    .await?;

    assert_eq!(activation.seed_owned_draft_rules, 1);
    assert_eq!(activation.proposals_created, 0);
    assert_eq!(activation.proposals_applied, 0);
    assert_eq!(activation.blocked_rule_ids.len(), 1);
    assert!(
        activation
            .blocked_reasons
            .values()
            .flatten()
            .any(|reason| reason == "zero_matches")
    );
    Ok(())
}

#[tokio::test]
async fn seed_activation_reports_zero_match_diagnostics() -> Result<()> {
    let relational = relational().await?;
    persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        seeded_taxonomy_with_artefact_target("command_dispatcher"),
    )
    .await?;

    let activation = activate_seeded_draft_rules(
        &relational,
        &PreviewGateway::empty(),
        "repo-1",
        "local_agent",
        cli_provenance("seed_activate_rules"),
    )
    .await?;

    let rule_id = activation
        .blocked_rule_ids
        .first()
        .expect("blocked rule id")
        .clone();
    assert_eq!(
        activation.blocked_diagnostics[&rule_id]["matched_targets"],
        json!(0)
    );
    Ok(())
}

#[tokio::test]
async fn activate_seeded_draft_rules_blocks_only_rule_with_preview_error() -> Result<()> {
    let relational = relational().await?;
    persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        seeded_taxonomy_with_artefact_target("command_dispatcher"),
    )
    .await?;
    relational
        .exec(
            "UPDATE architecture_role_detection_rules \
             SET positive_conditions_json = '[{\"kind\":\"signature\",\"key\":\"contains\",\"op\":\"eq\",\"value\":\"Result\",\"score\":1.0}]';",
        )
        .await?;

    let activation = activate_seeded_draft_rules(
        &relational,
        &PreviewGateway::empty(),
        "repo-1",
        "local_agent",
        cli_provenance("seed_activate_rules"),
    )
    .await?;

    assert_eq!(activation.seed_owned_draft_rules, 1);
    assert_eq!(activation.proposals_created, 0);
    assert_eq!(activation.blocked_rule_ids.len(), 1);
    let rule_id = activation.blocked_rule_ids[0].clone();
    assert_eq!(
        activation.blocked_reasons[&rule_id],
        vec!["preview_error".to_string()]
    );
    assert!(
        activation.blocked_diagnostics[&rule_id]["error"]
            .as_str()
            .expect("preview error")
            .contains("non-signature positive evidence")
    );
    Ok(())
}

#[tokio::test]
async fn seed_rule_activation_applies_matching_file_target_rule() -> Result<()> {
    let relational = relational().await?;
    persist_seeded_taxonomy(
        &relational,
        "repo-1",
        "local_agent",
        seeded_taxonomy_with_file_target("platform_bootstrapper"),
    )
    .await?;

    let files = vec![CurrentCanonicalFileRecord {
        repo_id: "repo-1".to_string(),
        path: "src/main.rs".to_string(),
        analysis_mode: "code".to_string(),
        file_role: "source_code".to_string(),
        language: "rust".to_string(),
        resolved_language: "rust".to_string(),
        effective_content_id: "content-main".to_string(),
        parser_version: "parser".to_string(),
        extractor_version: "extractor".to_string(),
        exists_in_head: true,
        exists_in_index: true,
        exists_in_worktree: true,
    }];
    let current_state = SliceArchitectureRoleCurrentStateSource::new(&[], &[]);
    let affected_paths = BTreeSet::from(["src/main.rs".to_string()]);
    let extraction = extract_architecture_role_facts(
        ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 1,
            affected_paths: &affected_paths,
            files: &files,
        },
        &current_state,
    )?;
    let facts = extraction.facts;
    let fact_paths = vec!["src/main.rs".to_string()];
    crate::capability_packs::architecture_graph::roles::storage::replace_role_classification_state(
        &relational,
        crate::capability_packs::architecture_graph::roles::storage::RoleClassificationStateReplacement {
            repo_id: "repo-1",
            fact_and_signal_paths: &fact_paths,
            facts: &facts,
            signals: &[],
            assignment_paths: &[],
            assignments: &[],
            assignment_history_writes: &[],
            removed_assignment_paths: &[],
            generation_seq: 1,
        },
    )
    .await?;

    let activation = activate_seeded_draft_rules(
        &relational,
        &PreviewGateway::empty(),
        "repo-1",
        "local_agent",
        cli_provenance("seed_activate_rules"),
    )
    .await?;

    assert_eq!(activation.seed_owned_draft_rules, 1);
    assert_eq!(activation.proposals_created, 1);
    assert_eq!(activation.proposals_applied, 1);
    assert_eq!(activation.activated_rule_ids.len(), 1);
    assert!(activation.blocked_rule_ids.is_empty());
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
        seeded_taxonomy_with_artefact_target("command_dispatcher"),
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
        &PreviewGateway::with_cli_artefact(),
        "repo-1",
        "local_agent",
        cli_provenance("seed_activate_rules"),
    )
    .await?;

    assert_eq!(activation.seed_owned_draft_rules, 1);
    assert_eq!(activation.proposals_created, 1);
    assert_eq!(activation.proposals_applied, 1);
    assert_eq!(activation.activated_rule_ids.len(), 1);
    assert!(activation.blocked_rule_ids.is_empty());

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
        seeded_taxonomy_with_artefact_target("command_dispatcher"),
    )
    .await?;

    let first = activate_seeded_draft_rules(
        &relational,
        &PreviewGateway::with_cli_artefact(),
        "repo-1",
        "local_agent",
        cli_provenance("seed_activate_rules"),
    )
    .await?;
    let second = activate_seeded_draft_rules(
        &relational,
        &PreviewGateway::with_cli_artefact(),
        "repo-1",
        "local_agent",
        cli_provenance("seed_activate_rules"),
    )
    .await?;

    assert_eq!(first.seed_owned_draft_rules, 1);
    assert_eq!(first.proposals_applied, 1);
    assert!(first.blocked_rule_ids.is_empty());
    assert_eq!(second.seed_owned_draft_rules, 0);
    assert_eq!(second.proposals_applied, 0);
    assert!(second.activated_rule_ids.is_empty());
    assert!(second.blocked_rule_ids.is_empty());
    Ok(())
}

#[tokio::test]
async fn seed_activation_enables_full_classification() -> Result<()> {
    let relational = relational().await?;
    let mut taxonomy = seeded_taxonomy("command_dispatcher");
    taxonomy.rule_candidates[0].candidate_selector = RoleRuleCandidateSelector {
        target_kinds: vec![TargetKind::Artefact],
        path_prefixes: vec!["src/cli".to_string()],
        ..Default::default()
    };
    taxonomy.rule_candidates[0].score = RoleRuleScore {
        base_confidence: Some(0.95),
        priority_hint: Some(100),
        min_positive_ratio: Some(1.0),
    };
    taxonomy.rule_candidates[0].positive_conditions = vec![RoleRuleCondition {
        kind: "path_contains".to_string(),
        value: json!("commands"),
        ..Default::default()
    }];

    persist_seeded_taxonomy(&relational, "repo-1", "local_agent", taxonomy).await?;
    activate_seeded_draft_rules(
        &relational,
        &PreviewGateway::with_cli_artefact(),
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
    let artefacts = vec![CurrentCanonicalArtefactRecord {
        repo_id: "repo-1".to_string(),
        path: "src/cli/commands/run.rs".to_string(),
        content_id: "content-1".to_string(),
        symbol_id: "symbol-1".to_string(),
        artefact_id: "artefact-1".to_string(),
        language: "rust".to_string(),
        extraction_fingerprint: "fingerprint".to_string(),
        canonical_kind: Some("function".to_string()),
        language_kind: Some("function".to_string()),
        symbol_fqn: Some("crate::cli::commands::run".to_string()),
        parent_symbol_id: None,
        parent_artefact_id: None,
        start_line: 1,
        end_line: 10,
        start_byte: 0,
        end_byte: 50,
        signature: Some("fn run()".to_string()),
        modifiers: "[]".to_string(),
        docstring: None,
    }];
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
