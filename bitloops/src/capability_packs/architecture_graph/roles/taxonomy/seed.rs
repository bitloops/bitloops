use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::capability_packs::architecture_graph::roles::storage::normalize_role_key;

use super::{RoleFactCondition, RoleFactConditionOp, TargetKind};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeededArchitectureTaxonomy {
    pub roles: Vec<SeededArchitectureRole>,
    #[serde(rename = "rule_candidates")]
    pub rule_candidates: Vec<SeededArchitectureRuleCandidate>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeededArchitectureRoleDiscovery {
    pub roles: Vec<SeededArchitectureRole>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeededArchitectureRuleCandidates {
    #[serde(rename = "rule_candidates")]
    pub rule_candidates: Vec<SeededArchitectureRuleCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeededArchitectureRole {
    pub canonical_key: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub lifecycle_status: Option<String>,
    #[serde(default)]
    pub provenance: Value,
    #[serde(default)]
    pub evidence: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeededArchitectureRuleCandidate {
    pub target_role_key: String,
    #[serde(default)]
    pub candidate_selector: RoleRuleCandidateSelector,
    #[serde(default)]
    pub positive_conditions: Vec<RoleRuleCondition>,
    #[serde(default)]
    pub negative_conditions: Vec<RoleRuleCondition>,
    #[serde(default)]
    pub score: RoleRuleScore,
    #[serde(default)]
    pub evidence: Value,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRuleCandidateSelector {
    #[serde(default)]
    pub target_kinds: Vec<TargetKind>,
    #[serde(default)]
    pub path_prefixes: Vec<String>,
    #[serde(default)]
    pub path_suffixes: Vec<String>,
    #[serde(default)]
    pub path_contains: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub canonical_kinds: Vec<String>,
    #[serde(default)]
    pub symbol_fqn_contains: Vec<String>,
    #[serde(default)]
    pub required_facts: Vec<RoleRuleCondition>,
    #[serde(default)]
    pub required_fact_any_groups: Vec<Vec<RoleRuleCondition>>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRuleCondition {
    pub kind: String,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub op: Option<RoleFactConditionOp>,
    pub value: Value,
    #[serde(default)]
    pub score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRuleScore {
    #[serde(default)]
    pub base_confidence: Option<f64>,
    #[serde(default)]
    pub priority_hint: Option<i64>,
    #[serde(default)]
    pub min_positive_ratio: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSpecFile {
    pub role_ref: String,
    #[serde(default)]
    pub candidate_selector: RoleRuleCandidateSelector,
    #[serde(default)]
    pub positive_conditions: Vec<RoleRuleCondition>,
    #[serde(default)]
    pub negative_conditions: Vec<RoleRuleCondition>,
    #[serde(default)]
    pub score: RoleRuleScore,
    #[serde(default)]
    pub evidence: Value,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleSplitSpecFile {
    pub target_roles: Vec<RoleSplitTargetRole>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleSplitTargetRole {
    pub canonical_key: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub alias_keys: Vec<String>,
}

pub const SUPPORTED_RULE_CONDITION_KINDS: [&str; 7] = [
    "path_contains",
    "path_equals",
    "path_prefix",
    "path_suffix",
    "language_is",
    "canonical_kind_is",
    "symbol_fqn_contains",
];

pub fn allowed_rule_condition_kinds() -> &'static [&'static str] {
    &SUPPORTED_RULE_CONDITION_KINDS
}

pub fn supported_rule_fact_catalog() -> Value {
    json!([
        {"kind": "path", "key": "full", "ops": ["eq", "contains", "prefix", "suffix"]},
        {"kind": "path", "key": "segment", "ops": ["eq"]},
        {"kind": "path", "key": "extension", "ops": ["eq"]},
        {"kind": "language", "key": "resolved", "ops": ["eq"]},
        {"kind": "file", "key": "analysis_mode", "ops": ["eq"]},
        {"kind": "file", "key": "role", "ops": ["eq"]},
        {"kind": "artefact", "key": "canonical_kind", "ops": ["eq"]},
        {"kind": "artefact", "key": "language_kind", "ops": ["eq", "contains"]},
        {"kind": "artefact", "key": "has_parent_artefact", "ops": ["eq"]},
        {"kind": "symbol", "key": "fqn", "ops": ["contains", "prefix", "suffix", "eq"]},
        {"kind": "symbol", "key": "name", "ops": ["eq", "contains", "prefix", "suffix"]},
        {"kind": "symbol", "key": "name_suffix", "ops": ["eq"]},
        {"kind": "symbol", "key": "has_signature", "ops": ["eq"]},
        {"kind": "signature", "key": "contains", "ops": ["eq"]},
        {"kind": "dependency", "key": "incoming_kind", "ops": ["eq"]},
        {"kind": "dependency", "key": "outgoing_kind", "ops": ["eq"]},
        {"kind": "dependency", "key": "incoming_count", "ops": ["gte", "lte", "eq"]},
        {"kind": "dependency", "key": "outgoing_count", "ops": ["gte", "lte", "eq"]}
    ])
}

pub fn validate_seeded_taxonomy(taxonomy: &SeededArchitectureTaxonomy) -> Result<()> {
    validate_seeded_roles(&taxonomy.roles)?;

    let keys = taxonomy
        .roles
        .iter()
        .map(|role| normalize_role_key(&role.canonical_key))
        .collect::<BTreeSet<_>>();

    for candidate in &taxonomy.rule_candidates {
        let target =
            normalise_non_empty("rule_candidate.target_role_key", &candidate.target_role_key)?;
        if !keys.contains(&target) {
            bail!("rule candidate references unknown target role key `{target}`");
        }
        validate_rule_shape(
            "rule_candidate",
            &candidate.candidate_selector,
            &candidate.positive_conditions,
            &candidate.negative_conditions,
            &candidate.score,
        )?;
    }
    Ok(())
}

pub fn validate_seeded_roles(roles: &[SeededArchitectureRole]) -> Result<()> {
    if roles.is_empty() {
        bail!("seeded architecture role discovery did not include any roles");
    }

    let mut keys = BTreeSet::new();
    for role in roles {
        let key = normalise_non_empty("role.canonical_key", &role.canonical_key)?;
        normalise_non_empty("role.display_name", &role.display_name)?;
        seeded_role_lifecycle_status(role.lifecycle_status.as_deref())?;
        if !keys.insert(key.clone()) {
            bail!("seeded taxonomy response contained duplicate role key `{key}`");
        }
    }

    Ok(())
}

pub fn validate_rule_spec_file(spec: &RuleSpecFile) -> Result<()> {
    normalise_non_empty("rule_spec.role_ref", &spec.role_ref)?;
    validate_rule_shape(
        "rule_spec",
        &spec.candidate_selector,
        &spec.positive_conditions,
        &spec.negative_conditions,
        &spec.score,
    )?;
    Ok(())
}

pub fn validate_role_split_spec(spec: &RoleSplitSpecFile) -> Result<()> {
    if spec.target_roles.is_empty() {
        bail!("split spec must declare at least one target role");
    }
    let mut keys = BTreeSet::new();
    for role in &spec.target_roles {
        let key = normalise_non_empty("split.target_role.canonical_key", &role.canonical_key)?;
        normalise_non_empty("split.target_role.display_name", &role.display_name)?;
        if !keys.insert(key.clone()) {
            bail!("split spec contained duplicate target role key `{key}`");
        }
    }
    Ok(())
}
pub fn generic_role_family_examples() -> Value {
    json!([
        {
            "family": "entrypoint",
            "examples": ["cli_command_surface", "http_route_handler", "job_runner"]
        },
        {
            "family": "application",
            "examples": ["use_case_orchestrator", "service_facade", "workflow_coordinator"]
        },
        {
            "family": "domain",
            "examples": ["aggregate_root", "domain_service", "policy_engine"]
        },
        {
            "family": "infrastructure",
            "examples": ["repository_adapter", "queue_adapter", "external_api_client"]
        }
    ])
}

pub fn role_rule_condition_catalog() -> Value {
    json!([
        {
            "kind": "path_contains",
            "fact": "path.full",
            "value": "Substring that must appear in the repository-relative path.",
            "description": "Use for stable path segments such as `src/cli`, `commands`, or `tests`."
        },
        {
            "kind": "path_equals",
            "fact": "path.full",
            "value": "Exact repository-relative path.",
            "description": "Use only when one specific file or artefact path is the intended deterministic match."
        },
        {
            "kind": "path_prefix",
            "fact": "path.full",
            "value": "Repository-relative path prefix.",
            "description": "Use for directories or stable source tree areas."
        },
        {
            "kind": "path_suffix",
            "fact": "path.full",
            "value": "Repository-relative path suffix.",
            "description": "Use for file names, extensions, or stable suffixes such as `_test.rs`."
        },
        {
            "kind": "language_is",
            "fact": "language.name",
            "value": "Language identifier from evidence, such as `rust` or `typescript`.",
            "description": "Use to keep a rule scoped to one language."
        },
        {
            "kind": "canonical_kind_is",
            "fact": "symbol.canonical_kind",
            "value": "Canonical artefact kind from evidence, such as `function`, `method`, `class`, or `test`.",
            "description": "Use to constrain rules to specific artefact kinds."
        },
        {
            "kind": "symbol_fqn_contains",
            "fact": "symbol.fqn",
            "value": "Substring that must appear in the fully qualified symbol name.",
            "description": "Use for stable module, namespace, type, or function naming patterns."
        }
    ])
}

pub fn role_rule_fact_to_condition_mapping() -> Value {
    json!([
        {
            "evidence_field": "canonical_files.path",
            "condition_kind": "path_prefix|path_suffix|path_contains|path_equals",
            "guidance": "Use stable repository-relative path structure. Prefer prefixes for directories and suffixes for file names or extensions."
        },
        {
            "evidence_field": "canonical_files.resolved_language",
            "condition_kind": "language_is",
            "guidance": "Use the resolved language value when available."
        },
        {
            "evidence_field": "canonical_artefacts.language",
            "condition_kind": "language_is",
            "guidance": "Use when the rule targets symbols rather than files."
        },
        {
            "evidence_field": "canonical_artefacts.canonical_kind",
            "condition_kind": "canonical_kind_is",
            "guidance": "Use for symbol kind constraints such as function, method, struct, class, module, or test when present in evidence."
        },
        {
            "evidence_field": "canonical_artefacts.symbol_fqn",
            "condition_kind": "symbol_fqn_contains",
            "guidance": "Use stable namespace, module, type, or function substrings. Avoid one-off generated ids."
        }
    ])
}

pub fn unsupported_role_rule_signals() -> Value {
    json!([
        {
            "signal": "dependency_count",
            "reason": "Dependency counts are useful evidence but are not a supported deterministic condition kind today."
        },
        {
            "signal": "dependency_edge_kind",
            "reason": "dependency_graph_hints can support confidence, but edge_kind is not directly matchable by the current classifier."
        },
        {
            "signal": "signature_contains",
            "reason": "Signatures are supplied as evidence, but the current deterministic rule DSL does not match signatures."
        },
        {
            "signal": "file_role",
            "reason": "file_role appears in canonical_files, but the current deterministic rule DSL does not match it."
        },
        {
            "signal": "analysis_mode",
            "reason": "analysis_mode appears in canonical_files, but the current deterministic rule DSL does not match it."
        },
        {
            "signal": "target_kind",
            "reason": "The current seed candidate selector does not expose target kind filtering."
        }
    ])
}

pub fn role_rule_candidate_examples() -> Value {
    json!([
        {
            "target_role_key": "cli_command_surface",
            "candidate_selector": {
                "target_kinds": ["artefact"],
                "path_prefixes": ["src/cli"],
                "path_suffixes": [".rs"],
                "path_contains": ["commands"],
                "languages": ["rust"],
                "canonical_kinds": ["function"],
                "symbol_fqn_contains": [],
                "required_facts": [
                    { "kind": "language", "key": "resolved", "op": "eq", "value": "rust", "score": 1.0 }
                ],
                "required_fact_any_groups": []
            },
            "positive_conditions": [
                { "kind": "path", "key": "full", "op": "prefix", "value": "src/cli", "score": 0.35 },
                { "kind": "path", "key": "full", "op": "contains", "value": "commands", "score": 0.35 },
                { "kind": "language", "key": "resolved", "op": "eq", "value": "rust", "score": 0.30 }
            ],
            "negative_conditions": [
                { "kind": "path", "key": "full", "op": "suffix", "value": "_test.rs", "score": 1.0 }
            ],
            "score": {
                "base_confidence": 0.82,
                "priority_hint": 100,
                "min_positive_ratio": 1.0
            },
            "evidence": {
                "inspected_paths": ["src/cli/commands/run.rs"],
                "positive_examples": [
                    {
                        "path": "src/cli/commands/run.rs",
                        "symbol_fqn": "crate::cli::commands::run",
                        "canonical_kind": "function",
                        "why": "Command path and function symbol match the CLI command surface role."
                    }
                ],
                "negative_examples": [
                    {
                        "path": "src/cli/commands/run_test.rs",
                        "symbol_fqn": null,
                        "canonical_kind": "test",
                        "why": "Test files should not define the runtime command surface role."
                    }
                ],
                "db_sections_used": ["canonical_files", "canonical_artefacts"],
                "reasoning_summary": "CLI command files under src/cli/commands in Rust are likely command-surface artefacts.",
                "confidence_reason": "Path, language, and canonical kind are stable deterministic signals.",
                "uncertainty": ""
            },
            "metadata": {}
        },
        {
            "target_role_key": "domain_policy",
            "candidate_selector": {
                "target_kinds": ["artefact"],
                "path_prefixes": ["src/domain"],
                "path_suffixes": [],
                "path_contains": [],
                "languages": ["rust"],
                "canonical_kinds": ["struct", "enum", "function"],
                "symbol_fqn_contains": ["policy"],
                "required_facts": [
                    { "kind": "language", "key": "resolved", "op": "eq", "value": "rust", "score": 1.0 }
                ],
                "required_fact_any_groups": []
            },
            "positive_conditions": [
                { "kind": "artefact", "key": "canonical_kind", "op": "eq", "value": "function", "score": 0.50 },
                { "kind": "symbol", "key": "fqn", "op": "contains", "value": "policy", "score": 0.50 }
            ],
            "negative_conditions": [],
            "score": {
                "base_confidence": 0.74,
                "priority_hint": 100,
                "min_positive_ratio": 1.0
            },
            "evidence": {
                "inspected_paths": ["src/domain/policy.rs"],
                "positive_examples": [
                    {
                        "path": "src/domain/policy.rs",
                        "symbol_fqn": "crate::domain::policy::apply_policy",
                        "canonical_kind": "function",
                        "why": "Domain path and policy symbol naming match the role."
                    }
                ],
                "negative_examples": [],
                "db_sections_used": ["canonical_files", "canonical_artefacts"],
                "reasoning_summary": "Domain path and policy naming are stable enough for reviewable deterministic suggestions.",
                "confidence_reason": "The rule uses stable path and symbol naming constraints.",
                "uncertainty": ""
            },
            "metadata": {}
        }
    ])
}

fn strict_empty_object_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "required": [],
        "additionalProperties": false
    })
}

fn string_array_schema() -> Value {
    json!({
        "type": "array",
        "items": { "type": "string" }
    })
}

fn evidence_example_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["path", "symbol_fqn", "canonical_kind", "why"],
        "properties": {
            "path": { "type": "string" },
            "symbol_fqn": { "type": ["string", "null"] },
            "canonical_kind": { "type": ["string", "null"] },
            "why": { "type": "string" }
        }
    })
}

fn seeded_role_evidence_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "inspected_paths",
            "supporting_paths",
            "supporting_symbols",
            "db_sections_used",
            "reasoning_summary",
            "confidence_reason",
            "uncertainty"
        ],
        "properties": {
            "inspected_paths": string_array_schema(),
            "supporting_paths": string_array_schema(),
            "supporting_symbols": string_array_schema(),
            "db_sections_used": string_array_schema(),
            "reasoning_summary": { "type": "string" },
            "confidence_reason": { "type": "string" },
            "uncertainty": { "type": "string" }
        }
    })
}

