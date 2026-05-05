use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_role_id_does_not_depend_on_display_name() {
        let first = stable_role_id("repo-1", "application", "entrypoint");
        let second = stable_role_id("repo-1", "Application", "Entrypoint");
        assert_eq!(first, second);
    }

    #[test]
    fn role_fragment_normalization_preserves_separator_boundaries() {
        assert_eq!(normalize_role_fragment("Entry Point"), "entry_point");
        assert_ne!(
            normalize_role_fragment("Entry Point"),
            normalize_role_fragment("Entrypoint")
        );
    }

    #[test]
    fn db_enum_values_match_schema_contract() {
        assert_eq!(RoleLifecycle::Active.as_db(), "active");
        assert_eq!(RoleLifecycle::Deprecated.as_db(), "deprecated");
        assert_eq!(RoleLifecycle::Removed.as_db(), "removed");
        assert_eq!(AssignmentStatus::NeedsReview.as_db(), "needs_review");
        assert_eq!(AssignmentSource::Llm.as_db(), "llm");
        assert_eq!(TargetKind::Artefact.as_db(), "artefact");
    }

    #[test]
    fn assignment_id_is_stable_for_same_target() {
        let target = RoleTarget::artefact("art-1", "sym-1", "src/main.rs");
        let first = assignment_id("repo-1", "role-1", &target);
        let second = assignment_id("repo-1", "role-1", &target);
        assert_eq!(first, second);
    }

    #[test]
    fn symbol_target_constructor_uses_symbol_target_kind() {
        let target = RoleTarget::symbol("art-1", "sym-1", "src/main.rs");
        assert_eq!(target.target_kind, TargetKind::Symbol);
        assert_eq!(target.artefact_id.as_deref(), Some("art-1"));
        assert_eq!(target.symbol_id.as_deref(), Some("sym-1"));
    }
}
