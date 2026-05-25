use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::contracts::RoleCandidateDescriptor;
use super::super::taxonomy::{
    ArchitectureRoleReconcileMetrics, SeededArchitectureRuleCandidate, TargetKind,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRuleMiningRequest {
    pub repo_id: String,
    pub contract_version: String,
    pub coverage_metrics: ArchitectureRoleReconcileMetrics,
    pub active_roles: Vec<RoleCandidateDescriptor>,
    pub active_rules_summary: Vec<Value>,
    pub clusters: Vec<RoleRuleMiningCluster>,
    pub supported_facts: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRuleMiningCluster {
    pub cluster_key: String,
    pub reason_family: String,
    pub target_kind: TargetKind,
    pub stable_path_prefix: Option<String>,
    pub language: Option<String>,
    pub canonical_kind: Option<String>,
    pub file_role: Option<String>,
    pub analysis_mode: Option<String>,
    pub symbol_suffix: Option<String>,
    pub dependency_kinds: Vec<String>,
    pub candidate_role_ids: Vec<String>,
    pub representative_target_ids: Vec<String>,
    pub member_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRuleMiningProposalSet {
    pub rule_candidates: Vec<SeededArchitectureRuleCandidate>,
    pub skipped_clusters: Vec<RoleRuleMiningSkippedCluster>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRuleMiningSkippedCluster {
    pub cluster_key: String,
    pub reason: String,
}