fn seeded_rule_evidence_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "inspected_paths",
            "positive_examples",
            "negative_examples",
            "db_sections_used",
            "reasoning_summary",
            "confidence_reason",
            "uncertainty"
        ],
        "properties": {
            "inspected_paths": string_array_schema(),
            "positive_examples": {
                "type": "array",
                "items": evidence_example_schema()
            },
            "negative_examples": {
                "type": "array",
                "items": evidence_example_schema()
            },
            "db_sections_used": string_array_schema(),
            "reasoning_summary": { "type": "string" },
            "confidence_reason": { "type": "string" },
            "uncertainty": { "type": "string" }
        }
    })
}

fn role_condition_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "key", "op", "value", "score"],
        "properties": {
            "kind": { "type": "string", "minLength": 1 },
            "key": { "type": "string", "minLength": 1 },
            "op": { "type": "string", "enum": ["eq", "contains", "prefix", "suffix", "gte", "lte"] },
            "value": { "type": "string", "minLength": 1 },
            "score": { "type": "number", "minimum": 0, "maximum": 1 }
        }
    })
}

fn seeded_role_schema() -> Value {
    let strict_object = strict_empty_object_schema();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "canonical_key",
            "display_name",
            "description",
            "family",
            "provenance",
            "evidence"
        ],
        "properties": {
            "canonical_key": { "type": "string", "minLength": 1 },
            "display_name": { "type": "string", "minLength": 1 },
            "description": { "type": "string" },
            "family": { "type": ["string", "null"] },
            "provenance": strict_object.clone(),
            "evidence": seeded_role_evidence_schema()
        }
    })
}

