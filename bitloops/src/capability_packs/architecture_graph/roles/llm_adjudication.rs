use anyhow::{Context, Result};
use serde_json::{Map, Value, json};

use crate::devql_transport::SlimCliRepoScope;
use crate::host::capability_host::CurrentStateConsumerContext;
use crate::host::inference::StructuredGenerationRequest;
#[cfg(test)]
use crate::host::inference::StructuredGenerationService;

#[cfg(test)]
use super::taxonomy::architecture_roles_seed_schema;
use super::taxonomy::{
    SeededArchitectureRole, SeededArchitectureRoleDiscovery, SeededArchitectureRuleCandidate,
    SeededArchitectureRuleCandidates, SeededArchitectureTaxonomy,
    architecture_roles_seed_roles_schema, architecture_roles_seed_rule_candidates_schema,
    generic_role_family_examples, role_rule_candidate_examples, supported_rule_fact_catalog,
    validate_seeded_roles, validate_seeded_taxonomy,
};

const MAX_FILE_EVIDENCE: usize = 120;
const MAX_ARTEFACT_EVIDENCE: usize = 200;
const MAX_EDGE_EVIDENCE: usize = 200;
const MAX_GRAPH_EVIDENCE: usize = 120;
const MAX_SEED_TEXT_CHARS: usize = 800;
pub(crate) const SEED_RULE_ROLE_BATCH_SIZE: usize = 3;
const TRUNCATED_MARKER: &str = "…[truncated]";

pub(crate) async fn collect_seed_evidence(
    scope: &SlimCliRepoScope,
    context: &CurrentStateConsumerContext,
) -> Result<Value> {
    let files = context
        .relational
        .load_current_canonical_files(&scope.repo.repo_id)
        .context("loading current canonical files for architecture role seed")?;
    let artefacts = context
        .relational
        .load_current_canonical_artefacts(&scope.repo.repo_id)
        .context("loading current canonical artefacts for architecture role seed")?;
    let dependency_edges = context
        .relational
        .load_current_canonical_edges(&scope.repo.repo_id)
        .context("loading current canonical dependency edges for architecture role seed")?;

    let repo_metadata = load_repository_metadata(context, &scope.repo.repo_id).await?;
    let file_contexts = load_file_context_evidence(context, &scope.repo.repo_id).await?;
    let architecture_graph = load_architecture_graph_evidence(context, &scope.repo.repo_id).await?;
    let semantic_summaries = load_semantic_summary_evidence(context, &scope.repo.repo_id).await?;

    Ok(json!({
        "repository": {
            "repo_id": scope.repo.repo_id,
            "provider": scope.repo.provider,
            "organization": scope.repo.organization,
            "name": scope.repo.name,
            "identity": scope.repo.identity,
            "repo_root": scope.repo_root.display().to_string(),
            "branch_name": scope.branch_name,
            "project_path": scope.project_path,
            "metadata": repo_metadata,
        },
        "language_framework_signals": file_contexts,
        "canonical_files": files
            .iter()
            .take(MAX_FILE_EVIDENCE)
            .map(|file| json!({
                "path": file.path,
                "analysis_mode": file.analysis_mode,
                "file_role": file.file_role,
                "language": file.language,
                "resolved_language": file.resolved_language,
            }))
            .collect::<Vec<_>>(),
        "canonical_artefacts": artefacts
            .iter()
            .take(MAX_ARTEFACT_EVIDENCE)
            .map(|artefact| json!({
                "artefact_id": artefact.artefact_id,
                "path": artefact.path,
                "language": artefact.language,
                "canonical_kind": artefact.canonical_kind,
                "language_kind": artefact.language_kind,
                "symbol_fqn": artefact
                    .symbol_fqn
                    .as_ref()
                    .map(|value| truncate_seed_evidence_text(value, MAX_SEED_TEXT_CHARS)),
                "signature": artefact
                    .signature
                    .as_ref()
                    .map(|value| truncate_seed_evidence_text(value, MAX_SEED_TEXT_CHARS)),
                "docstring": artefact
                    .docstring
                    .as_ref()
                    .map(|value| truncate_seed_evidence_text(value, MAX_SEED_TEXT_CHARS)),
            }))
            .collect::<Vec<_>>(),
        "artefact_summaries": semantic_summaries,
        "dependency_graph_hints": dependency_edges
            .iter()
            .take(MAX_EDGE_EVIDENCE)
            .map(|edge| json!({
                "edge_id": edge.edge_id,
                "path": edge.path,
                "from_artefact_id": edge.from_artefact_id,
                "to_artefact_id": edge.to_artefact_id,
                "to_symbol_ref": edge.to_symbol_ref,
                "edge_kind": edge.edge_kind,
                "language": edge.language,
            }))
            .collect::<Vec<_>>(),
        "existing_architecture_graph_facts": architecture_graph
            .into_iter()
            .take(MAX_GRAPH_EVIDENCE)
            .collect::<Vec<_>>(),
        "generic_role_family_examples": generic_role_family_examples(),
    }))
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn architecture_roles_seed_system_prompt() -> &'static str {
    "You infer repository-specific architectural role taxonomies. Return JSON only that matches the supplied schema. \
Do not hardcode Bitloops-specific roles. Use the supplied repository evidence to propose role identities and reviewable deterministic rule candidates for this repository. \
Rule candidates must use only facts and operators listed in rule_authoring_contract.supported_facts."
}

