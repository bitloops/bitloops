use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

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
    pub affected_paths: usize,
    pub facts_written: usize,
    pub facts_deleted: usize,
    pub rules_loaded: usize,
    pub signals_written: usize,
    pub assignments_written: usize,
    pub assignments_marked_stale: usize,
    pub assignment_history_rows: usize,
    pub adjudication_candidates: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureRoleReconcileOutcome {
    pub metrics: ArchitectureRoleReconcileMetrics,
    pub warnings: Vec<String>,
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

#[cfg(test)]
mod tests;
