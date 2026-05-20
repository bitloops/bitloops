use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::capability_packs::architecture_graph::roles::storage::normalize_role_key;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeededArchitectureTaxonomy {
    pub roles: Vec<SeededArchitectureRole>,
    #[serde(rename = "rule_candidates")]
    pub rule_candidates: Vec<SeededArchitectureRuleCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRuleCandidateSelector {
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRuleCondition {
    pub kind: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRuleScore {
    #[serde(default)]
    pub base_confidence: Option<f64>,
    #[serde(default)]
    pub weight: Option<f64>,
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

pub fn role_rule_candidate_examples() -> Value {
    json!([
        {
            "target_role_key": "cli_command_surface",
            "candidate_selector": {
                "path_prefixes": ["src/cli"],
                "path_suffixes": [".rs"],
                "path_contains": ["commands"],
                "languages": ["rust"],
                "canonical_kinds": ["function"],
                "symbol_fqn_contains": []
            },
            "positive_conditions": [
                { "kind": "path_prefix", "value": "src/cli" },
                { "kind": "path_contains", "value": "commands" },
                { "kind": "language_is", "value": "rust" }
            ],
            "negative_conditions": [
                { "kind": "path_suffix", "value": "_test.rs" }
            ],
            "score": {
                "base_confidence": 0.82,
                "weight": 1.0
            },
            "evidence": {
                "example_only": true,
                "why": "CLI command files under src/cli/commands in Rust are likely command-surface artefacts."
            },
            "metadata": {
                "example_only": true
            }
        },
        {
            "target_role_key": "domain_policy",
            "candidate_selector": {
                "path_prefixes": ["src/domain"],
                "path_suffixes": [],
                "path_contains": [],
                "languages": ["rust"],
                "canonical_kinds": ["struct", "enum", "function"],
                "symbol_fqn_contains": ["policy"]
            },
            "positive_conditions": [
                { "kind": "canonical_kind_is", "value": "function" },
                { "kind": "symbol_fqn_contains", "value": "policy" }
            ],
            "negative_conditions": [],
            "score": {
                "base_confidence": 0.74,
                "weight": 1.0
            },
            "evidence": {
                "example_only": true,
                "why": "Domain path and policy naming are stable enough for reviewable deterministic suggestions."
            },
            "metadata": {
                "example_only": true
            }
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

fn role_condition_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "value"],
        "properties": {
            "kind": {
                "type": "string",
                "enum": SUPPORTED_RULE_CONDITION_KINDS
            },
            "value": { "type": "string", "minLength": 1 }
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
            "evidence": strict_object
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
                    "path_prefixes",
                    "path_suffixes",
                    "path_contains",
                    "languages",
                    "canonical_kinds",
                    "symbol_fqn_contains"
                ],
                "properties": {
                    "path_prefixes": { "type": "array", "items": { "type": "string" } },
                    "path_suffixes": { "type": "array", "items": { "type": "string" } },
                    "path_contains": { "type": "array", "items": { "type": "string" } },
                    "languages": { "type": "array", "items": { "type": "string" } },
                    "canonical_kinds": { "type": "array", "items": { "type": "string" } },
                    "symbol_fqn_contains": { "type": "array", "items": { "type": "string" } }
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
                "required": ["base_confidence", "weight"],
                "properties": {
                    "base_confidence": { "type": ["number", "null"], "minimum": 0, "maximum": 1 },
                    "weight": { "type": ["number", "null"] }
                }
            },
            "evidence": strict_object.clone(),
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
    for condition in positive_conditions {
        validate_condition(&format!("{prefix}.positive_conditions"), condition)?;
    }
    for condition in negative_conditions {
        validate_condition(&format!("{prefix}.negative_conditions"), condition)?;
    }
    if let Some(base_confidence) = score.base_confidence
        && !(0.0..=1.0).contains(&base_confidence)
    {
        bail!("{prefix}.score.base_confidence must be between 0 and 1");
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