pub(crate) fn architecture_roles_seed_roles_system_prompt() -> &'static str {
    "You infer repository-specific architectural roles. Return JSON only that matches the supplied schema."
}

pub(crate) fn architecture_roles_seed_rules_system_prompt() -> &'static str {
    "You infer deterministic architecture role matching rules for known roles. Return JSON only that matches the supplied schema."
}

fn fact_synthesis_context(seed_phase: &'static str) -> Value {
    json!({
        "capability_id": "architecture_graph",
        "slot_name": "fact_synthesis",
        "seed_phase": seed_phase,
        "purpose": "Use repository evidence and read-only code exploration to synthesize architecture roles and deterministic rule candidates.",
        "source_of_truth_order": [
            "source code inspected through workspace_path",
            "canonical DB evidence supplied in this prompt",
            "generic role family examples"
        ]
    })
}

fn agentic_code_exploration_contract() -> Value {
    json!({
        "write_policy": "read_only",
        "expected_behavior": [
            "Inspect the repository through workspace_path before finalizing role or rule output.",
            "Use supplied DB evidence as an index for which paths, symbols, summaries, and graph facts to inspect first.",
            "Prefer source code when DB evidence is incomplete, stale, or ambiguous.",
            "Do not edit files, create files, run migrations, or modify repository state.",
            "Return only JSON matching the supplied schema."
        ],
        "inspection_priorities": [
            "files and symbols referenced by existing_architecture_graph_facts",
            "canonical_artefacts with stable paths and symbol_fqn values",
            "canonical_files with strong language, file_role, or analysis_mode signals",
            "artefact_summaries that describe durable responsibilities",
            "dependency_graph_hints when they clarify boundaries between roles"
        ]
    })
}

fn db_evidence_guide() -> Value {
    json!({
        "interpretation": "The evidence object is a DB snapshot and may be partial. Use it as an index into the codebase, not as a complete replacement for code inspection.",
        "sections": [
            {
                "name": "repository",
                "use_for": "Repo identity, root path, branch, and metadata."
            },
            {
                "name": "language_framework_signals",
                "use_for": "Framework, runtime, and context hints for files."
            },
            {
                "name": "canonical_files",
                "use_for": "Stable repository-relative paths, language, file_role, and analysis_mode."
            },
            {
                "name": "canonical_artefacts",
                "use_for": "Symbols, canonical_kind, language_kind, signatures, and docstrings."
            },
            {
                "name": "artefact_summaries",
                "use_for": "Semantic summaries that help identify responsibilities."
            },
            {
                "name": "dependency_graph_hints",
                "use_for": "Call/import/reference hints. Use only to support a rule when source paths or symbols also support it."
            },
            {
                "name": "existing_architecture_graph_facts",
                "use_for": "Previously synthesized graph nodes ordered by confidence. Treat as hints, not guaranteed truth."
            },
            {
                "name": "evidence_budget",
                "use_for": "Counts showing how much evidence was included or omitted from the prompt."
            }
        ]
    })
}

