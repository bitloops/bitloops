use super::*;
use crate::capability_packs::architecture_graph::roles::taxonomy::supported_fact_predicates;
use crate::host::inference::StructuredGenerationService;
use std::collections::VecDeque;
use std::sync::Mutex;

struct QueuedStructuredGenerationService {
    responses: Mutex<VecDeque<Value>>,
    prompts: Mutex<Vec<String>>,
}

impl QueuedStructuredGenerationService {
    fn new(responses: Vec<Value>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            prompts: Mutex::new(Vec::new()),
        }
    }

    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().expect("prompts lock").clone()
    }
}

impl StructuredGenerationService for QueuedStructuredGenerationService {
    fn descriptor(&self) -> String {
        "test:queued".to_string()
    }

    fn generate(&self, request: StructuredGenerationRequest) -> Result<Value> {
        self.prompts
            .lock()
            .expect("prompts lock")
            .push(request.user_prompt);
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("no queued response"))
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

fn contract_supports_predicate(
    contract: &serde_json::Map<String, Value>,
    predicate_id: &str,
) -> bool {
    contract
        .get("supported_predicates")
        .and_then(Value::as_array)
        .map(|predicates| {
            predicates
                .iter()
                .any(|predicate| predicate.get("id").and_then(Value::as_str) == Some(predicate_id))
        })
        .unwrap_or(false)
}

#[test]
fn seed_prompt_mentions_project_specific_inference() {
    let prompt = architecture_roles_seed_user_prompt(&test_scope(), &json!({"files": []}));
    assert!(prompt.contains("project-specific architecture role taxonomy"));
    assert!(prompt.contains("Generic role families are examples only"));
}

#[test]
fn seed_prompt_includes_rule_authoring_contract_visible_to_llm() {
    let prompt =
        architecture_roles_seed_user_prompt(&test_scope(), &json!({"canonical_files": []}));
    let value: Value = serde_json::from_str(&prompt).expect("prompt is JSON");

    let contract = value
        .get("rule_authoring_contract")
        .and_then(Value::as_object)
        .expect("prompt includes rule_authoring_contract");

    assert_eq!(
        contract.get("contract_version").and_then(Value::as_str),
        Some("fact-backed-rule-v2")
    );
    assert!(contract_supports_predicate(contract, "path.full:prefix"));
    assert!(contract_supports_predicate(
        contract,
        "artefact.canonical_kind:eq"
    ));
    assert!(
        contract
            .get("target_kinds")
            .and_then(Value::as_array)
            .expect("prompt includes target kinds")
            .iter()
            .any(|kind| kind.as_str() == Some("file"))
    );
    assert!(contract_supports_predicate(
        contract,
        "dependency.outgoing_count:gte"
    ));

    let examples = contract
        .get("rule_candidate_examples")
        .and_then(Value::as_array)
        .expect("prompt includes rule examples");
    assert!(examples.iter().any(|example| {
        example
            .get("positive_conditions")
            .and_then(Value::as_array)
            .map(|conditions| {
                conditions.iter().any(|condition| {
                    condition.get("predicate").and_then(Value::as_str) == Some("path.full:contains")
                })
            })
            .unwrap_or(false)
    }));
}

#[test]
fn role_discovery_prompt_explains_fact_synthesis_and_workspace_use() {
    let prompt =
        architecture_roles_seed_roles_user_prompt(&test_scope(), &json!({"canonical_files": []}));
    let value: Value = serde_json::from_str(&prompt).expect("prompt is JSON");

    assert_eq!(
        value
            .pointer("/fact_synthesis_context/slot_name")
            .and_then(Value::as_str),
        Some("fact_synthesis")
    );
    assert_eq!(
        value
            .pointer("/agentic_code_exploration/write_policy")
            .and_then(Value::as_str),
        Some("read_only")
    );
    assert!(
        value
            .pointer("/db_evidence_guide/sections")
            .and_then(Value::as_array)
            .expect("sections")
            .iter()
            .any(|section| section.get("name").and_then(Value::as_str)
                == Some("canonical_artefacts"))
    );
}

#[test]
fn rule_generation_prompt_maps_db_facts_to_supported_conditions() {
    let roles = vec![SeededArchitectureRole {
        canonical_key: "cli_surface".to_string(),
        display_name: "CLI Surface".to_string(),
        description: "Command handlers.".to_string(),
        family: Some("entrypoint".to_string()),
        lifecycle_status: Some("active".to_string()),
        provenance: json!({}),
        evidence: json!({}),
    }];
    let prompt = architecture_roles_seed_rules_user_prompt(
        &test_scope(),
        &json!({"canonical_files": []}),
        &roles,
    );
    let value: Value = serde_json::from_str(&prompt).expect("prompt is JSON");

    let contract = value
        .get("rule_authoring_contract")
        .and_then(Value::as_object)
        .expect("prompt includes rule_authoring_contract");
    assert!(contract_supports_predicate(
        contract,
        "artefact.canonical_kind:eq"
    ));
    assert!(contract_supports_predicate(
        contract,
        "dependency.outgoing_count:gte"
    ));
    assert!(contract_supports_predicate(contract, "file.role:eq"));
    assert!(
            value
                .get("rules")
                .and_then(Value::as_array)
                .expect("prompt includes rules")
                .iter()
                .any(|rule| {
                    rule.as_str()
                        == Some("Use dependency conditions only when dependency facts are present in evidence.")
                })
        );
}

#[test]
fn architecture_roles_rule_generation_prompt_uses_supported_predicates() {
    let roles = vec![SeededArchitectureRole {
        canonical_key: "cli_surface".to_string(),
        display_name: "CLI Surface".to_string(),
        description: "Command handlers.".to_string(),
        family: Some("entrypoint".to_string()),
        lifecycle_status: Some("active".to_string()),
        provenance: json!({}),
        evidence: json!({}),
    }];
    let prompt = architecture_roles_seed_rules_user_prompt(
        &test_scope(),
        &json!({"canonical_files": []}),
        &roles,
    );
    let value: Value = serde_json::from_str(&prompt).expect("prompt is JSON");
    let contract = value
        .get("rule_authoring_contract")
        .and_then(Value::as_object)
        .expect("contract");
    let rendered = serde_json::to_string(&value).expect("render prompt");

    assert!(contract.get("supported_predicates").is_some());
    assert!(contract.get("supported_facts").is_none());
    assert!(rendered.contains("signature.contains:eq"));
    assert!(rendered.contains("Do not use natural language operators such as `contains`"));
    assert!(rendered.contains("\"predicate\":\"signature.contains:eq\""));
}

#[test]
fn architecture_roles_rule_generation_retry_prompt_is_compact() {
    let roles = vec![SeededArchitectureRole {
        canonical_key: "cli_surface".to_string(),
        display_name: "CLI Surface".to_string(),
        description: "Command handlers.".to_string(),
        family: Some("entrypoint".to_string()),
        lifecycle_status: Some("active".to_string()),
        provenance: json!({}),
        evidence: json!({}),
    }];
    let issues = vec![SeedRuleCandidateValidationIssue {
        candidate_index: 7,
        role_slug: Some("cli_surface".to_string()),
        candidate_slug: None,
        field_path: "positive_conditions[0]".to_string(),
        reason: "unsupported predicate `signature.contains:contains`".to_string(),
        raw_candidate_excerpt: "{\"target_role_key\":\"cli_surface\"}".to_string(),
    }];

    let request = architecture_roles_seed_rule_candidates_retry_request(
        &test_scope(),
        2,
        &roles,
        &supported_fact_predicates(),
        &issues,
    );
    let prompt: Value = serde_json::from_str(&request.user_prompt).expect("retry prompt JSON");

    assert!(request.user_prompt.len() < 16 * 1024);
    assert!(prompt.get("evidence").is_none());
    assert_eq!(
        prompt.get("original_batch_index").and_then(Value::as_u64),
        Some(2)
    );
    assert!(request.user_prompt.contains("replacements only"));
    assert!(request.user_prompt.contains("positive_conditions[0]"));
}

#[test]
fn architecture_roles_rule_generation_retry_uses_predicate_schema() {
    let request = architecture_roles_seed_rule_candidates_retry_request(
        &test_scope(),
        0,
        &[],
        &supported_fact_predicates(),
        &[],
    );
    let properties = request
        .json_schema
        .pointer(
            "/properties/rule_candidates/items/properties/positive_conditions/items/properties",
        )
        .and_then(Value::as_object)
        .expect("condition properties");

    assert!(properties.contains_key("predicate"));
    assert!(!properties.contains_key("kind"));
    assert!(!properties.contains_key("key"));
    assert!(!properties.contains_key("op"));
}

#[test]
fn role_discovery_prompt_requires_generic_durable_roles_with_evidence() {
    let prompt =
        architecture_roles_seed_roles_user_prompt(&test_scope(), &json!({"canonical_files": []}));
    let value: Value = serde_json::from_str(&prompt).expect("prompt is JSON");
    let rules = value.get("rules").and_then(Value::as_array).expect("rules");
    let rendered_rules = rules
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered_rules.contains("Do not create repository-specific prompt branches"));
    assert!(rendered_rules.contains("Inspect source code through workspace_path"));
    assert!(rendered_rules.contains("Populate each role evidence object"));
    assert!(
        rendered_rules.contains("Prefer fewer durable roles over many weak or redundant roles")
    );
}

