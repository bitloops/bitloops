use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::contracts::RoleAdjudicationRequest;

mod seed;
pub use seed::{
    RoleRuleCandidateSelector, RoleRuleCondition, RoleRuleScore, RoleSplitSpecFile,
    RoleSplitTargetRole, RuleSpecFile, SeededArchitectureRole, SeededArchitectureRuleCandidate,
    SeededArchitectureTaxonomy, architecture_roles_seed_schema, generic_role_family_examples,
    validate_role_split_spec, validate_rule_spec_file, validate_seeded_taxonomy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleLifecycle {
    Active,
    Deprecated,
    Removed,
}

impl RoleLifecycle {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deprecated => "deprecated",
            Self::Removed => "removed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleRuleLifecycle {
    Draft,
    Active,
    Disabled,
    Deprecated,
}

impl RoleRuleLifecycle {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Deprecated => "deprecated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentStatus {
    Active,
    Stale,
    NeedsReview,
    Rejected,
}

impl AssignmentStatus {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Stale => "stale",
            Self::NeedsReview => "needs_review",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentSource {
    Rule,
    Llm,
    Human,
    Migration,
}

impl AssignmentSource {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Llm => "llm",
            Self::Human => "human",
            Self::Migration => "migration",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentPriority {
    Primary,
    Secondary,
}

impl AssignmentPriority {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    File,
    Artefact,
    Symbol,
}

impl TargetKind {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Artefact => "artefact",
            Self::Symbol => "symbol",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleSignalPolarity {
    Positive,
    Negative,
}

impl RoleSignalPolarity {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Draft,
    Previewed,
    Applied,
    Rejected,
}

impl ProposalStatus {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Previewed => "previewed",
            Self::Applied => "applied",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RoleTarget {
    pub target_kind: TargetKind,
    pub artefact_id: Option<String>,
    pub symbol_id: Option<String>,
    pub path: String,
}

impl RoleTarget {
    pub fn file(path: impl Into<String>) -> Self {
        Self {
            target_kind: TargetKind::File,
            artefact_id: None,
            symbol_id: None,
            path: path.into(),
        }
    }

    pub fn artefact(
        artefact_id: impl Into<String>,
        symbol_id: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            target_kind: TargetKind::Artefact,
            artefact_id: Some(artefact_id.into()),
            symbol_id: Some(symbol_id.into()),
            path: path.into(),
        }
    }

    pub fn symbol(
        artefact_id: impl Into<String>,
        symbol_id: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            target_kind: TargetKind::Symbol,
            artefact_id: Some(artefact_id.into()),
            symbol_id: Some(symbol_id.into()),
            path: path.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureRole {
    pub repo_id: String,
    pub role_id: String,
    pub family: String,
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub lifecycle: RoleLifecycle,
    pub provenance: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureRoleDetectionRule {
    pub repo_id: String,
    pub rule_id: String,
    pub role_id: String,
    pub version: i64,
    pub lifecycle: RoleRuleLifecycle,
    pub priority: i64,
    pub score: f64,
    pub candidate_selector: Value,
    pub positive_conditions: Value,
    pub negative_conditions: Value,
    pub provenance: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureArtefactFact {
    pub repo_id: String,
    pub fact_id: String,
    pub target: RoleTarget,
    pub language: Option<String>,
    pub fact_kind: String,
    pub fact_key: String,
    pub fact_value: String,
    pub source: String,
    pub confidence: f64,
    pub evidence: Value,
    pub generation_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureRoleRuleSignal {
    pub repo_id: String,
    pub signal_id: String,
    pub rule_id: String,
    pub rule_version: i64,
    pub role_id: String,
    pub target: RoleTarget,
    pub polarity: RoleSignalPolarity,
    pub score: f64,
    pub evidence: Value,
    pub generation_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureRoleAssignment {
    pub repo_id: String,
    pub assignment_id: String,
    pub role_id: String,
    pub target: RoleTarget,
    pub priority: AssignmentPriority,
    pub status: AssignmentStatus,
    pub source: AssignmentSource,
    pub confidence: f64,
    pub evidence: Value,
    pub provenance: Value,
    pub classifier_version: String,
    pub rule_version: Option<i64>,
    pub generation_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureRoleAssignmentHistory {
    pub repo_id: String,
    pub history_id: String,
    pub assignment_id: String,
    pub role_id: String,
    pub target: RoleTarget,
    pub previous_status: Option<AssignmentStatus>,
    pub new_status: AssignmentStatus,
    pub previous_confidence: Option<f64>,
    pub new_confidence: f64,
    pub change_kind: String,
    pub evidence: Value,
    pub provenance: Value,
    pub generation_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureRoleChangeProposal {
    pub repo_id: String,
    pub proposal_id: String,
    pub proposal_kind: String,
    pub status: ProposalStatus,
    pub payload: Value,
    pub impact_preview: Value,
    pub provenance: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleCandidateSelector {
    #[serde(default)]
    pub target_kinds: Vec<TargetKind>,
    #[serde(default)]
    pub path_prefixes: Vec<String>,
    #[serde(default)]
    pub path_suffixes: Vec<String>,
    #[serde(default)]
    pub required_facts: Vec<RoleFactCondition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_fact_any_groups: Vec<Vec<RoleFactCondition>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleFactCondition {
    pub kind: String,
    pub key: String,
    pub op: RoleFactConditionOp,
    pub value: String,
    #[serde(default = "default_condition_score")]
    pub score: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleFactConditionOp {
    Eq,
    Contains,
    Prefix,
    Suffix,
    Gte,
    Lte,
}

pub fn default_condition_score() -> f64 {
    0.10
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureRoleReconcileMetrics {
    pub full_reconcile: bool,
    pub affected_paths: usize,
    pub refreshed_paths: usize,
    pub removed_paths: usize,
    pub skipped_unchanged_paths: usize,
    pub facts_written: usize,
    pub facts_deleted: usize,
    pub rules_loaded: usize,
    pub signals_written: usize,
    pub signals_deleted: usize,
    pub assignments_written: usize,
    pub assignments_marked_stale: usize,
    pub assignment_history_rows: usize,
    pub adjudication_candidates: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureRoleReconcileOutcome {
    pub metrics: ArchitectureRoleReconcileMetrics,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub adjudication_requests: Vec<RoleAdjudicationRequest>,
}

pub fn stable_role_id(repo_id: &str, family: &str, slug: &str) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "architecture_role|{}|{}|{}",
        repo_id,
        normalize_role_fragment(family),
        normalize_role_fragment(slug)
    ))
}

pub fn role_alias_id(repo_id: &str, alias: &str) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "architecture_role_alias|{}|{}",
        repo_id,
        normalize_role_fragment(alias)
    ))
}

pub fn fact_id(
    repo_id: &str,
    target: &RoleTarget,
    fact_kind: &str,
    fact_key: &str,
    fact_value: &str,
) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "architecture_role_fact|{}|{}|{}|{}|{}|{}|{}|{}",
        repo_id,
        target.target_kind.as_db(),
        target.artefact_id.as_deref().unwrap_or(""),
        target.symbol_id.as_deref().unwrap_or(""),
        target.path,
        normalize_role_fragment(fact_kind),
        normalize_role_fragment(fact_key),
        fact_value.trim().to_ascii_lowercase()
    ))
}

pub fn rule_id(repo_id: &str, role_id: &str, slug: &str) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "architecture_role_detection_rule|{}|{}|{}",
        repo_id,
        role_id,
        normalize_role_fragment(slug)
    ))
}

pub fn rule_signal_id(
    repo_id: &str,
    rule_id: &str,
    rule_version: i64,
    role_id: &str,
    target: &RoleTarget,
    polarity: RoleSignalPolarity,
) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "architecture_role_rule_signal|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        repo_id,
        rule_id,
        rule_version,
        role_id,
        target.target_kind.as_db(),
        target.artefact_id.as_deref().unwrap_or(""),
        target.symbol_id.as_deref().unwrap_or(""),
        target.path,
        polarity.as_db()
    ))
}

pub fn assignment_id(repo_id: &str, role_id: &str, target: &RoleTarget) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "architecture_role_assignment|{}|{}|{}|{}|{}|{}",
        repo_id,
        role_id,
        target.target_kind.as_db(),
        target.artefact_id.as_deref().unwrap_or(""),
        target.symbol_id.as_deref().unwrap_or(""),
        target.path
    ))
}