fn seeded_rule_candidate_schema() -> Value {
    let strict_object = strict_empty_object_schema();
    let condition_schema = role_condition_schema();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "target_role_key",
            "candidate_selector",
            "positive_conditions",
            "negative_conditions",
            "score",
            "evidence",
            "metadata"
        ],
        "properties": {
            "target_role_key": { "type": "string", "minLength": 1 },
            "candidate_selector": {
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "target_kinds",
                    "path_prefixes",
                    "path_suffixes",
                    "path_contains",
                    "languages",
                    "canonical_kinds",
                    "symbol_fqn_contains",
                    "required_facts",
                    "required_fact_any_groups"
                ],
                "properties": {
                    "target_kinds": {
                        "type": "array",
                        "items": { "type": "string", "enum": ["file", "artefact", "symbol"] }
                    },
                    "path_prefixes": { "type": "array", "items": { "type": "string" } },
                    "path_suffixes": { "type": "array", "items": { "type": "string" } },
                    "path_contains": { "type": "array", "items": { "type": "string" } },
                    "languages": { "type": "array", "items": { "type": "string" } },
                    "canonical_kinds": { "type": "array", "items": { "type": "string" } },
                    "symbol_fqn_contains": { "type": "array", "items": { "type": "string" } },
                    "required_facts": {
                        "type": "array",
                        "items": condition_schema.clone()
                    },
                    "required_fact_any_groups": {
                        "type": "array",
                        "items": {
                            "type": "array",
                            "items": condition_schema.clone()
                        }
                    }
                }
            },
            "positive_conditions": {
                "type": "array",
                "items": condition_schema.clone()
            },
            "negative_conditions": {
                "type": "array",
                "items": condition_schema
            },
            "score": {
                "type": "object",
                "additionalProperties": false,
                "required": ["base_confidence", "priority_hint", "min_positive_ratio"],
                "properties": {
                    "base_confidence": { "type": ["number", "null"], "minimum": 0, "maximum": 1 },
                    "priority_hint": { "type": ["integer", "null"] },
                    "min_positive_ratio": { "type": ["number", "null"], "minimum": 0, "maximum": 1 }
                }
            },
            "evidence": seeded_rule_evidence_schema(),
            "metadata": strict_object
        }
    })
}