#[cfg(test)]
pub(crate) fn architecture_roles_seed_user_prompt(
    scope: &SlimCliRepoScope,
    evidence: &Value,
) -> String {
    json!({
        "task": "Infer a project-specific architecture role taxonomy and candidate deterministic matching rules for this repository.",
        "rules": [
            "Return repository-specific roles, not a hardcoded generic taxonomy.",
            "Generic role families are examples only; adapt them to the repository evidence.",
            "Return only durable role identities that are justified by the repository evidence.",
            "Do not include lifecycle state; newly inferred roles are activated by Bitloops after validation.",
            "Detection rules must be reviewable and safe for deterministic use.",
            "Use only fact-backed conditions from rule_authoring_contract.supported_facts.",
            "Do not invent additional fact keys, operators, aliases, or candidate_selector fields.",
            "Use rule_authoring_contract.rule_candidate_examples as shape examples only; adapt role keys, paths, languages, kinds, and symbols to the repository evidence.",
            "Prefer multi-signal rules over path-only rules and include target kinds whenever possible.",
            "Prefer fewer strong roles over many weak or redundant roles."
        ],
        "repository_identity": {
            "repo_id": scope.repo.repo_id,
            "provider": scope.repo.provider,
            "organization": scope.repo.organization,
            "name": scope.repo.name,
            "identity": scope.repo.identity,
            "branch_name": scope.branch_name,
        },
        "rule_authoring_contract": {
            "contract_version": "fact-backed-rule-v2",
            "supported_facts": supported_rule_fact_catalog(),
            "target_kinds": ["file", "artefact", "symbol"],
            "ops": ["eq", "contains", "prefix", "suffix", "gte", "lte"],
            "scoring": {
                "base_confidence": "Maximum confidence for this rule when positive evidence is fully satisfied.",
                "condition_score": "Relative contribution within the rule; normalized at evaluation time.",
                "min_positive_ratio": "Minimum normalized positive evidence required before emitting a signal."
            },
            "rule_candidate_examples": role_rule_candidate_examples(),
        },
        "evidence": evidence,
    })
    .to_string()
}

pub(crate) fn architecture_roles_seed_roles_user_prompt(
    scope: &SlimCliRepoScope,
    evidence: &Value,
) -> String {
    json!({
        "task": "Infer repository-specific architecture roles for this repository.",
        "rules": [
            "Return repository-specific architecture roles, but keep this prompt generic. Do not create repository-specific prompt branches.",
            "Inspect source code through workspace_path before finalizing roles. Use the supplied DB evidence as an index for where to look first.",
            "Generic role families are examples only. Adapt them to the repository evidence and inspected source code.",
            "Return only durable role identities that are justified by multiple evidence signals or a very strong single structural signal.",
            "Do not include lifecycle state; newly inferred roles are activated by Bitloops after validation.",
            "Prefer fewer durable roles over many weak or redundant roles.",
            "A role should describe a stable architectural responsibility, not one file, one temporary workflow, or one naming accident.",
            "If a likely responsibility cannot be supported by deterministic rules in the next phase, keep it only when the codebase makes the role clearly important.",
            "Populate each role evidence object with inspected paths, supporting paths, supporting symbols, DB sections used, a concise reasoning summary, confidence reason, and uncertainty."
        ],
        "repository_identity": {
            "repo_id": scope.repo.repo_id,
            "provider": scope.repo.provider,
            "organization": scope.repo.organization,
            "name": scope.repo.name,
            "identity": scope.repo.identity,
            "branch_name": scope.branch_name,
        },
        "fact_synthesis_context": fact_synthesis_context("roles"),
        "agentic_code_exploration": agentic_code_exploration_contract(),
        "db_evidence_guide": db_evidence_guide(),
        "evidence": evidence,
    })
    .to_string()
}

