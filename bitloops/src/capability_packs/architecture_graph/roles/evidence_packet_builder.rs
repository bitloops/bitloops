use std::collections::BTreeSet;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::contracts::{
    RoleAdjudicationRequest, RoleCandidateDescriptor, RoleFactsReader, RoleTaxonomyReader,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidencePacketLimits {
    pub max_facts: usize,
    pub max_rule_signals: usize,
    pub max_dependency_items: usize,
    pub max_related_artefacts: usize,
    pub max_source_snippets: usize,
    pub max_snippet_chars: usize,
    pub max_candidate_roles: usize,
}

impl Default for EvidencePacketLimits {
    fn default() -> Self {
        Self {
            max_facts: 64,
            max_rule_signals: 64,
            max_dependency_items: 64,
            max_related_artefacts: 32,
            max_source_snippets: 8,
            max_snippet_chars: 4096,
            max_candidate_roles: 128,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleEvidencePacket {
    pub request: RoleAdjudicationRequest,
    pub candidate_roles: Vec<RoleCandidateDescriptor>,
    #[serde(default)]
    pub facts: Vec<Value>,
    #[serde(default)]
    pub rule_signals: Vec<Value>,
    #[serde(default)]
    pub dependency_context: Vec<Value>,
    #[serde(default)]
    pub related_artefacts: Vec<Value>,
    #[serde(default)]
    pub source_snippets: Vec<String>,
    #[serde(default)]
    pub reachability: Option<Value>,
}

pub struct RoleEvidencePacketBuilder<'a> {
    pub taxonomy: &'a dyn RoleTaxonomyReader,
    pub facts: &'a dyn RoleFactsReader,
    pub limits: EvidencePacketLimits,
}

impl<'a> RoleEvidencePacketBuilder<'a> {
    pub async fn build(&self, request: &RoleAdjudicationRequest) -> Result<RoleEvidencePacket> {
        let active_roles = self
            .taxonomy
            .load_active_roles(&request.repo_id, request.generation)
            .await?;

        let role_candidates =
            candidate_roles_for_request(request, &active_roles, self.limits.max_candidate_roles);

        let mut facts_bundle = self.facts.load_facts(request).await?;
        facts_bundle.facts.truncate(self.limits.max_facts);
        facts_bundle
            .rule_signals
            .truncate(self.limits.max_rule_signals);
        facts_bundle
            .dependency_context
            .truncate(self.limits.max_dependency_items);
        facts_bundle
            .related_artefacts
            .truncate(self.limits.max_related_artefacts);

        let mut source_snippets = facts_bundle.source_snippets;
        source_snippets.truncate(self.limits.max_source_snippets);
        source_snippets = trim_snippets(source_snippets, self.limits.max_snippet_chars);

        Ok(RoleEvidencePacket {
            request: request.clone(),
            candidate_roles: role_candidates,
            facts: facts_bundle.facts,
            rule_signals: facts_bundle
                .rule_signals
                .into_iter()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()?,
            dependency_context: facts_bundle.dependency_context,
            related_artefacts: facts_bundle.related_artefacts,
            source_snippets,
            reachability: facts_bundle.reachability,
        })
    }
}

fn candidate_roles_for_request(
    request: &RoleAdjudicationRequest,
    active_roles: &[RoleCandidateDescriptor],
    max_candidate_roles: usize,
) -> Vec<RoleCandidateDescriptor> {
    if max_candidate_roles == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut selected = BTreeSet::new();

    for role_id in &request.candidate_role_ids {
        if selected.contains(role_id) {
            continue;
        }
        if let Some(role) = active_roles.iter().find(|role| role.role_id == *role_id) {
            selected.insert(role.role_id.clone());
            out.push(role.clone());
            if out.len() >= max_candidate_roles {
                return out;
            }
        }
    }

    for role in active_roles {
        if selected.insert(role.role_id.clone()) {
            out.push(role.clone());
            if out.len() >= max_candidate_roles {
                return out;
            }
        }
    }

    out
}

fn trim_snippets(snippets: Vec<String>, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return Vec::new();
    }

    let mut remaining = max_chars;
    let mut out = Vec::new();
    for snippet in snippets {
        if remaining == 0 {
            break;
        }
        if snippet.len() <= remaining {
            remaining -= snippet.len();
            out.push(snippet);
            continue;
        }
        out.push(snippet[..remaining].to_string());
        break;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_packs::architecture_graph::roles::contracts::{
        RoleCandidateDescriptor, RoleFactsBundle, RuleSignalFact,
    };

    struct FakeTaxonomy;
    impl RoleTaxonomyReader for FakeTaxonomy {
        fn load_active_roles<'a>(
            &'a self,
            _repo_id: &'a str,
            _generation: u64,
        ) -> crate::capability_packs::architecture_graph::roles::contracts::RoleBoxFuture<
            'a,
            Vec<RoleCandidateDescriptor>,
        > {
            Box::pin(async move {
                Ok(vec![
                    RoleCandidateDescriptor {
                        role_id: "command_dispatcher".to_string(),
                        canonical_key: "command_dispatcher".to_string(),
                        family: "application".to_string(),
                        display_name: "Command Dispatcher".to_string(),
                        description: "Dispatches application commands".to_string(),
                    },
                    RoleCandidateDescriptor {
                        role_id: "entrypoint".to_string(),
                        canonical_key: "entrypoint_http_api".to_string(),
                        family: "interface".to_string(),
                        display_name: "HTTP API Entry Point".to_string(),
                        description: "Accepts inbound HTTP API requests".to_string(),
                    },
                    RoleCandidateDescriptor {
                        role_id: "storage_adapter".to_string(),
                        canonical_key: "storage_adapter".to_string(),
                        family: "infrastructure".to_string(),
                        display_name: "Storage Adapter".to_string(),
                        description: "Persists and loads data".to_string(),
                    },
                ])
            })
        }
    }

    struct FakeFacts;
    impl RoleFactsReader for FakeFacts {
        fn load_facts<'a>(
            &'a self,
            _request: &'a RoleAdjudicationRequest,
        ) -> crate::capability_packs::architecture_graph::roles::contracts::RoleBoxFuture<
            'a,
            RoleFactsBundle,
        > {
            Box::pin(async move {
                Ok(RoleFactsBundle {
                    facts: vec![
                        Value::String("f1".to_string()),
                        Value::String("f2".to_string()),
                    ],
                    rule_signals: vec![RuleSignalFact {
                        rule_id: "r1".to_string(),
                        polarity: "positive".to_string(),
                        weight: 0.8,
                        evidence: Value::Null,
                    }],
                    dependency_context: vec![Value::String("dep".to_string())],
                    related_artefacts: vec![Value::String("a1".to_string())],
                    source_snippets: vec!["0123456789".to_string(), "abcdefghij".to_string()],
                    reachability: Some(Value::String("reachable".to_string())),
                })
            })
        }
    }

    fn request() -> RoleAdjudicationRequest {
        RoleAdjudicationRequest {
            repo_id: "repo".to_string(),
            generation: 7,
            target_kind: Some("artefact".to_string()),
            artefact_id: Some("a-1".to_string()),
            symbol_id: None,
            path: Some("src/main.rs".to_string()),
            language: Some("rust".to_string()),
            canonical_kind: Some("function".to_string()),
            reason: crate::capability_packs::architecture_graph::roles::contracts::AdjudicationReason::Conflict,
            deterministic_confidence: Some(0.42),
            candidate_role_ids: vec!["entrypoint".to_string(), "missing".to_string()],
            current_assignment: None,
        }
    }

    #[tokio::test]
    async fn evidence_packet_is_bounded_and_uses_active_roles() {
        let builder = RoleEvidencePacketBuilder {
            taxonomy: &FakeTaxonomy,
            facts: &FakeFacts,
            limits: EvidencePacketLimits {
                max_source_snippets: 2,
                max_snippet_chars: 12,
                max_candidate_roles: 2,
                ..EvidencePacketLimits::default()
            },
        };

        let packet = builder
            .build(&request())
            .await
            .expect("packet should build");

        assert_eq!(packet.candidate_roles.len(), 2);
        assert_eq!(packet.candidate_roles[0].role_id, "entrypoint");
        assert_eq!(
            packet.candidate_roles[0].canonical_key,
            "entrypoint_http_api"
        );
        assert_eq!(
            packet.candidate_roles[0].display_name,
            "HTTP API Entry Point"
        );
        assert_eq!(packet.candidate_roles[1].role_id, "command_dispatcher");
        assert_eq!(packet.candidate_roles[1].family, "application");
        assert_eq!(packet.source_snippets.join(""), "0123456789ab");
        assert_eq!(packet.rule_signals.len(), 1);

        let zero_candidate_packet = RoleEvidencePacketBuilder {
            taxonomy: &FakeTaxonomy,
            facts: &FakeFacts,
            limits: EvidencePacketLimits {
                max_candidate_roles: 0,
                ..EvidencePacketLimits::default()
            },
        }
        .build(&request())
        .await
        .expect("packet should build");

        assert!(zero_candidate_packet.candidate_roles.is_empty());
    }
}