pub fn architecture_roles_seed_roles_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["roles"],
        "properties": {
            "roles": {
                "type": "array",
                "minItems": 1,
                "items": seeded_role_schema()
            }
        }
    })
}

pub fn architecture_roles_seed_rule_candidates_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["rule_candidates"],
        "properties": {
            "rule_candidates": {
                "type": "array",
                "items": seeded_rule_candidate_schema()
            }
        }
    })
}

pub fn architecture_roles_seed_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["roles", "rule_candidates"],
        "properties": {
            "roles": {
                "type": "array",
                "minItems": 1,
                "items": seeded_role_schema()
            },
            "rule_candidates": {
                "type": "array",
                "items": seeded_rule_candidate_schema()
            }
        }
    })
}
fn normalise_non_empty(field_name: &str, value: &str) -> Result<String> {
    let normalized = normalize_role_key(value);
    if normalized.is_empty() {
        return Err(anyhow!("{field_name} must not be empty"));
    }
    Ok(normalized)
}

pub fn seeded_role_lifecycle_status(lifecycle_status: Option<&str>) -> Result<&'static str> {
    match lifecycle_status.map(str::trim) {
        None => Ok("active"),
        Some("active") => Ok("active"),
        Some(value) => bail!(
            "unsupported seeded role lifecycle_status `{value}`; seed inference only creates active roles"
        ),
    }
}

