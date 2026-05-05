use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchableArtefact {
    pub artefact_id: String,
    pub path: String,
    pub language: Option<String>,
    pub canonical_kind: Option<String>,
    pub symbol_fqn: Option<String>,
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

pub fn parse_rule_selector(value: &Value) -> Result<RoleRuleCandidateSelector> {
    serde_json::from_value(value.clone()).context("parse role rule selector")
}

pub fn parse_rule_conditions(value: &Value) -> Result<Vec<RoleRuleCondition>> {
    serde_json::from_value(value.clone()).context("parse role rule conditions")
}

pub fn parse_rule_score(value: &Value) -> Result<RoleRuleScore> {
    serde_json::from_value(value.clone()).context("parse role rule score")
}

pub fn role_rule_matches(
    selector: &RoleRuleCandidateSelector,
    positive_conditions: &[RoleRuleCondition],
    negative_conditions: &[RoleRuleCondition],
    artefact: &MatchableArtefact,
) -> bool {
    if !selector_matches(selector, artefact) {
        return false;
    }
    if positive_conditions
        .iter()
        .any(|condition| !condition_matches(condition, artefact))
    {
        return false;
    }
    if negative_conditions
        .iter()
        .any(|condition| condition_matches(condition, artefact))
    {
        return false;
    }
    true
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
                        "provenance": {},
                        "evidence": {}
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
                                    "value": {}
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
                                    "value": {}
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
                        "evidence": {},
                        "metadata": {}
                    }
                }
            }
        }
    })
}

fn selector_matches(selector: &RoleRuleCandidateSelector, artefact: &MatchableArtefact) -> bool {
    if !selector.path_prefixes.is_empty()
        && !selector
            .path_prefixes
            .iter()
            .any(|prefix| artefact.path.starts_with(prefix))
    {
        return false;
    }
    if !selector.path_suffixes.is_empty()
        && !selector
            .path_suffixes
            .iter()
            .any(|suffix| artefact.path.ends_with(suffix))
    {
        return false;
    }
    if !selector.path_contains.is_empty()
        && !selector
            .path_contains
            .iter()
            .any(|needle| artefact.path.contains(needle))
    {
        return false;
    }
    if !selector.languages.is_empty()
        && !selector.languages.iter().any(|language| {
            artefact
                .language
                .as_deref()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(language))
        })
    {
        return false;
    }
    if !selector.canonical_kinds.is_empty()
        && !selector.canonical_kinds.iter().any(|kind| {
            artefact
                .canonical_kind
                .as_deref()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(kind))
        })
    {
        return false;
    }
    if !selector.symbol_fqn_contains.is_empty()
        && !selector.symbol_fqn_contains.iter().any(|needle| {
            artefact
                .symbol_fqn
                .as_deref()
                .is_some_and(|actual| actual.contains(needle))
        })
    {
        return false;
    }
    true
}

fn condition_matches(condition: &RoleRuleCondition, artefact: &MatchableArtefact) -> bool {
    match condition.kind.trim() {
        "path_contains" => condition
            .value
            .as_str()
            .is_some_and(|value| artefact.path.contains(value)),
        "path_prefix" => condition
            .value
            .as_str()
            .is_some_and(|value| artefact.path.starts_with(value)),
        "path_suffix" => condition
            .value
            .as_str()
            .is_some_and(|value| artefact.path.ends_with(value)),
        "language_is" => condition.value.as_str().is_some_and(|value| {
            artefact
                .language
                .as_deref()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(value))
        }),
        "canonical_kind_is" => condition.value.as_str().is_some_and(|value| {
            artefact
                .canonical_kind
                .as_deref()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(value))
        }),
        "symbol_fqn_contains" => condition.value.as_str().is_some_and(|value| {
            artefact
                .symbol_fqn
                .as_deref()
                .is_some_and(|actual| actual.contains(value))
        }),
        _ => false,
    }
}

