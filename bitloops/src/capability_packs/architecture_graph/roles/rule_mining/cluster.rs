use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::super::contracts::AdjudicationReason;
use super::super::taxonomy::{ArchitectureArtefactFact, RoleTarget, TargetKind};
use super::contracts::RoleRuleMiningCluster;

#[derive(Debug, Clone, PartialEq)]
pub struct RoleRuleMiningClusterInput {
    pub reason: AdjudicationReason,
    pub target: RoleTarget,
    pub facts: Vec<ArchitectureArtefactFact>,
    pub candidate_role_ids: Vec<String>,
    pub deterministic_confidence: Option<f64>,
    pub high_impact: bool,
}

pub fn build_rule_mining_clusters(
    repo_id: &str,
    inputs: Vec<RoleRuleMiningClusterInput>,
    max_representatives: usize,
) -> Vec<RoleRuleMiningCluster> {
    let mut grouped: BTreeMap<ClusterSignature, Vec<RoleRuleMiningClusterInput>> = BTreeMap::new();
    for input in inputs {
        grouped
            .entry(ClusterSignature::from_input(&input))
            .or_default()
            .push(input);
    }

    grouped
        .into_iter()
        .map(|(signature, mut members)| {
            members.sort_by(|left, right| {
                right
                    .high_impact
                    .cmp(&left.high_impact)
                    .then_with(|| fact_count(right).cmp(&fact_count(left)))
                    .then_with(|| {
                        right
                            .deterministic_confidence
                            .partial_cmp(&left.deterministic_confidence)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| left.target.path.cmp(&right.target.path))
                    .then_with(|| target_id(left).cmp(&target_id(right)))
            });
            let representative_target_ids = members
                .iter()
                .take(max_representatives)
                .map(target_id)
                .collect::<Vec<_>>();
            RoleRuleMiningCluster {
                cluster_key: signature.cluster_key(repo_id),
                reason_family: signature.reason_family,
                target_kind: signature.target_kind,
                stable_path_prefix: signature.stable_path_prefix,
                language: signature.language,
                canonical_kind: signature.canonical_kind,
                file_role: signature.file_role,
                analysis_mode: signature.analysis_mode,
                symbol_suffix: signature.symbol_suffix,
                dependency_kinds: signature.dependency_kinds.into_iter().collect(),
                candidate_role_ids: signature.candidate_role_ids,
                representative_target_ids,
                member_count: members.len(),
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ClusterSignature {
    reason_family: String,
    target_kind: TargetKind,
    stable_path_prefix: Option<String>,
    language: Option<String>,
    canonical_kind: Option<String>,
    file_role: Option<String>,
    analysis_mode: Option<String>,
    symbol_suffix: Option<String>,
    dependency_kinds: BTreeSet<String>,
    candidate_role_ids: Vec<String>,
}

impl ClusterSignature {
    fn from_input(input: &RoleRuleMiningClusterInput) -> Self {
        let mut facts = FactView::default();
        for fact in &input.facts {
            facts.observe(fact);
        }
        let mut candidate_role_ids = input.candidate_role_ids.clone();
        candidate_role_ids.sort();
        candidate_role_ids.dedup();
        Self {
            reason_family: reason_family(input.reason).to_string(),
            target_kind: input.target.target_kind,
            stable_path_prefix: stable_path_prefix(&input.target.path),
            language: facts.language,
            canonical_kind: facts.canonical_kind,
            file_role: facts.file_role,
            analysis_mode: facts.analysis_mode,
            symbol_suffix: facts.symbol_suffix,
            dependency_kinds: facts.dependency_kinds,
            candidate_role_ids,
        }
    }

    fn cluster_key(&self, repo_id: &str) -> String {
        let digest = Sha256::digest(format!("{repo_id}:{self:?}").as_bytes());
        format!("role-rule-cluster:{}", hex::encode(digest))
    }
}

#[derive(Default)]
struct FactView {
    language: Option<String>,
    canonical_kind: Option<String>,
    file_role: Option<String>,
    analysis_mode: Option<String>,
    symbol_suffix: Option<String>,
    dependency_kinds: BTreeSet<String>,
}

impl FactView {
    fn observe(&mut self, fact: &ArchitectureArtefactFact) {
        match (fact.fact_kind.as_str(), fact.fact_key.as_str()) {
            ("language", "resolved") => self.language = Some(fact.fact_value.clone()),
            ("artefact", "canonical_kind") => self.canonical_kind = Some(fact.fact_value.clone()),
            ("file", "role") => self.file_role = Some(fact.fact_value.clone()),
            ("file", "analysis_mode") => self.analysis_mode = Some(fact.fact_value.clone()),
            ("symbol", "name_suffix") => self.symbol_suffix = Some(fact.fact_value.clone()),
            ("dependency", "incoming_kind" | "outgoing_kind") => {
                self.dependency_kinds.insert(fact.fact_value.clone());
            }
            _ => {}
        }
    }
}

fn reason_family(reason: AdjudicationReason) -> &'static str {
    match reason {
        AdjudicationReason::Unknown | AdjudicationReason::HighImpact => "unknown",
        AdjudicationReason::Conflict => "conflict",
        AdjudicationReason::LowConfidence => "low_confidence",
        AdjudicationReason::NovelPattern => "unknown",
        AdjudicationReason::ManualReview => "manual_review",
    }
}

fn stable_path_prefix(path: &str) -> Option<String> {
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let first = parts.next()?;
    let second = parts.next()?;
    Some(format!("{first}/{second}"))
}

fn target_id(input: &RoleRuleMiningClusterInput) -> String {
    input
        .target
        .symbol_id
        .clone()
        .or_else(|| input.target.artefact_id.clone())
        .unwrap_or_else(|| input.target.path.clone())
}

fn fact_count(input: &RoleRuleMiningClusterInput) -> usize {
    input.facts.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fact(target: &RoleTarget, kind: &str, key: &str, value: &str) -> ArchitectureArtefactFact {
        ArchitectureArtefactFact {
            repo_id: "repo-1".to_string(),
            fact_id: format!("{kind}:{key}:{value}"),
            target: target.clone(),
            language: Some("rust".to_string()),
            fact_kind: kind.to_string(),
            fact_key: key.to_string(),
            fact_value: value.to_string(),
            source: "test".to_string(),
            confidence: 1.0,
            evidence: json!([]),
            generation_seq: 1,
        }
    }

    fn input(path: &str, suffix: &str) -> RoleRuleMiningClusterInput {
        let target =
            RoleTarget::artefact(format!("artefact-{path}"), format!("symbol-{path}"), path);
        RoleRuleMiningClusterInput {
            reason: AdjudicationReason::Unknown,
            facts: vec![
                fact(&target, "language", "resolved", "rust"),
                fact(&target, "symbol", "name_suffix", suffix),
            ],
            target,
            candidate_role_ids: Vec::new(),
            deterministic_confidence: None,
            high_impact: false,
        }
    }

    #[test]
    fn unknown_targets_cluster_by_path_kind_language_and_symbol_suffix() {
        let clusters = build_rule_mining_clusters(
            "repo-1",
            vec![
                input("src/cli/run_command.rs", "Command"),
                input("src/cli/init_command.rs", "Command"),
            ],
            3,
        );

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].reason_family, "unknown");
        assert_eq!(clusters[0].stable_path_prefix.as_deref(), Some("src/cli"));
        assert_eq!(clusters[0].language.as_deref(), Some("rust"));
        assert_eq!(clusters[0].symbol_suffix.as_deref(), Some("Command"));
        assert_eq!(clusters[0].member_count, 2);
    }

    #[test]
    fn cluster_representatives_are_bounded_and_stable() {
        let clusters = build_rule_mining_clusters(
            "repo-1",
            vec![
                input("src/cli/z_command.rs", "Command"),
                input("src/cli/a_command.rs", "Command"),
            ],
            1,
        );

        assert_eq!(clusters[0].representative_target_ids.len(), 1);
        assert!(clusters[0].representative_target_ids[0].contains("a_command"));
    }
}