fn validate_rule_shape(
    prefix: &str,
    selector: &RoleRuleCandidateSelector,
    positive_conditions: &[RoleRuleCondition],
    negative_conditions: &[RoleRuleCondition],
    score: &RoleRuleScore,
) -> Result<()> {
    validate_string_list(
        &format!("{prefix}.candidate_selector.path_prefixes"),
        &selector.path_prefixes,
    )?;
    validate_string_list(
        &format!("{prefix}.candidate_selector.path_suffixes"),
        &selector.path_suffixes,
    )?;
    validate_string_list(
        &format!("{prefix}.candidate_selector.path_contains"),
        &selector.path_contains,
    )?;
    validate_string_list(
        &format!("{prefix}.candidate_selector.languages"),
        &selector.languages,
    )?;
    validate_string_list(
        &format!("{prefix}.candidate_selector.canonical_kinds"),
        &selector.canonical_kinds,
    )?;
    validate_string_list(
        &format!("{prefix}.candidate_selector.symbol_fqn_contains"),
        &selector.symbol_fqn_contains,
    )?;
    for condition in &selector.required_facts {
        validate_condition(
            &format!("{prefix}.candidate_selector.required_facts"),
            condition,
        )?;
    }
    for group in &selector.required_fact_any_groups {
        for condition in group {
            validate_condition(
                &format!("{prefix}.candidate_selector.required_fact_any_groups"),
                condition,
            )?;
        }
    }
    for condition in positive_conditions {
        validate_condition(&format!("{prefix}.positive_conditions"), condition)?;
    }
    for condition in negative_conditions {
        validate_condition(&format!("{prefix}.negative_conditions"), condition)?;
    }
    reject_signature_only_positive_conditions(
        &format!("{prefix}.positive_conditions"),
        positive_conditions,
    )?;
    validate_optional_unit_interval(
        &format!("{prefix}.score.base_confidence"),
        score.base_confidence,
    )?;
    validate_optional_unit_interval(
        &format!("{prefix}.score.min_positive_ratio"),
        score.min_positive_ratio,
    )?;
    Ok(())
}

