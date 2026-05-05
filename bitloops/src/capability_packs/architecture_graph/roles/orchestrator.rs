use anyhow::Result;
use serde_json::json;
use std::collections::BTreeSet;

use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
    ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
};
use crate::host::capability_host::gateways::{CapabilityWorkplaneGateway, CapabilityWorkplaneJob};
use crate::host::inference::InferenceGateway;

use super::adjudication_selector::{DeterministicRoleOutcomeInput, select_adjudication_reason};
use super::assignment_applier::apply_adjudication_result;
use super::contracts::{
    RoleAdjudicationFailure, RoleAdjudicationMailboxPayload, RoleAdjudicationRequest,
    RoleAssignmentWriteOutcome,
};
use super::evidence_packet_builder::{EvidencePacketLimits, RoleEvidencePacketBuilder};
use super::llm_executor::execute_llm_adjudication;
use super::queue_store::RoleAdjudicationQueueStore;
use super::response_validator::validate_adjudication_result;

pub struct RoleAdjudicationServices<'a> {
    pub queue: &'a dyn RoleAdjudicationQueueStore,
    pub taxonomy: &'a dyn super::contracts::RoleTaxonomyReader,
    pub facts: &'a dyn super::contracts::RoleFactsReader,
    pub writer: &'a dyn super::contracts::RoleAssignmentWriter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleAdjudicationEnqueueMetrics {
    pub selected: usize,
    pub enqueued: usize,
    pub deduped: usize,
}

pub fn enqueue_adjudication_jobs_for_delta(
    repo_id: &str,
    generation: u64,
    artefact_upserts: &[crate::host::capability_host::ChangedArtefact],
    workplane: &dyn CapabilityWorkplaneGateway,
    queue: &dyn RoleAdjudicationQueueStore,
) -> Result<RoleAdjudicationEnqueueMetrics> {
    let requests = role_requests_from_delta(repo_id, generation, artefact_upserts);
    let mut jobs = Vec::new();
    let mut deduped = 0usize;

    for request in &requests {
        let dedupe_key = request.scope_key();
        match queue.enqueue(request, &dedupe_key)? {
            super::contracts::RoleQueueEnqueueResult::Enqueued => {
                jobs.push(CapabilityWorkplaneJob::new_for_capability(
                    ARCHITECTURE_GRAPH_CAPABILITY_ID,
                    ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
                    Some(dedupe_key),
                    serde_json::to_value(RoleAdjudicationMailboxPayload {
                        request: request.clone(),
                    })?,
                ));
            }
            super::contracts::RoleQueueEnqueueResult::AlreadyQueued
            | super::contracts::RoleQueueEnqueueResult::AlreadyCompleted => {
                deduped += 1;
            }
        }
    }

    if !jobs.is_empty() {
        workplane.enqueue_jobs(jobs)?;
    }

    Ok(RoleAdjudicationEnqueueMetrics {
        selected: requests.len(),
        enqueued: requests.len().saturating_sub(deduped),
        deduped,
    })
}

fn role_requests_from_delta(
    repo_id: &str,
    generation: u64,
    artefact_upserts: &[crate::host::capability_host::ChangedArtefact],
) -> Vec<RoleAdjudicationRequest> {
    artefact_upserts
        .iter()
        .filter_map(|artefact| {
            let deterministic = infer_deterministic_outcome(artefact);
            let reason = select_adjudication_reason(&deterministic)?;
            Some(RoleAdjudicationRequest {
                repo_id: repo_id.to_string(),
                generation,
                artefact_id: Some(artefact.artefact_id.clone()),
                symbol_id: Some(artefact.symbol_id.clone()),
                path: Some(artefact.path.clone()),
                language: None,
                canonical_kind: artefact.canonical_kind.clone(),
                reason,
                deterministic_confidence: deterministic.best_confidence,
                candidate_role_ids: Vec::new(),
                current_assignment: None,
            })
        })
        .collect()
}