#[test]
fn decode_seeded_taxonomy_response_rejects_invalid_payload() {
    let err = decode_seeded_taxonomy_response(json!({
        "roles": [],
        "rule_candidates": []
    }))
    .expect_err("expected empty roles to fail");
    assert!(err.to_string().contains("did not include any roles"));
}

#[test]
fn run_seed_generation_validates_structured_response() {
    let service = QueuedStructuredGenerationService::new(vec![
        json!({
            "roles": [{
                "canonical_key": "command_dispatcher",
                "display_name": "Command Dispatcher"
            }]
        }),
        json!({
            "rule_candidates": [{
                "target_role_key": "command_dispatcher",
                "candidate_selector": {
                    "path_prefixes": ["src/cli"]
                }
            }]
        }),
    ]);
    let taxonomy = run_seed_generation(&service, &test_scope(), &json!({"canonical_files": []}))
        .expect("valid taxonomy");
    assert_eq!(taxonomy.roles.len(), 1);
    assert_eq!(taxonomy.rule_candidates.len(), 1);
}

#[test]
fn role_discovery_rejects_stable_lifecycle_before_rule_generation() {
    let err = decode_seeded_role_discovery_response(json!({
        "roles": [{
            "canonical_key": "command_dispatcher",
            "display_name": "Command Dispatcher",
            "description": "Routes CLI commands.",
            "family": "entrypoint",
            "lifecycle_status": "stable",
            "provenance": {},
            "evidence": {}
        }]
    }))
    .expect_err("stable lifecycle should fail during role discovery decode");

    assert!(
        err.to_string()
            .contains("unsupported seeded role lifecycle_status `stable`")
    );
}