fn validate_optional_unit_interval(field_name: &str, value: Option<f64>) -> Result<()> {
    if let Some(value) = value
        && !(0.0..=1.0).contains(&value)
    {
        bail!("{field_name} must be between 0 and 1");
    }
    Ok(())
}

fn validate_string_list(field_name: &str, values: &[String]) -> Result<()> {
    if values.iter().any(|value| value.trim().is_empty()) {
        bail!("{field_name} must not contain blank values");
    }
    Ok(())
}

fn validate_condition(field_name: &str, condition: &RoleRuleCondition) -> Result<()> {
    if condition.key.is_some() || condition.op.is_some() || condition.score.is_some() {
        let fact_condition = fact_condition_from_rule_condition(field_name, condition)?;
        return validate_supported_fact_condition(field_name, &fact_condition);
    }

    let kind = condition.kind.trim();
    if kind.is_empty() {
        bail!("{field_name}.kind must not be empty");
    }
    if !SUPPORTED_RULE_CONDITION_KINDS.contains(&kind) {
        bail!("unsupported rule condition kind `{kind}`");
    }
    if condition.value.as_str().is_none() {
        bail!("{field_name}.{kind} must use a string value");
    }
    Ok(())
}

fn reject_signature_only_positive_conditions(
    field_name: &str,
    conditions: &[RoleRuleCondition],
) -> Result<()> {
    let mut saw_fact_condition = false;
    let mut saw_non_signature_condition = false;
    for condition in conditions {
        if condition.key.is_none() && condition.op.is_none() && condition.score.is_none() {
            saw_non_signature_condition = true;
            continue;
        }
        let fact_condition = fact_condition_from_rule_condition(field_name, condition)?;
        saw_fact_condition = true;
        if fact_condition.kind != "signature" || fact_condition.key != "contains" {
            saw_non_signature_condition = true;
        }
    }
    if saw_fact_condition && !saw_non_signature_condition {
        bail!("{field_name} must include at least one non-signature condition");
    }
    Ok(())
}

