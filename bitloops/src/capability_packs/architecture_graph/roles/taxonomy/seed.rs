use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::capability_packs::architecture_graph::roles::storage::normalize_role_key;

use super::{RoleFactCondition, RoleFactConditionOp, TargetKind};

mod contract;
mod schema;
mod supported_facts;

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
pub struct SeededRoleRulePredicateCondition {
    pub predicate: String,
    pub value: String,
    pub score: f64,
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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DecodedSeedRuleCandidates {
    pub accepted: Vec<SeededArchitectureRuleCandidate>,
    pub repaired: Vec<SeedRuleCandidateRepair>,
    pub rejected: Vec<SeedRuleCandidateValidationIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeedRuleCandidateValidationIssue {
    pub candidate_index: usize,
    pub role_slug: Option<String>,
    pub candidate_slug: Option<String>,
    pub field_path: String,
    pub reason: String,
    pub raw_candidate_excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeedRuleCandidateRepair {
    pub candidate_index: usize,
    pub role_slug: Option<String>,
    pub candidate_slug: Option<String>,
    pub field_path: String,
    pub from: String,
    pub to: String,
    pub reason: String,
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
    supported_facts::catalog()
}

pub use contract::{
    generic_role_family_examples, role_rule_candidate_examples, role_rule_condition_catalog,
    role_rule_fact_to_condition_mapping, rule_authoring_contract_json,
    unsupported_role_rule_signals,
};
pub(crate) use schema::seeded_rule_candidate_schema;
pub use schema::{
    architecture_roles_seed_roles_schema, architecture_roles_seed_rule_candidates_schema,
    architecture_roles_seed_schema,
};
pub use supported_facts::{
    SupportedFactPredicate, parse_supported_fact_predicate, supported_fact_predicate_ids,
    supported_fact_predicates,
};

pub fn validate_seeded_taxonomy(taxonomy: &SeededArchitectureTaxonomy) -> Result<()> {
    validate_seeded_roles(&taxonomy.roles)?;

    let keys = taxonomy
        .roles
        .iter()
        .map(|role| normalize_role_key(&role.canonical_key))
        .collect::<BTreeSet<_>>();

    for (candidate_index, candidate) in taxonomy.rule_candidates.iter().enumerate() {
        let target = normalise_non_empty(
            &format!("rule_candidates[{candidate_index}].target_role_key"),
            &candidate.target_role_key,
        )?;
        if !keys.contains(&target) {
            bail!("rule candidate {candidate_index} references unknown target role key `{target}`");
        }
        let prefix = format!("rule_candidates[{candidate_index}:{target}]");
        validate_rule_shape(
            &prefix,
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

pub fn role_fact_condition_from_seed_predicate(
    field_path: &str,
    condition: &SeededRoleRulePredicateCondition,
) -> Result<RoleFactCondition> {
    let predicate =
        parse_supported_fact_predicate(condition.predicate.trim()).ok_or_else(|| {
            anyhow!(
                "{field_path}.predicate uses unsupported predicate `{}`",
                condition.predicate
            )
        })?;
    let value = condition.value.trim();
    if value.is_empty() {
        bail!("{field_path}.value must not be empty");
    }
    if !condition.score.is_finite() || !(0.0..=1.0).contains(&condition.score) {
        bail!("{field_path}.score must be between 0 and 1");
    }
    let op = role_fact_condition_op_from_name(predicate.op).ok_or_else(|| {
        anyhow!(
            "{field_path}.predicate uses unsupported op `{}`",
            predicate.op
        )
    })?;
    let fact_condition = RoleFactCondition {
        kind: predicate.kind.to_string(),
        key: predicate.key.to_string(),
        op,
        value: value.to_string(),
        score: condition.score,
    };
    validate_supported_fact_condition(field_path, &fact_condition)?;
    Ok(fact_condition)
}

pub fn decode_seeded_rule_candidates_with_recovery(value: Value) -> DecodedSeedRuleCandidates {
    let Some(raw_candidates) = value.get("rule_candidates").and_then(Value::as_array) else {
        return DecodedSeedRuleCandidates {
            rejected: vec![SeedRuleCandidateValidationIssue {
                candidate_index: 0,
                role_slug: None,
                candidate_slug: None,
                field_path: "rule_candidates".to_string(),
                reason: "response must include rule_candidates array".to_string(),
                raw_candidate_excerpt: compact_raw_excerpt(&value),
            }],
            ..Default::default()
        };
    };

    let mut decoded = DecodedSeedRuleCandidates::default();
    for (candidate_index, raw_candidate) in raw_candidates.iter().enumerate() {
        let role_slug = raw_candidate
            .get("target_role_key")
            .and_then(Value::as_str)
            .map(str::to_string);
        let candidate_slug = raw_candidate
            .pointer("/metadata/slug")
            .and_then(Value::as_str)
            .map(str::to_string);
        match decode_seeded_rule_candidate_with_recovery(candidate_index, raw_candidate.clone()) {
            Ok((candidate, mut repaired)) => {
                decoded.accepted.push(candidate);
                decoded.repaired.append(&mut repaired);
            }
            Err((field_path, reason)) => {
                decoded.rejected.push(SeedRuleCandidateValidationIssue {
                    candidate_index,
                    role_slug,
                    candidate_slug,
                    field_path,
                    reason,
                    raw_candidate_excerpt: compact_raw_excerpt(raw_candidate),
                });
            }
        }
    }

    decoded
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
    for (condition_index, condition) in selector.required_facts.iter().enumerate() {
        validate_condition(
            &format!("{prefix}.candidate_selector.required_facts[{condition_index}]"),
            condition,
        )?;
    }
    for (group_index, group) in selector.required_fact_any_groups.iter().enumerate() {
        for (condition_index, condition) in group.iter().enumerate() {
            validate_condition(
                &format!(
                    "{prefix}.candidate_selector.required_fact_any_groups[{group_index}][{condition_index}]"
                ),
                condition,
            )?;
        }
    }
    for (condition_index, condition) in positive_conditions.iter().enumerate() {
        validate_condition(
            &format!("{prefix}.positive_conditions[{condition_index}]"),
            condition,
        )?;
    }
    for (condition_index, condition) in negative_conditions.iter().enumerate() {
        validate_condition(
            &format!("{prefix}.negative_conditions[{condition_index}]"),
            condition,
        )?;
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

fn decode_seeded_rule_candidate_with_recovery(
    candidate_index: usize,
    mut raw_candidate: Value,
) -> std::result::Result<
    (
        SeededArchitectureRuleCandidate,
        Vec<SeedRuleCandidateRepair>,
    ),
    (String, String),
> {
    let role_slug = raw_candidate
        .get("target_role_key")
        .and_then(Value::as_str)
        .map(str::to_string);
    let candidate_slug = raw_candidate
        .pointer("/metadata/slug")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut repairs = Vec::new();
    rewrite_seed_candidate_conditions(
        candidate_index,
        role_slug.as_deref(),
        candidate_slug.as_deref(),
        &mut raw_candidate,
        &mut repairs,
    )?;
    let candidate = serde_json::from_value::<SeededArchitectureRuleCandidate>(raw_candidate)
        .map_err(|error| ("rule_candidates".to_string(), error.to_string()))?;
    validate_rule_shape(
        &format!("rule_candidates[{candidate_index}]"),
        &candidate.candidate_selector,
        &candidate.positive_conditions,
        &candidate.negative_conditions,
        &candidate.score,
    )
    .map_err(|error| ("rule_candidates".to_string(), error.to_string()))?;
    Ok((candidate, repairs))
}

fn rewrite_seed_candidate_conditions(
    candidate_index: usize,
    role_slug: Option<&str>,
    candidate_slug: Option<&str>,
    candidate: &mut Value,
    repairs: &mut Vec<SeedRuleCandidateRepair>,
) -> std::result::Result<(), (String, String)> {
    rewrite_condition_array(
        candidate_index,
        role_slug,
        candidate_slug,
        candidate.pointer_mut("/candidate_selector/required_facts"),
        "candidate_selector.required_facts",
        repairs,
    )?;
    if let Some(groups) = candidate
        .pointer_mut("/candidate_selector/required_fact_any_groups")
        .and_then(Value::as_array_mut)
    {
        for (group_index, group) in groups.iter_mut().enumerate() {
            rewrite_condition_array(
                candidate_index,
                role_slug,
                candidate_slug,
                Some(group),
                &format!("candidate_selector.required_fact_any_groups[{group_index}]"),
                repairs,
            )?;
        }
    }
    rewrite_condition_array(
        candidate_index,
        role_slug,
        candidate_slug,
        candidate.get_mut("positive_conditions"),
        "positive_conditions",
        repairs,
    )?;
    rewrite_condition_array(
        candidate_index,
        role_slug,
        candidate_slug,
        candidate.get_mut("negative_conditions"),
        "negative_conditions",
        repairs,
    )?;
    Ok(())
}

fn rewrite_condition_array(
    candidate_index: usize,
    role_slug: Option<&str>,
    candidate_slug: Option<&str>,
    value: Option<&mut Value>,
    field_path: &str,
    repairs: &mut Vec<SeedRuleCandidateRepair>,
) -> std::result::Result<(), (String, String)> {
    let Some(Value::Array(conditions)) = value else {
        return Ok(());
    };
    for (condition_index, condition) in conditions.iter_mut().enumerate() {
        let condition_path = format!("{field_path}[{condition_index}]");
        rewrite_condition_object(
            candidate_index,
            role_slug,
            candidate_slug,
            condition,
            &condition_path,
            repairs,
        )?;
    }
    Ok(())
}

fn rewrite_condition_object(
    candidate_index: usize,
    role_slug: Option<&str>,
    candidate_slug: Option<&str>,
    condition: &mut Value,
    field_path: &str,
    repairs: &mut Vec<SeedRuleCandidateRepair>,
) -> std::result::Result<(), (String, String)> {
    let Some(object) = condition.as_object_mut() else {
        return Err((
            field_path.to_string(),
            "condition must be an object".to_string(),
        ));
    };

    if object.contains_key("predicate") {
        let predicate_condition =
            serde_json::from_value::<SeededRoleRulePredicateCondition>(condition.clone())
                .map_err(|error| (field_path.to_string(), error.to_string()))?;
        let fact_condition =
            role_fact_condition_from_seed_predicate(field_path, &predicate_condition)
                .map_err(|error| (field_path.to_string(), error.to_string()))?;
        *condition = role_rule_condition_from_fact_condition(fact_condition);
        return Ok(());
    }

    if object.contains_key("key") || object.contains_key("op") {
        let from_op = object
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let kind = object
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let key = object
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind == "signature" && key == "contains" && from_op == "contains" {
            object.insert("op".to_string(), Value::String("eq".to_string()));
            repairs.push(SeedRuleCandidateRepair {
                candidate_index,
                role_slug: role_slug.map(str::to_string),
                candidate_slug: candidate_slug.map(str::to_string),
                field_path: field_path.to_string(),
                from: "signature.contains:contains".to_string(),
                to: "signature.contains:eq".to_string(),
                reason: "signature.contains supports exact token matches; repaired legacy contains op to eq".to_string(),
            });
        }

        let role_condition = serde_json::from_value::<RoleRuleCondition>(condition.clone())
            .map_err(|error| (field_path.to_string(), error.to_string()))?;
        let fact_condition = fact_condition_from_rule_condition(field_path, &role_condition)
            .map_err(|error| (field_path.to_string(), error.to_string()))?;
        validate_supported_fact_condition(field_path, &fact_condition)
            .map_err(|error| (field_path.to_string(), error.to_string()))?;
    }

    Ok(())
}

fn role_rule_condition_from_fact_condition(condition: RoleFactCondition) -> Value {
    json!({
        "kind": condition.kind,
        "key": condition.key,
        "op": fact_condition_op_name(condition.op),
        "value": condition.value,
        "score": condition.score,
    })
}

fn compact_raw_excerpt(value: &Value) -> String {
    const MAX_RAW_CANDIDATE_EXCERPT_CHARS: usize = 500;
    let raw = serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string());
    if raw.chars().count() <= MAX_RAW_CANDIDATE_EXCERPT_CHARS {
        return raw;
    }
    let mut excerpt = raw
        .chars()
        .take(MAX_RAW_CANDIDATE_EXCERPT_CHARS)
        .collect::<String>();
    excerpt.push_str("...");
    excerpt
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
    supported_facts::ops_for(kind, key)
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

fn role_fact_condition_op_from_name(value: &str) -> Option<RoleFactConditionOp> {
    match value {
        "eq" => Some(RoleFactConditionOp::Eq),
        "contains" => Some(RoleFactConditionOp::Contains),
        "prefix" => Some(RoleFactConditionOp::Prefix),
        "suffix" => Some(RoleFactConditionOp::Suffix),
        "gte" => Some(RoleFactConditionOp::Gte),
        "lte" => Some(RoleFactConditionOp::Lte),
        _ => None,
    }
}