#[test]
fn seed_generation_runs_role_discovery_before_rule_generation() {
    let service = QueuedStructuredGenerationService::new(vec![
        json!({
            "roles": [
                {
                    "canonical_key": "cli_surface",
                    "display_name": "CLI Surface",
                    "description": "Command handlers.",
                    "family": "entrypoint",
                    "lifecycle_status": "active",
                    "provenance": {},
                    "evidence": {
                        "inspected_paths": ["bitloops/src/cli"],
                        "supporting_paths": ["bitloops/src/cli/commands"],
                        "supporting_symbols": [],
                        "db_sections_used": ["canonical_files", "canonical_artefacts"],
                        "reasoning_summary": "Command handlers form a stable entrypoint surface.",
                        "confidence_reason": "Paths and command naming are stable.",
                        "uncertainty": ""
                    }
                }
            ]
        }),
        json!({
            "rule_candidates": [
                {
                    "target_role_key": "cli_surface",
                    "candidate_selector": {
                        "path_prefixes": ["bitloops/src/cli"],
                        "path_suffixes": [".rs"],
                        "path_contains": [],
                        "languages": ["rust"],
                        "canonical_kinds": ["function"],
                        "symbol_fqn_contains": []
                    },
                    "positive_conditions": [
                        { "kind": "path_prefix", "value": "bitloops/src/cli" }
                    ],
                    "negative_conditions": [],
                    "score": {
                        "base_confidence": 0.8,
                        "priority_hint": 100,
                        "min_positive_ratio": 1.0
                    },
                    "evidence": {
                        "inspected_paths": ["bitloops/src/cli/commands/run.rs"],
                        "positive_examples": [
                            {
                                "path": "bitloops/src/cli/commands/run.rs",
                                "symbol_fqn": "crate::cli::commands::run",
                                "canonical_kind": "function",
                                "why": "Command path and function symbol match the CLI surface role."
                            }
                        ],
                        "negative_examples": [],
                        "db_sections_used": ["canonical_files", "canonical_artefacts"],
                        "reasoning_summary": "The path prefix and Rust language constrain the rule.",
                        "confidence_reason": "The rule is based on stable path structure.",
                        "uncertainty": ""
                    },
                    "metadata": {}
                }
            ]
        }),
    ]);

    let taxonomy = run_seed_generation(
        &service,
        &test_scope(),
        &json!({
            "canonical_files": [],
            "canonical_artefacts": [],
            "artefact_summaries": [],
            "dependency_graph_hints": [],
            "existing_architecture_graph_facts": []
        }),
    )
    .expect("phased seed generation");

    assert_eq!(taxonomy.roles.len(), 1);
    assert_eq!(taxonomy.rule_candidates.len(), 1);

    let prompts = service.prompts();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[0].contains("Infer repository-specific architecture roles"));
    assert!(prompts[1].contains("Generate deterministic rule candidates"));
    assert!(prompts[1].contains("cli_surface"));
}

#[test]
fn seed_evidence_text_is_truncated_with_omission_marker() {
    let input = "a".repeat(MAX_SEED_TEXT_CHARS + 20);
    let truncated = truncate_seed_evidence_text(&input, MAX_SEED_TEXT_CHARS);

    assert!(truncated.len() <= MAX_SEED_TEXT_CHARS + "…[truncated]".len());
    assert!(truncated.ends_with("…[truncated]"));
}

#[test]
fn seed_evidence_text_keeps_short_values_unchanged() {
    let input = "short docstring";
    assert_eq!(
        truncate_seed_evidence_text(input, MAX_SEED_TEXT_CHARS),
        "short docstring"
    );
}