pub fn assignment_history_id(
    repo_id: &str,
    assignment_id: &str,
    generation_seq: u64,
    change_kind: &str,
) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "architecture_role_assignment_history|{repo_id}|{assignment_id}|{generation_seq}|{change_kind}"
    ))
}

pub fn proposal_id(repo_id: &str, proposal_kind: &str, payload: &Value) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "architecture_role_change_proposal|{}|{}|{}",
        repo_id,
        normalize_role_fragment(proposal_kind),
        payload
    ))
}

pub fn normalize_role_fragment(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchableArtefact {
    pub artefact_id: String,
    pub path: String,
    pub language: Option<String>,
    pub canonical_kind: Option<String>,
    pub symbol_fqn: Option<String>,
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
    let selector = role_rule_candidate_selector_contract(selector);
    let Ok(positive_conditions) = role_rule_conditions_contract(positive_conditions) else {
        return false;
    };
    let Ok(negative_conditions) = role_rule_conditions_contract(negative_conditions) else {
        return false;
    };
    role_rule_contract_matches(
        &selector,
        &positive_conditions,
        &negative_conditions,
        artefact,
    )
}

pub fn role_rule_contract_matches(
    selector: &RoleCandidateSelector,
    positive_conditions: &[RoleFactCondition],
    negative_conditions: &[RoleFactCondition],
    artefact: &MatchableArtefact,
) -> bool {
    let target = RoleTarget::artefact(
        artefact.artefact_id.clone(),
        artefact.artefact_id.clone(),
        artefact.path.clone(),
    );
    let facts = facts_for_matchable_artefact(artefact);

    if !fact_selector_matches(selector, &target, &facts) {
        return false;
    }
    if !positive_conditions.is_empty()
        && !positive_conditions
            .iter()
            .any(|condition| fact_condition_matches(condition, &facts))
    {
        return false;
    }
    if negative_conditions
        .iter()
        .any(|condition| fact_condition_matches(condition, &facts))
    {
        return false;
    }
    true
}

pub fn role_rule_candidate_selector_contract(
    selector: &RoleRuleCandidateSelector,
) -> RoleCandidateSelector {
    let mut required_facts = Vec::new();
    let mut required_fact_any_groups = Vec::new();
    push_required_fact_group(
        &mut required_facts,
        &mut required_fact_any_groups,
        selector
            .path_contains
            .iter()
            .map(|value| RoleFactCondition {
                kind: "path".to_string(),
                key: "full".to_string(),
                op: RoleFactConditionOp::Contains,
                value: value.clone(),
                score: 1.0,
            })
            .collect(),
    );
    push_required_fact_group(
        &mut required_facts,
        &mut required_fact_any_groups,
        selector
            .languages
            .iter()
            .map(|value| RoleFactCondition {
                kind: "language".to_string(),
                key: "resolved".to_string(),
                op: RoleFactConditionOp::Eq,
                value: value.clone(),
                score: 1.0,
            })
            .collect(),
    );
    push_required_fact_group(
        &mut required_facts,
        &mut required_fact_any_groups,
        selector
            .canonical_kinds
            .iter()
            .map(|value| RoleFactCondition {
                kind: "artefact".to_string(),
                key: "canonical_kind".to_string(),
                op: RoleFactConditionOp::Eq,
                value: value.clone(),
                score: 1.0,
            })
            .collect(),
    );
    push_required_fact_group(
        &mut required_facts,
        &mut required_fact_any_groups,
        selector
            .symbol_fqn_contains
            .iter()
            .map(|value| RoleFactCondition {
                kind: "symbol".to_string(),
                key: "fqn".to_string(),
                op: RoleFactConditionOp::Contains,
                value: value.clone(),
                score: 1.0,
            })
            .collect(),
    );

    RoleCandidateSelector {
        target_kinds: Vec::new(),
        path_prefixes: selector.path_prefixes.clone(),
        path_suffixes: selector.path_suffixes.clone(),
        required_facts,
        required_fact_any_groups,
    }
}

fn push_required_fact_group(
    required_facts: &mut Vec<RoleFactCondition>,
    required_fact_any_groups: &mut Vec<Vec<RoleFactCondition>>,
    mut conditions: Vec<RoleFactCondition>,
) {
    match conditions.len() {
        0 => {}
        1 => required_facts.push(conditions.remove(0)),
        _ => required_fact_any_groups.push(conditions),
    }
}

pub fn role_rule_conditions_contract(
    conditions: &[RoleRuleCondition],
) -> Result<Vec<RoleFactCondition>> {
    conditions
        .iter()
        .map(role_rule_condition_contract)
        .collect()
}

pub fn role_rule_condition_contract(condition: &RoleRuleCondition) -> Result<RoleFactCondition> {
    let value = condition
        .value
        .as_str()
        .ok_or_else(|| {
            anyhow!(
                "rule condition `{}` must use a string value",
                condition.kind
            )
        })?
        .to_string();
    let (kind, key, op) = match condition.kind.trim() {
        "path_contains" => ("path", "full", RoleFactConditionOp::Contains),
        "path_prefix" => ("path", "full", RoleFactConditionOp::Prefix),
        "path_suffix" => ("path", "full", RoleFactConditionOp::Suffix),
        "language_is" => ("language", "resolved", RoleFactConditionOp::Eq),
        "canonical_kind_is" => ("artefact", "canonical_kind", RoleFactConditionOp::Eq),
        "symbol_fqn_contains" => ("symbol", "fqn", RoleFactConditionOp::Contains),
        other => bail!("unsupported rule condition kind `{other}`"),
    };
    Ok(RoleFactCondition {
        kind: kind.to_string(),
        key: key.to_string(),
        op,
        value,
        score: 1.0,
    })
}

fn facts_for_matchable_artefact(artefact: &MatchableArtefact) -> Vec<ArchitectureArtefactFact> {
    let target = RoleTarget::artefact(
        artefact.artefact_id.clone(),
        artefact.artefact_id.clone(),
        artefact.path.clone(),
    );
    let mut facts = Vec::new();
    push_preview_fact(&mut facts, &target, "path", "full", &artefact.path);
    for segment in artefact
        .path
        .split('/')
        .filter(|segment| !segment.is_empty())
    {
        push_preview_fact(&mut facts, &target, "path", "segment", segment);
    }
    if let Some(language) = artefact.language.as_deref() {
        push_preview_fact(&mut facts, &target, "language", "resolved", language);
    }
    if let Some(canonical_kind) = artefact.canonical_kind.as_deref() {
        push_preview_fact(
            &mut facts,
            &target,
            "artefact",
            "canonical_kind",
            canonical_kind,
        );
    }
    if let Some(symbol_fqn) = artefact.symbol_fqn.as_deref() {
        push_preview_fact(&mut facts, &target, "symbol", "fqn", symbol_fqn);
    }
    facts
}

fn push_preview_fact(
    facts: &mut Vec<ArchitectureArtefactFact>,
    target: &RoleTarget,
    kind: &str,
    key: &str,
    value: &str,
) {
    facts.push(ArchitectureArtefactFact {
        repo_id: String::new(),
        fact_id: format!("{kind}:{key}:{value}"),
        target: target.clone(),
        language: None,
        fact_kind: kind.to_string(),
        fact_key: key.to_string(),
        fact_value: value.to_string(),
        source: "rule_preview".to_string(),
        confidence: 1.0,
        evidence: json!([]),
        generation_seq: 0,
    });
}

fn fact_selector_matches(
    selector: &RoleCandidateSelector,
    target: &RoleTarget,
    facts: &[ArchitectureArtefactFact],
) -> bool {
    if !selector.target_kinds.is_empty() && !selector.target_kinds.contains(&target.target_kind) {
        return false;
    }
    if !selector.path_prefixes.is_empty()
        && !selector
            .path_prefixes
            .iter()
            .any(|prefix| target.path.starts_with(prefix))
    {
        return false;
    }
    if !selector.path_suffixes.is_empty()
        && !selector
            .path_suffixes
            .iter()
            .any(|suffix| target.path.ends_with(suffix))
    {
        return false;
    }
    selector
        .required_facts
        .iter()
        .all(|condition| fact_condition_matches(condition, facts))
        && selector.required_fact_any_groups.iter().all(|group| {
            group.is_empty()
                || group
                    .iter()
                    .any(|condition| fact_condition_matches(condition, facts))
        })
}

fn fact_condition_matches(
    condition: &RoleFactCondition,
    facts: &[ArchitectureArtefactFact],
) -> bool {
    facts.iter().any(|fact| {
        if fact.fact_kind != condition.kind || fact.fact_key != condition.key {
            return false;
        }
        match condition.op {
            RoleFactConditionOp::Eq => fact.fact_value == condition.value,
            RoleFactConditionOp::Contains => fact.fact_value.contains(&condition.value),
            RoleFactConditionOp::Prefix => fact.fact_value.starts_with(&condition.value),
            RoleFactConditionOp::Suffix => fact.fact_value.ends_with(&condition.value),
            RoleFactConditionOp::Gte => {
                numeric_fact_matches(&fact.fact_value, &condition.value, |left, right| {
                    left >= right
                })
            }
            RoleFactConditionOp::Lte => {
                numeric_fact_matches(&fact.fact_value, &condition.value, |left, right| {
                    left <= right
                })
            }
        }
    })
}

fn numeric_fact_matches(left: &str, right: &str, predicate: impl FnOnce(f64, f64) -> bool) -> bool {
    let Ok(left) = left.parse::<f64>() else {
        return false;
    };
    let Ok(right) = right.parse::<f64>() else {
        return false;
    };
    predicate(left, right)
}

#[cfg(test)]
mod tests;
