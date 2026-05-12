use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use super::*;
use crate::capability_packs::architecture_graph::roles::contracts::{
    AdjudicationOutcome, AdjudicationReason, RoleAdjudicationAttemptOutcome, RoleAdjudicationResult,
};
use crate::capability_packs::architecture_graph::roles::queue_store::{
    InMemoryRoleAdjudicationAttemptWriter, InMemoryRoleAdjudicationQueueStore,
    InMemoryRoleAssignmentWriter,
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

#[tokio::test]
async fn invalid_response_marks_needs_review_without_assignment_apply() {
    let queue = InMemoryRoleAdjudicationQueueStore::new();
    let writer = InMemoryRoleAssignmentWriter::default();
    let attempts = InMemoryRoleAdjudicationAttemptWriter::default();
    let taxonomy = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleTaxonomyReader::with_roles(&["entrypoint"]);
    let facts = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleFactsReader::default();

    let request = RoleAdjudicationRequest {
        repo_id: "repo".to_string(),
        generation: 7,
        target_kind: Some("artefact".to_string()),
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
        attempts: &attempts,
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
        .await
        .expect("adjudication write path should succeed");

    assert!(writer.applied_events().is_empty());
    assert_eq!(writer.needs_review_events().len(), 1);
}

#[tokio::test]
async fn unknown_response_without_request_candidate_persists_attempt() {
    let queue = InMemoryRoleAdjudicationQueueStore::new();
    let writer = InMemoryRoleAssignmentWriter::default();
    let attempts = InMemoryRoleAdjudicationAttemptWriter::default();
    let taxonomy = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleTaxonomyReader::with_roles(&["entrypoint"]);
    let facts = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleFactsReader::default();
    let request = RoleAdjudicationRequest {
        repo_id: "repo".to_string(),
        generation: 7,
        target_kind: Some("artefact".to_string()),
        artefact_id: Some("a1".to_string()),
        symbol_id: Some("s1".to_string()),
        path: Some("src/application/create_user.rs".to_string()),
        language: Some("rust".to_string()),
        canonical_kind: Some("function".to_string()),
        reason: AdjudicationReason::Unknown,
        deterministic_confidence: None,
        candidate_role_ids: vec![],
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
        attempts: &attempts,
    };
    let inference = FakeInferenceGateway {
        response: json!({
            "outcome": "unknown",
            "assignments": [],
            "confidence": 0.31,
            "evidence": {},
            "reasoning_summary": "The evidence does not prove a role.",
            "rule_suggestions": []
        }),
    };

    let outcome =
        run_adjudication_request(&request, &inference, std::path::Path::new("."), &services)
            .await
            .expect("adjudication should complete");

    assert!(!outcome.persisted);
    assert_eq!(writer.applied_events().len(), 0);
    assert_eq!(attempts.recorded_attempts().len(), 1);
    assert_eq!(
        attempts.recorded_attempts()[0].outcome,
        RoleAdjudicationAttemptOutcome::Unknown
    );
    assert!(attempts.recorded_attempts()[0].raw_response_json.is_some());
    assert_eq!(attempts.write_results().len(), 1);
    assert!(!attempts.write_results()[0].assignment_write_persisted);
}

#[tokio::test]
async fn valid_unknown_response_with_request_candidate_skips_assignment_writer() {
    let queue = InMemoryRoleAdjudicationQueueStore::new();
    let writer = InMemoryRoleAssignmentWriter::default();
    let attempts = InMemoryRoleAdjudicationAttemptWriter::default();
    let taxonomy = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleTaxonomyReader::with_roles(&["entrypoint"]);
    let facts = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleFactsReader::default();
    let request = RoleAdjudicationRequest {
        repo_id: "repo".to_string(),
        generation: 7,
        target_kind: Some("artefact".to_string()),
        artefact_id: Some("a1".to_string()),
        symbol_id: Some("s1".to_string()),
        path: Some("src/application/create_user.rs".to_string()),
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
        attempts: &attempts,
    };
    let inference = FakeInferenceGateway {
        response: json!({
            "outcome": "unknown",
            "assignments": [],
            "confidence": 0.31,
            "evidence": {},
            "reasoning_summary": "The evidence does not prove a role.",
            "rule_suggestions": []
        }),
    };

    let outcome =
        run_adjudication_request(&request, &inference, std::path::Path::new("."), &services)
            .await
            .expect("adjudication should complete");

    assert!(!outcome.persisted);
    assert_eq!(outcome.source, "skipped_non_assignment");
    assert_eq!(writer.applied_events().len(), 0);
    assert_eq!(writer.needs_review_events().len(), 0);
    assert_eq!(attempts.recorded_attempts().len(), 1);
    assert_eq!(
        attempts.recorded_attempts()[0].outcome,
        RoleAdjudicationAttemptOutcome::Unknown
    );
    assert!(attempts.recorded_attempts()[0].raw_response_json.is_some());
    assert!(
        attempts.recorded_attempts()[0]
            .validated_result_json
            .is_some()
    );
    assert_eq!(attempts.write_results().len(), 1);
    assert!(!attempts.write_results()[0].assignment_write_persisted);
    assert_eq!(
        attempts.write_results()[0].assignment_write_source,
        "skipped_non_assignment"
    );
}

#[tokio::test]
async fn valid_needs_review_response_with_request_candidate_skips_assignment_writer() {
    let queue = InMemoryRoleAdjudicationQueueStore::new();
    let writer = InMemoryRoleAssignmentWriter::default();
    let attempts = InMemoryRoleAdjudicationAttemptWriter::default();
    let taxonomy = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleTaxonomyReader::with_roles(&["entrypoint"]);
    let facts = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleFactsReader::default();
    let request = RoleAdjudicationRequest {
        repo_id: "repo".to_string(),
        generation: 7,
        target_kind: Some("artefact".to_string()),
        artefact_id: Some("a1".to_string()),
        symbol_id: Some("s1".to_string()),
        path: Some("src/application/create_user.rs".to_string()),
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
        attempts: &attempts,
    };
    let inference = FakeInferenceGateway {
        response: json!({
            "outcome": "needs_review",
            "assignments": [],
            "confidence": 0.44,
            "evidence": {},
            "reasoning_summary": "The role remains ambiguous.",
            "rule_suggestions": []
        }),
    };

    let outcome =
        run_adjudication_request(&request, &inference, std::path::Path::new("."), &services)
            .await
            .expect("adjudication should complete");

    assert!(!outcome.persisted);
    assert_eq!(outcome.source, "skipped_non_assignment");
    assert_eq!(writer.applied_events().len(), 0);
    assert_eq!(writer.needs_review_events().len(), 0);
    assert_eq!(attempts.recorded_attempts().len(), 1);
    assert_eq!(
        attempts.recorded_attempts()[0].outcome,
        RoleAdjudicationAttemptOutcome::NeedsReview
    );
    assert!(attempts.recorded_attempts()[0].raw_response_json.is_some());
    assert!(
        attempts.recorded_attempts()[0]
            .validated_result_json
            .is_some()
    );
    assert_eq!(attempts.write_results().len(), 1);
    assert!(!attempts.write_results()[0].assignment_write_persisted);
    assert_eq!(
        attempts.write_results()[0].assignment_write_source,
        "skipped_non_assignment"
    );
}

#[tokio::test]
async fn invalid_response_persists_raw_attempt_before_review_mark() {
    let queue = InMemoryRoleAdjudicationQueueStore::new();
    let writer = InMemoryRoleAssignmentWriter::default();
    let attempts = InMemoryRoleAdjudicationAttemptWriter::default();
    let taxonomy = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleTaxonomyReader::with_roles(&["entrypoint"]);
    let facts = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleFactsReader::default();
    let request = RoleAdjudicationRequest {
        repo_id: "repo".to_string(),
        generation: 7,
        target_kind: Some("artefact".to_string()),
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
        attempts: &attempts,
    };
    let inference = FakeInferenceGateway {
        response: json!({
            "outcome": "assigned",
            "assignments": [{
                "role_id": "missing-role",
                "confidence": 0.8,
                "primary": true,
                "evidence": {}
            }],
            "confidence": 0.8,
            "evidence": {},
            "reasoning_summary": "Invalid role id.",
            "rule_suggestions": []
        }),
    };

    run_adjudication_request(&request, &inference, std::path::Path::new("."), &services)
        .await
        .expect("validation failure should be handled");

    assert_eq!(attempts.recorded_attempts().len(), 1);
    assert_eq!(
        attempts.recorded_attempts()[0].outcome,
        RoleAdjudicationAttemptOutcome::ValidationError
    );
    assert!(
        attempts.recorded_attempts()[0]
            .failure_message
            .as_deref()
            .unwrap_or("")
            .contains("unknown role id")
    );
    assert!(attempts.recorded_attempts()[0].raw_response_json.is_some());
}

#[tokio::test]
async fn valid_response_is_persisted_with_llm_source() {
    let queue = InMemoryRoleAdjudicationQueueStore::new();
    let writer = InMemoryRoleAssignmentWriter::default();
    let attempts = InMemoryRoleAdjudicationAttemptWriter::default();
    let taxonomy = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleTaxonomyReader::with_roles(&["entrypoint"]);
    let facts = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleFactsReader::default();

    let request = RoleAdjudicationRequest {
        repo_id: "repo".to_string(),
        generation: 7,
        target_kind: Some("artefact".to_string()),
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
        attempts: &attempts,
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
        .await
        .expect("adjudication should succeed");

    assert_eq!(writer.applied_events().len(), 1);
    assert_eq!(writer.applied_events()[0].provenance.source, "llm");
}

#[tokio::test]
async fn fallback_inference_marks_retryable_failure() {
    let queue = InMemoryRoleAdjudicationQueueStore::new();
    let writer = InMemoryRoleAssignmentWriter::default();
    let attempts = InMemoryRoleAdjudicationAttemptWriter::default();
    let taxonomy = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleTaxonomyReader::with_roles(&["entrypoint"]);
    let facts = crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleFactsReader::default();

    let request = RoleAdjudicationRequest {
        repo_id: "repo".to_string(),
        generation: 7,
        target_kind: Some("artefact".to_string()),
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
        attempts: &attempts,
    };

    run_adjudication_request(
        &request,
        &EmptyInferenceGateway,
        std::path::Path::new("."),
        &services,
    )
    .await
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
        target_kind: Some("artefact".to_string()),
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
