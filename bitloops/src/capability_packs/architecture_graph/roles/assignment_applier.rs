use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::contracts::{
    AdjudicationReason, RoleAdjudicationFailure, RoleAdjudicationProvenance,
    RoleAdjudicationRequest, RoleAdjudicationResult, RoleAssignmentWriteEvent,
    RoleAssignmentWriteOutcome, RoleAssignmentWriter,
};

pub async fn apply_adjudication_result(
    writer: &dyn RoleAssignmentWriter,
    request: &RoleAdjudicationRequest,
    result: Result<RoleAdjudicationResult, RoleAdjudicationFailure>,
    model_descriptor: &str,
    slot_name: &str,
    packet_json: &Value,
) -> Result<RoleAssignmentWriteOutcome> {
    let provenance = build_provenance(request.reason, model_descriptor, slot_name, packet_json);

    match result {
        Ok(validated) => {
            writer
                .apply_llm_assignment(RoleAssignmentWriteEvent {
                    request: request.clone(),
                    result: validated,
                    provenance,
                })
                .await
        }
        Err(failure) => {
            writer
                .mark_needs_review(request, &failure, &provenance)
                .await
        }
    }
}

fn build_provenance(
    reason: AdjudicationReason,
    model_descriptor: &str,
    slot_name: &str,
    packet_json: &Value,
) -> RoleAdjudicationProvenance {
    let packet_bytes = serde_json::to_vec(packet_json).unwrap_or_else(|_| b"{}".to_vec());
    let packet_sha256 = hex::encode(Sha256::digest(packet_bytes));
    let adjudicated_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    RoleAdjudicationProvenance {
        source: "llm".to_string(),
        model_descriptor: model_descriptor.to_string(),
        slot_name: slot_name.to_string(),
        packet_sha256,
        adjudication_reason: reason,
        adjudicated_at_unix,
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use serde_json::json;

    use super::*;
    use crate::capability_packs::architecture_graph::roles::contracts::AdjudicationOutcome;
    use crate::capability_packs::architecture_graph::roles::queue_store::InMemoryRoleAssignmentWriter;

    fn request() -> RoleAdjudicationRequest {
        RoleAdjudicationRequest {
            repo_id: "repo".to_string(),
            generation: 5,
            target_kind: Some("artefact".to_string()),
            artefact_id: Some("a1".to_string()),
            symbol_id: Some("s1".to_string()),
            path: Some("src/main.rs".to_string()),
            language: Some("rust".to_string()),
            canonical_kind: Some("function".to_string()),
            reason: AdjudicationReason::LowConfidence,
            deterministic_confidence: Some(0.44),
            candidate_role_ids: vec!["entrypoint".to_string()],
            current_assignment: None,
        }
    }

    #[tokio::test]
    async fn valid_result_is_applied_with_llm_provenance() {
        let writer = InMemoryRoleAssignmentWriter::default();
        let outcome = apply_adjudication_result(
            &writer,
            &request(),
            Ok(RoleAdjudicationResult {
                outcome: AdjudicationOutcome::Assigned,
                assignments: vec![crate::capability_packs::architecture_graph::roles::contracts::RoleAssignmentDecision {
                    role_id: "entrypoint".to_string(),
                    primary: true,
                    confidence: 0.91,
                    evidence: json!(["main.rs"]),
                }],
                confidence: 0.93,
                evidence: json!(["signal"]),
                reasoning_summary: "clear entrypoint semantics".to_string(),
                rule_suggestions: vec![],
            }),
            "openai:gpt-test",
            "role_adjudication",
            &json!({"packet": true}),
        )
        .await
        .expect("apply should succeed");

        assert!(outcome.persisted);
        let events = writer.applied_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].provenance.source, "llm");
        assert_eq!(events[0].provenance.model_descriptor, "openai:gpt-test");
    }

    #[tokio::test]
    async fn invalid_result_marks_needs_review_and_does_not_apply_assignment() {
        let writer = InMemoryRoleAssignmentWriter::default();

        apply_adjudication_result(
            &writer,
            &request(),
            Err(RoleAdjudicationFailure {
                message: anyhow!("schema mismatch").to_string(),
                retryable: false,
            }),
            "openai:gpt-test",
            "role_adjudication",
            &json!({"packet": true}),
        )
        .await
        .expect("needs-review write should succeed");

        assert!(writer.applied_events().is_empty());
        assert_eq!(writer.needs_review_events().len(), 1);
    }
}
