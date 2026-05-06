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
pub fn validate_seeded_taxonomy(taxonomy: &SeededArchitectureTaxonomy) -> Result<()> {
    if taxonomy.roles.is_empty() {
        bail!("seeded taxonomy response did not include any roles");
    }

    let mut keys = BTreeSet::new();
    for role in &taxonomy.roles {
        let key = normalise_non_empty("role.canonical_key", &role.canonical_key)?;
        normalise_non_empty("role.display_name", &role.display_name)?;
        if !keys.insert(key.clone()) {
            bail!("seeded taxonomy response contained duplicate role key `{key}`");
        }
    }

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

pub fn architecture_roles_seed_schema() -> Value {
    let strict_object = json!({
        "type": "object",
        "additionalProperties": false
    });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["roles", "rule_candidates"],
        "properties": {
            "roles": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["canonical_key", "display_name"],
                    "properties": {
                        "canonical_key": { "type": "string", "minLength": 1 },
                        "display_name": { "type": "string", "minLength": 1 },
                        "description": { "type": "string" },
                        "family": { "type": ["string", "null"] },
                        "lifecycle_status": { "type": ["string", "null"] },
                        "provenance": strict_object.clone(),
                        "evidence": strict_object.clone()
                    }
                }
            },
            "rule_candidates": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["target_role_key", "candidate_selector"],
                    "properties": {
                        "target_role_key": { "type": "string", "minLength": 1 },
                        "candidate_selector": {
                            "type": "object",
                            "additionalProperties": false,
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
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["kind", "value"],
                                "properties": {
                                    "kind": { "type": "string", "minLength": 1 },
                                    "value": { "type": "string", "minLength": 1 }
                                }
                            }
                        },
                        "negative_conditions": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["kind", "value"],
                                "properties": {
                                    "kind": { "type": "string", "minLength": 1 },
                                    "value": { "type": "string", "minLength": 1 }
                                }
                            }
                        },
                        "score": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "base_confidence": { "type": "number", "minimum": 0, "maximum": 1 },
                                "weight": { "type": "number" }
                            }
                        },
                        "evidence": strict_object.clone(),
                        "metadata": strict_object.clone()
                    }
                }
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
    match kind {
        "path_contains"
        | "path_prefix"
        | "path_suffix"
        | "language_is"
        | "canonical_kind_is"
        | "symbol_fqn_contains" => {}
        _ => bail!("unsupported rule condition kind `{kind}`"),
    }
    if condition.value.as_str().is_none() {
        bail!("{field_name}.{kind} must use a string value");
    }
    Ok(())
}