fn fact_condition_from_rule_condition(
    field_name: &str,
    condition: &RoleRuleCondition,
) -> Result<RoleFactCondition> {
    let key = condition
        .key
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("{field_name}.key must not be empty"))?;
    let op = condition
        .op
        .ok_or_else(|| anyhow!("{field_name}.op must not be empty"))?;
    let value = condition
        .value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("{field_name}.value must not be empty"))?;
    Ok(RoleFactCondition {
        kind: condition.kind.trim().to_string(),
        key: key.to_string(),
        op,
        value: value.to_string(),
        score: condition
            .score
            .unwrap_or_else(super::default_condition_score),
    })
}

pub fn validate_supported_fact_condition(
    prefix: &str,
    condition: &RoleFactCondition,
) -> Result<()> {
    let Some(ops) = supported_fact_ops(&condition.kind, &condition.key) else {
        bail!(
            "{prefix} uses unsupported fact condition {}.{}",
            condition.kind,
            condition.key
        );
    };
    let op = fact_condition_op_name(condition.op);
    if !ops.contains(&op) {
        bail!(
            "{prefix} uses unsupported op {op} for fact {}.{}",
            condition.kind,
            condition.key
        );
    }
    if condition.value.trim().is_empty() {
        bail!("{prefix}.value must not be empty");
    }
    if matches!(
        condition.op,
        RoleFactConditionOp::Gte | RoleFactConditionOp::Lte
    ) {
        condition.value.parse::<f64>().map_err(|_| {
            anyhow!(
                "{prefix}.value must be numeric for {}.{} {op}",
                condition.kind,
                condition.key
            )
        })?;
    }
    if !(0.0..=1.0).contains(&condition.score) {
        bail!("{prefix}.score must be between 0 and 1");
    }
    Ok(())
}

fn supported_fact_ops(kind: &str, key: &str) -> Option<&'static [&'static str]> {
    match (kind, key) {
        ("path", "full") => Some(&["eq", "contains", "prefix", "suffix"]),
        ("path", "segment" | "extension") => Some(&["eq"]),
        ("language", "resolved") => Some(&["eq"]),
        ("file", "analysis_mode" | "role") => Some(&["eq"]),
        ("artefact", "canonical_kind") => Some(&["eq"]),
        ("artefact", "language_kind") => Some(&["eq", "contains"]),
        ("artefact", "has_parent_artefact") => Some(&["eq"]),
        ("symbol", "fqn") => Some(&["contains", "prefix", "suffix", "eq"]),
        ("symbol", "name") => Some(&["eq", "contains", "prefix", "suffix"]),
        ("symbol", "name_suffix" | "has_signature") => Some(&["eq"]),
        ("signature", "contains") => Some(&["eq"]),
        ("dependency", "incoming_kind" | "outgoing_kind") => Some(&["eq"]),
        ("dependency", "incoming_count" | "outgoing_count") => Some(&["gte", "lte", "eq"]),
        _ => None,
    }
}

fn fact_condition_op_name(op: RoleFactConditionOp) -> &'static str {
    match op {
        RoleFactConditionOp::Eq => "eq",
        RoleFactConditionOp::Contains => "contains",
        RoleFactConditionOp::Prefix => "prefix",
        RoleFactConditionOp::Suffix => "suffix",
        RoleFactConditionOp::Gte => "gte",
        RoleFactConditionOp::Lte => "lte",
    }
}