fn normalise_non_empty(field_name: &str, value: &str) -> Result<String> {
    let normalized = super::storage::normalize_role_key(value);
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
    if let Some(base_confidence) = score.base_confidence {
        if !(0.0..=1.0).contains(&base_confidence) {
            bail!("{prefix}.score.base_confidence must be between 0 and 1");
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_seeded_taxonomy_and_rejects_unknown_target_roles() {
        let valid = SeededArchitectureTaxonomy {
            roles: vec![SeededArchitectureRole {
                canonical_key: "command_dispatcher".to_string(),
                display_name: "Command Dispatcher".to_string(),
                description: String::new(),
                family: Some("entrypoint".to_string()),
                lifecycle_status: Some("active".to_string()),
                provenance: json!({}),
                evidence: json!([]),
            }],
            rule_candidates: vec![SeededArchitectureRuleCandidate {
                target_role_key: "command_dispatcher".to_string(),
                candidate_selector: RoleRuleCandidateSelector {
                    path_prefixes: vec!["src/cli".to_string()],
                    ..Default::default()
                },
                positive_conditions: vec![],
                negative_conditions: vec![],
                score: RoleRuleScore {
                    base_confidence: Some(0.8),
                    weight: None,
                },
                evidence: json!([]),
                metadata: json!({}),
            }],
        };
        validate_seeded_taxonomy(&valid).expect("valid taxonomy");

        let invalid = SeededArchitectureTaxonomy {
            roles: valid.roles.clone(),
            rule_candidates: vec![SeededArchitectureRuleCandidate {
                target_role_key: "unknown".to_string(),
                ..valid.rule_candidates[0].clone()
            }],
        };
        let err = validate_seeded_taxonomy(&invalid).expect_err("invalid taxonomy");
        assert!(err.to_string().contains("unknown target role key"));

        let invalid_condition = SeededArchitectureTaxonomy {
            roles: vec![SeededArchitectureRole {
                canonical_key: "command_dispatcher".to_string(),
                display_name: "Command Dispatcher".to_string(),
                description: String::new(),
                family: Some("entrypoint".to_string()),
                lifecycle_status: Some("active".to_string()),
                provenance: json!({}),
                evidence: json!([]),
            }],
            rule_candidates: vec![SeededArchitectureRuleCandidate {
                target_role_key: "command_dispatcher".to_string(),
                candidate_selector: RoleRuleCandidateSelector::default(),
                positive_conditions: vec![RoleRuleCondition {
                    kind: "unsupported".to_string(),
                    value: json!("x"),
                }],
                negative_conditions: vec![],
                score: RoleRuleScore::default(),
                evidence: json!([]),
                metadata: json!({}),
            }],
        };
        let err = validate_seeded_taxonomy(&invalid_condition).expect_err("invalid condition kind");
        assert!(err.to_string().contains("unsupported rule condition kind"));
    }

    #[test]
    fn selector_and_conditions_match_expected_artefacts() {
        let artefact = MatchableArtefact {
            artefact_id: "artefact-1".to_string(),
            path: "src/cli/commands/run.rs".to_string(),
            language: Some("rust".to_string()),
            canonical_kind: Some("function".to_string()),
            symbol_fqn: Some("crate::cli::commands::run".to_string()),
        };

        let selector = RoleRuleCandidateSelector {
            path_prefixes: vec!["src/cli".to_string()],
            languages: vec!["rust".to_string()],
            ..Default::default()
        };
        let positive = vec![RoleRuleCondition {
            kind: "path_contains".to_string(),
            value: json!("commands"),
        }];

        assert!(role_rule_matches(&selector, &positive, &[], &artefact));

        let negative = vec![RoleRuleCondition {
            kind: "path_suffix".to_string(),
            value: json!(".ts"),
        }];
        assert!(role_rule_matches(
            &selector, &positive, &negative, &artefact
        ));
    }
}