pub(crate) fn architecture_roles_seed_rules_user_prompt(
    scope: &SlimCliRepoScope,
    evidence: &Value,
    roles: &[SeededArchitectureRole],
) -> String {
    json!({
        "task": "Generate deterministic rule candidates for the supplied architecture roles.",
        "rules": [
            "Generate deterministic rule candidates for the supplied known roles only.",
            "Inspect source code through workspace_path before finalizing rules. Use the supplied DB evidence as an index for where to look first.",
            "Detection rules must be reviewable and safe for deterministic use.",
            "A good rule should match a reusable architectural pattern, not only one currently visible artefact.",
            "Use only fact-backed conditions from rule_authoring_contract.supported_facts.",
            "Do not invent additional fact keys, operators, aliases, or candidate_selector fields.",
            "Prefer multi-signal rules over path-only rules.",
            "Include target kinds whenever possible.",
            "Use dependency conditions only when dependency facts are present in evidence.",
            "Use rule_authoring_contract.rule_candidate_examples as shape examples only. Adapt paths, languages, kinds, and symbols to repository evidence and inspected source code.",
            "Populate each rule evidence object with inspected paths, positive examples, negative examples when known, DB sections used, a concise reasoning summary, confidence reason, and uncertainty.",
            "Return zero rule candidates for a role when evidence does not support a stable deterministic rule."
        ],
        "repository_identity": {
            "repo_id": scope.repo.repo_id,
            "provider": scope.repo.provider,
            "organization": scope.repo.organization,
            "name": scope.repo.name,
            "identity": scope.repo.identity,
            "branch_name": scope.branch_name,
        },
        "known_roles": roles,
        "fact_synthesis_context": fact_synthesis_context("rules"),
        "agentic_code_exploration": agentic_code_exploration_contract(),
        "db_evidence_guide": db_evidence_guide(),
        "rule_authoring_contract": {
            "contract_version": "fact-backed-rule-v2",
            "supported_facts": supported_rule_fact_catalog(),
            "target_kinds": ["file", "artefact", "symbol"],
            "ops": ["eq", "contains", "prefix", "suffix", "gte", "lte"],
            "scoring": {
                "base_confidence": "Maximum confidence for this rule when positive evidence is fully satisfied.",
                "condition_score": "Relative contribution within the rule; normalized at evaluation time.",
                "min_positive_ratio": "Minimum normalized positive evidence required before emitting a signal."
            },
            "rule_candidate_examples": role_rule_candidate_examples(),
        },
        "evidence": evidence,
    })
    .to_string()
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn architecture_roles_seed_request(
    scope: &SlimCliRepoScope,
    evidence: &Value,
) -> StructuredGenerationRequest {
    let mut metadata = Map::new();
    metadata.insert(
        "capability_id".to_string(),
        Value::String("architecture_graph".to_string()),
    );
    metadata.insert(
        "slot_name".to_string(),
        Value::String("fact_synthesis".to_string()),
    );
    metadata.insert(
        "repo_id".to_string(),
        Value::String(scope.repo.repo_id.clone()),
    );

    StructuredGenerationRequest {
        system_prompt: architecture_roles_seed_system_prompt().to_string(),
        user_prompt: architecture_roles_seed_user_prompt(scope, evidence),
        json_schema: architecture_roles_seed_schema(),
        workspace_path: Some(scope.repo_root.display().to_string()),
        metadata,
    }
}

pub(crate) fn architecture_roles_seed_roles_request(
    scope: &SlimCliRepoScope,
    evidence: &Value,
) -> StructuredGenerationRequest {
    let mut metadata = Map::new();
    metadata.insert(
        "capability_id".to_string(),
        Value::String("architecture_graph".to_string()),
    );
    metadata.insert(
        "slot_name".to_string(),
        Value::String("fact_synthesis".to_string()),
    );
    metadata.insert("seed_phase".to_string(), Value::String("roles".to_string()));
    metadata.insert(
        "repo_id".to_string(),
        Value::String(scope.repo.repo_id.clone()),
    );

    StructuredGenerationRequest {
        system_prompt: architecture_roles_seed_roles_system_prompt().to_string(),
        user_prompt: architecture_roles_seed_roles_user_prompt(scope, evidence),
        json_schema: architecture_roles_seed_roles_schema(),
        workspace_path: Some(scope.repo_root.display().to_string()),
        metadata,
    }
}

pub(crate) fn architecture_roles_seed_rule_candidates_request(
    scope: &SlimCliRepoScope,
    evidence: &Value,
    roles: &[SeededArchitectureRole],
    batch_index: usize,
) -> StructuredGenerationRequest {
    let mut metadata = Map::new();
    metadata.insert(
        "capability_id".to_string(),
        Value::String("architecture_graph".to_string()),
    );
    metadata.insert(
        "slot_name".to_string(),
        Value::String("fact_synthesis".to_string()),
    );
    metadata.insert("seed_phase".to_string(), Value::String("rules".to_string()));
    metadata.insert(
        "seed_rule_batch_index".to_string(),
        Value::Number(serde_json::Number::from(batch_index)),
    );
    metadata.insert(
        "repo_id".to_string(),
        Value::String(scope.repo.repo_id.clone()),
    );

    StructuredGenerationRequest {
        system_prompt: architecture_roles_seed_rules_system_prompt().to_string(),
        user_prompt: architecture_roles_seed_rules_user_prompt(scope, evidence, roles),
        json_schema: architecture_roles_seed_rule_candidates_schema(),
        workspace_path: Some(scope.repo_root.display().to_string()),
        metadata,
    }
}

#[cfg(test)]
pub(crate) fn decode_seeded_taxonomy_response(value: Value) -> Result<SeededArchitectureTaxonomy> {
    let taxonomy: SeededArchitectureTaxonomy =
        serde_json::from_value(value).context("parse seeded architecture taxonomy")?;
    validate_seeded_taxonomy(&taxonomy)?;
    Ok(taxonomy)
}

pub(crate) fn decode_seeded_role_discovery_response(
    value: Value,
) -> Result<SeededArchitectureRoleDiscovery> {
    let discovery: SeededArchitectureRoleDiscovery =
        serde_json::from_value(value).context("parse seeded architecture role discovery")?;
    validate_seeded_roles(&discovery.roles)?;
    Ok(discovery)
}

pub(crate) fn decode_seeded_rule_candidates_response(
    value: Value,
) -> Result<SeededArchitectureRuleCandidates> {
    let candidates: SeededArchitectureRuleCandidates =
        serde_json::from_value(value).context("parse seeded architecture rule candidates")?;
    Ok(candidates)
}

pub(crate) fn combine_seeded_taxonomy(
    roles: Vec<SeededArchitectureRole>,
    rule_candidates: Vec<SeededArchitectureRuleCandidate>,
) -> Result<SeededArchitectureTaxonomy> {
    let taxonomy = SeededArchitectureTaxonomy {
        roles,
        rule_candidates,
    };
    validate_seeded_taxonomy(&taxonomy)?;
    Ok(taxonomy)
}

#[cfg(test)]
pub(crate) fn run_seed_generation(
    service: &dyn StructuredGenerationService,
    scope: &SlimCliRepoScope,
    evidence: &Value,
) -> Result<SeededArchitectureTaxonomy> {
    let role_response = service
        .generate(architecture_roles_seed_roles_request(scope, evidence))
        .context("generating architecture role discovery")?;
    let role_discovery = decode_seeded_role_discovery_response(role_response)?;

    let mut rule_candidates = Vec::new();
    for (batch_index, role_batch) in role_discovery
        .roles
        .chunks(SEED_RULE_ROLE_BATCH_SIZE)
        .enumerate()
    {
        let rule_response = service
            .generate(architecture_roles_seed_rule_candidates_request(
                scope,
                evidence,
                role_batch,
                batch_index,
            ))
            .with_context(|| {
                format!("generating architecture rule candidates batch {batch_index}")
            })?;
        let mut decoded = decode_seeded_rule_candidates_response(rule_response)?;
        rule_candidates.append(&mut decoded.rule_candidates);
    }

    combine_seeded_taxonomy(role_discovery.roles, rule_candidates)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn run_seed_generation_request(
    service: &dyn StructuredGenerationService,
    request: StructuredGenerationRequest,
) -> Result<SeededArchitectureTaxonomy> {
    let response = service.generate(request)?;
    decode_seeded_taxonomy_response(response)
}

fn truncate_seed_evidence_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str(TRUNCATED_MARKER);
    truncated
}

fn truncated_json_string(value: Option<&Value>, max_chars: usize) -> Value {
    match value.and_then(Value::as_str) {
        Some(text) if !text.is_empty() => json!(truncate_seed_evidence_text(text, max_chars)),
        _ => Value::Null,
    }
}

async fn load_repository_metadata(
    context: &CurrentStateConsumerContext,
    repo_id: &str,
) -> Result<Value> {
    let rows = context
        .storage
        .query_rows(&format!(
            "SELECT metadata_json FROM repositories WHERE repo_id = '{}' LIMIT 1;",
            crate::host::devql::esc_pg(repo_id)
        ))
        .await
        .context("loading repository metadata for architecture role seed")?;
    match rows.first().and_then(|row| row.get("metadata_json")) {
        Some(Value::String(text)) if !text.trim().is_empty() => {
            serde_json::from_str(text).context("parse repositories.metadata_json")
        }
        _ => Ok(json!({})),
    }
}

async fn load_file_context_evidence(
    context: &CurrentStateConsumerContext,
    repo_id: &str,
) -> Result<Vec<Value>> {
    let rows = context
        .storage
        .query_rows(&format!(
            "SELECT path, resolved_language, primary_context_id, secondary_context_ids_json, \
                    frameworks_json, runtime_profile, classification_reason \
             FROM current_file_state \
             WHERE repo_id = '{}' \
             ORDER BY path ASC \
             LIMIT {};",
            crate::host::devql::esc_pg(repo_id),
            MAX_FILE_EVIDENCE
        ))
        .await
        .context("loading file context evidence for architecture role seed")?;
    rows.into_iter()
        .map(|row| {
            Ok(json!({
                "path": row.get("path").and_then(Value::as_str).unwrap_or_default(),
                "resolved_language": row.get("resolved_language").and_then(Value::as_str).unwrap_or_default(),
                "primary_context_id": row.get("primary_context_id").and_then(Value::as_str),
                "secondary_context_ids": parse_json_array_field(row.get("secondary_context_ids_json"))?,
                "frameworks": parse_json_array_field(row.get("frameworks_json"))?,
                "runtime_profile": row.get("runtime_profile").and_then(Value::as_str),
                "classification_reason": row.get("classification_reason").and_then(Value::as_str),
            }))
        })
        .collect()
}

async fn load_architecture_graph_evidence(
    context: &CurrentStateConsumerContext,
    repo_id: &str,
) -> Result<Vec<Value>> {
    context
        .storage
        .query_rows(&format!(
            "SELECT node_kind, label, artefact_id, path, entry_kind, confidence \
             FROM architecture_graph_nodes_current \
             WHERE repo_id = '{}' \
             ORDER BY confidence DESC, label ASC \
             LIMIT {};",
            crate::host::devql::esc_pg(repo_id),
            MAX_GRAPH_EVIDENCE
        ))
        .await
        .context("loading architecture graph evidence for architecture role seed")
}

async fn load_semantic_summary_evidence(
    context: &CurrentStateConsumerContext,
    repo_id: &str,
) -> Result<Vec<Value>> {
    let rows = context
        .storage
        .query_rows(&format!(
            "SELECT a.artefact_id AS artefact_id, a.path AS path, \
                    COALESCE(sc.summary, sh.summary) AS summary \
             FROM artefacts_current a \
             LEFT JOIN symbol_semantics_current sc \
               ON sc.repo_id = a.repo_id \
              AND sc.artefact_id = a.artefact_id \
              AND sc.content_id = a.content_id \
             LEFT JOIN symbol_semantics sh \
               ON sh.repo_id = a.repo_id \
              AND sh.artefact_id = a.artefact_id \
              AND sh.blob_sha = a.content_id \
             WHERE a.repo_id = '{}' \
               AND COALESCE(sc.summary, sh.summary, '') <> '' \
             ORDER BY a.path ASC \
             LIMIT {};",
            crate::host::devql::esc_pg(repo_id),
            MAX_ARTEFACT_EVIDENCE
        ))
        .await
        .context("loading semantic summary evidence for architecture role seed")?;

    Ok(rows
        .into_iter()
        .map(|mut row| {
            if let Some(object) = row.as_object_mut() {
                let summary = truncated_json_string(object.get("summary"), MAX_SEED_TEXT_CHARS);
                object.insert("summary".to_string(), summary);
            }
            row
        })
        .collect())
}

fn parse_json_array_field(value: Option<&Value>) -> Result<Value> {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => {
            serde_json::from_str(text).context("parse JSON array field")
        }
        _ => Ok(json!([])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

        let allowed = contract
            .get("allowed_condition_kinds")
            .and_then(Value::as_array)
            .expect("prompt includes allowed condition kinds");
        assert!(
            allowed
                .iter()
                .any(|kind| kind.as_str() == Some("path_equals"))
        );
        assert!(
            allowed
                .iter()
                .any(|kind| kind.as_str() == Some("canonical_kind_is"))
        );

        let catalog = contract
            .get("condition_catalog")
            .and_then(Value::as_array)
            .expect("prompt includes condition catalog");
        assert!(catalog.iter().any(|entry| {
            entry.get("kind").and_then(Value::as_str) == Some("path_prefix")
                && entry.get("fact").and_then(Value::as_str) == Some("path.full")
        }));

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
                        condition.get("kind").and_then(Value::as_str) == Some("path_contains")
                    })
                })
                .unwrap_or(false)
        }));
    }

    #[test]
    fn role_discovery_prompt_explains_fact_synthesis_and_workspace_use() {
        let prompt = architecture_roles_seed_roles_user_prompt(
            &test_scope(),
            &json!({"canonical_files": []}),
        );
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

        let mapping = value
            .pointer("/rule_authoring_contract/fact_to_condition_mapping")
            .and_then(Value::as_array)
            .expect("fact mapping");
        assert!(mapping.iter().any(|entry| {
            entry.get("evidence_field").and_then(Value::as_str)
                == Some("canonical_artefacts.canonical_kind")
                && entry.get("condition_kind").and_then(Value::as_str) == Some("canonical_kind_is")
        }));

        let unsupported = value
            .pointer("/rule_authoring_contract/unsupported_today")
            .and_then(Value::as_array)
            .expect("unsupported list");
        assert!(unsupported.iter().any(|entry| {
            entry.get("signal").and_then(Value::as_str) == Some("dependency_count")
        }));
    }

    #[test]
    fn role_discovery_prompt_requires_generic_durable_roles_with_evidence() {
        let prompt = architecture_roles_seed_roles_user_prompt(
            &test_scope(),
            &json!({"canonical_files": []}),
        );
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
        let taxonomy =
            run_seed_generation(&service, &test_scope(), &json!({"canonical_files": []}))
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
}