fn infer_deterministic_outcome(
    artefact: &crate::host::capability_host::ChangedArtefact,
) -> DeterministicRoleOutcomeInput {
    let is_high_impact = artefact.path.ends_with("/main.rs")
        || artefact.path == "main.rs"
        || artefact.name.eq_ignore_ascii_case("main");

    DeterministicRoleOutcomeInput {
        classification_known: artefact.canonical_kind.is_some(),
        best_confidence: artefact
            .canonical_kind
            .as_ref()
            .map(|_| if is_high_impact { 0.72 } else { 0.95 }),
        has_conflict: false,
        high_impact: is_high_impact,
        novel_pattern: false,
        manual_review_requested: false,
    }
}

pub fn run_adjudication_request(
    request: &RoleAdjudicationRequest,
    inference: &dyn InferenceGateway,
    repo_root: &std::path::Path,
    services: &RoleAdjudicationServices<'_>,
) -> Result<RoleAssignmentWriteOutcome> {
    let dedupe_key = request.scope_key();
    let queue_status = services.queue.claim(&dedupe_key)?;
    if queue_status.is_none() {
        let _ = services.queue.enqueue(request, &dedupe_key)?;
        let _ = services.queue.claim(&dedupe_key)?;
    }

    let packet_builder = RoleEvidencePacketBuilder {
        taxonomy: services.taxonomy,
        facts: services.facts,
        limits: EvidencePacketLimits::default(),
    };

    let packet = packet_builder.build(request)?;
    let active_role_ids = packet
        .candidate_roles
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    if active_role_ids.is_empty() {
        let failure = RoleAdjudicationFailure {
            message: "no active taxonomy roles available for adjudication".to_string(),
            retryable: false,
        };
        services.queue.fail(&dedupe_key, &failure)?;
        return apply_adjudication_result(
            services.writer,
            request,
            Err(failure),
            "unavailable",
            ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
            &json!(packet),
        );
    }

    let (raw_result, model_descriptor) =
        match execute_llm_adjudication(inference, repo_root, &packet) {
            Ok((raw_result, model_descriptor)) => (raw_result, model_descriptor),
            Err(err) => {
                let failure = RoleAdjudicationFailure {
                    message: format!("llm adjudication failed: {err:#}"),
                    retryable: true,
                };
                services.queue.fail(&dedupe_key, &failure)?;
                return apply_adjudication_result(
                    services.writer,
                    request,
                    Err(failure),
                    "unknown",
                    ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
                    &json!(packet),
                );
            }
        };

    let validated = match validate_adjudication_result(raw_result.clone(), &active_role_ids) {
        Ok(result) => result,
        Err(err) => {
            let failure = RoleAdjudicationFailure {
                message: err.to_string(),
                retryable: false,
            };
            services.queue.fail(&dedupe_key, &failure)?;
            return apply_adjudication_result(
                services.writer,
                request,
                Err(failure),
                &model_descriptor,
                ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
                &json!(packet),
            );
        }
    };

    let outcome = apply_adjudication_result(
        services.writer,
        request,
        Ok(validated.clone()),
        &model_descriptor,
        ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
        &json!(packet),
    )?;
    services.queue.complete(
        &dedupe_key,
        &validated,
        &super::contracts::RoleAdjudicationProvenance {
            source: "llm".to_string(),
            model_descriptor,
            slot_name: ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT.to_string(),
            packet_sha256: "deferred".to_string(),
            adjudication_reason: request.reason,
            adjudicated_at_unix: 0,
        },
    )?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;
    use serde_json::json;

    use super::*;
    use crate::capability_packs::architecture_graph::roles::contracts::{
        AdjudicationOutcome, AdjudicationReason, RoleAdjudicationResult,
    };
    use crate::capability_packs::architecture_graph::roles::queue_store::{
        InMemoryRoleAdjudicationQueueStore, InMemoryRoleAssignmentWriter,
    };
    use crate::host::inference::{
        EmptyInferenceGateway, InferenceGateway, StructuredGenerationRequest,
        StructuredGenerationService,
    };

    struct FakeStructuredService {
        response: serde_json::Value,
    }

    impl StructuredGenerationService for FakeStructuredService {
        fn descriptor(&self) -> String {
            "fake:model".to_string()
        }

        fn generate(&self, _request: StructuredGenerationRequest) -> Result<serde_json::Value> {
            Ok(self.response.clone())
        }
    }

    struct FakeInferenceGateway {
        response: serde_json::Value,
    }

    impl InferenceGateway for FakeInferenceGateway {
        fn embeddings(
            &self,
            slot_name: &str,
        ) -> Result<Arc<dyn crate::host::inference::EmbeddingService>> {
            anyhow::bail!("no embeddings for slot `{slot_name}`")
        }

        fn text_generation(
            &self,
            slot_name: &str,
        ) -> Result<Arc<dyn crate::host::inference::TextGenerationService>> {
            anyhow::bail!("no text generation for slot `{slot_name}`")
        }

        fn structured_generation(
            &self,
            _slot_name: &str,
        ) -> Result<Arc<dyn StructuredGenerationService>> {
            Ok(Arc::new(FakeStructuredService {
                response: self.response.clone(),
            }))
        }

        fn has_slot(&self, _slot_name: &str) -> bool {
            true
        }
    }

    #[test]
    fn high_confidence_skip_does_not_select_delta_jobs() {
        let artefacts = vec![crate::host::capability_host::ChangedArtefact {
            artefact_id: "a1".to_string(),
            symbol_id: "s1".to_string(),
            path: "src/lib.rs".to_string(),
            canonical_kind: Some("function".to_string()),
            name: "helper".to_string(),
        }];

        let requests = role_requests_from_delta("repo", 10, &artefacts);
        assert!(requests.is_empty());
    }

    #[test]
    fn ambiguous_delta_enqueues_request() {
        let artefacts = vec![crate::host::capability_host::ChangedArtefact {
            artefact_id: "a1".to_string(),
            symbol_id: "s1".to_string(),
            path: "src/lib.rs".to_string(),
            canonical_kind: None,
            name: "helper".to_string(),
        }];
        let requests = role_requests_from_delta("repo", 10, &artefacts);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].reason, AdjudicationReason::Unknown);
    }

    #[test]
    fn invalid_response_marks_needs_review_without_assignment_apply() {
        let queue = InMemoryRoleAdjudicationQueueStore::new();
        let writer = InMemoryRoleAssignmentWriter::default();
        let taxonomy = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleTaxonomyReader::with_roles(&["entrypoint"]);
        let facts = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleFactsReader::default();

        let request = RoleAdjudicationRequest {
            repo_id: "repo".to_string(),
            generation: 7,
            artefact_id: Some("a1".to_string()),
            symbol_id: Some("s1".to_string()),
            path: Some("src/main.rs".to_string()),
            language: Some("rust".to_string()),
            canonical_kind: Some("function".to_string()),
            reason: AdjudicationReason::LowConfidence,
            deterministic_confidence: Some(0.5),
            candidate_role_ids: vec!["entrypoint".to_string()],
            current_assignment: None,
        };
        queue
            .enqueue(&request, &request.scope_key())
            .expect("enqueue should succeed");

        let services = RoleAdjudicationServices {
            queue: &queue,
            taxonomy: &taxonomy,
            facts: &facts,
            writer: &writer,
        };

        let inference = FakeInferenceGateway {
            response: json!({
                "outcome": "assigned",
                "assignments": [{
                    "role_id": "unknown_role",
                    "confidence": 0.9,
                    "primary": true,
                    "evidence": []
                }],
                "confidence": 0.9,
                "evidence": [],
                "reasoning_summary": "looks like entrypoint",
                "rule_suggestions": []
            }),
        };

        run_adjudication_request(&request, &inference, std::path::Path::new("."), &services)
            .expect("adjudication write path should succeed");

        assert!(writer.applied_events().is_empty());
        assert_eq!(writer.needs_review_events().len(), 1);
    }

    #[test]
    fn valid_response_is_persisted_with_llm_source() {
        let queue = InMemoryRoleAdjudicationQueueStore::new();
        let writer = InMemoryRoleAssignmentWriter::default();
        let taxonomy = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleTaxonomyReader::with_roles(&["entrypoint"]);
        let facts = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleFactsReader::default();

        let request = RoleAdjudicationRequest {
            repo_id: "repo".to_string(),
            generation: 7,
            artefact_id: Some("a1".to_string()),
            symbol_id: Some("s1".to_string()),
            path: Some("src/main.rs".to_string()),
            language: Some("rust".to_string()),
            canonical_kind: Some("function".to_string()),
            reason: AdjudicationReason::LowConfidence,
            deterministic_confidence: Some(0.5),
            candidate_role_ids: vec!["entrypoint".to_string()],
            current_assignment: None,
        };
        queue
            .enqueue(&request, &request.scope_key())
            .expect("enqueue should succeed");

        let services = RoleAdjudicationServices {
            queue: &queue,
            taxonomy: &taxonomy,
            facts: &facts,
            writer: &writer,
        };

        let inference = FakeInferenceGateway {
            response: json!({
                "outcome": "assigned",
                "assignments": [{
                    "role_id": "entrypoint",
                    "confidence": 0.9,
                    "primary": true,
                    "evidence": ["main.rs"]
                }],
                "confidence": 0.92,
                "evidence": ["signal"],
                "reasoning_summary": "clear entrypoint semantics",
                "rule_suggestions": []
            }),
        };

        run_adjudication_request(&request, &inference, std::path::Path::new("."), &services)
            .expect("adjudication should succeed");

        assert_eq!(writer.applied_events().len(), 1);
        assert_eq!(writer.applied_events()[0].provenance.source, "llm");
    }

    #[test]
    fn fallback_inference_marks_retryable_failure() {
        let queue = InMemoryRoleAdjudicationQueueStore::new();
        let writer = InMemoryRoleAssignmentWriter::default();
        let taxonomy = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleTaxonomyReader::with_roles(&["entrypoint"]);
        let facts = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleFactsReader::default();

        let request = RoleAdjudicationRequest {
            repo_id: "repo".to_string(),
            generation: 7,
            artefact_id: Some("a1".to_string()),
            symbol_id: Some("s1".to_string()),
            path: Some("src/main.rs".to_string()),
            language: Some("rust".to_string()),
            canonical_kind: Some("function".to_string()),
            reason: AdjudicationReason::LowConfidence,
            deterministic_confidence: Some(0.5),
            candidate_role_ids: vec!["entrypoint".to_string()],
            current_assignment: None,
        };
        queue
            .enqueue(&request, &request.scope_key())
            .expect("enqueue should succeed");

        let services = RoleAdjudicationServices {
            queue: &queue,
            taxonomy: &taxonomy,
            facts: &facts,
            writer: &writer,
        };

        run_adjudication_request(
            &request,
            &EmptyInferenceGateway,
            std::path::Path::new("."),
            &services,
        )
        .expect("failure should still be persisted as needs-review");

        assert_eq!(writer.applied_events().len(), 0);
        assert_eq!(writer.needs_review_events().len(), 1);
    }

    #[test]
    fn queue_retry_only_requeues_failed_items() {
        let queue = InMemoryRoleAdjudicationQueueStore::new();
        let request = RoleAdjudicationRequest {
            repo_id: "repo".to_string(),
            generation: 1,
            artefact_id: Some("a1".to_string()),
            symbol_id: None,
            path: Some("src/lib.rs".to_string()),
            language: None,
            canonical_kind: None,
            reason: AdjudicationReason::Unknown,
            deterministic_confidence: None,
            candidate_role_ids: vec![],
            current_assignment: None,
        };
        let key = request.scope_key();
        queue.enqueue(&request, &key).expect("enqueue");

        assert!(!queue.retry(&key).expect("retry queued should be false"));

        queue.claim(&key).expect("claim");
        queue
            .fail(
                &key,
                &RoleAdjudicationFailure {
                    message: "temporary".to_string(),
                    retryable: true,
                },
            )
            .expect("fail");

        assert!(queue.retry(&key).expect("retry failed should be true"));
    }

    #[test]
    fn result_shape_can_be_constructed_for_regression_guard() {
        let _ = RoleAdjudicationResult {
            outcome: AdjudicationOutcome::Unknown,
            assignments: Vec::new(),
            confidence: 0.2,
            evidence: json!([]),
            reasoning_summary: "none".to_string(),
            rule_suggestions: Vec::new(),
        };
    }
}
