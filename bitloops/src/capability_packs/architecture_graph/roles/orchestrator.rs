use anyhow::Result;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
    ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
};
use crate::host::capability_host::gateways::{CapabilityWorkplaneGateway, CapabilityWorkplaneJob};
use crate::host::inference::InferenceGateway;

use super::adjudication_selector::{DeterministicRoleOutcomeInput, select_adjudication_reason};
use super::assignment_applier::apply_adjudication_result;
use super::contracts::{
    AdjudicationOutcome, RoleAdjudicationAttemptEvent, RoleAdjudicationAttemptOutcome,
    RoleAdjudicationFailure, RoleAdjudicationMailboxPayload, RoleAdjudicationRequest,
    RoleAssignmentWriteOutcome,
};
use super::evidence_packet_builder::{
    EvidencePacketLimits, RoleEvidencePacket, RoleEvidencePacketBuilder,
};
use super::llm_executor::execute_llm_adjudication;
use super::queue_store::RoleAdjudicationQueueStore;
use super::response_validator::validate_adjudication_result;

const SKIPPED_NON_ASSIGNMENT_WRITE_SOURCE: &str = "skipped_non_assignment";

pub struct RoleAdjudicationServices<'a> {
    pub queue: &'a dyn RoleAdjudicationQueueStore,
    pub taxonomy: &'a dyn super::contracts::RoleTaxonomyReader,
    pub facts: &'a dyn super::contracts::RoleFactsReader,
    pub writer: &'a dyn super::contracts::RoleAssignmentWriter,
    pub attempts: &'a dyn super::contracts::RoleAdjudicationAttemptWriter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleAdjudicationEnqueueMetrics {
    pub selected: usize,
    pub enqueued: usize,
    pub deduped: usize,
}

pub fn enqueue_adjudication_requests(
    requests: &[RoleAdjudicationRequest],
    workplane: &dyn CapabilityWorkplaneGateway,
    queue: &dyn RoleAdjudicationQueueStore,
) -> Result<RoleAdjudicationEnqueueMetrics> {
    let mut jobs = Vec::new();
    let mut deduped = 0usize;

    for request in requests {
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

/// Compatibility helper for legacy delta callers; current-state sync selects role adjudication through the classifier.
pub fn enqueue_adjudication_jobs_for_delta(
    repo_id: &str,
    generation: u64,
    artefact_upserts: &[crate::host::capability_host::ChangedArtefact],
    workplane: &dyn CapabilityWorkplaneGateway,
    queue: &dyn RoleAdjudicationQueueStore,
) -> Result<RoleAdjudicationEnqueueMetrics> {
    let requests = role_requests_from_delta(repo_id, generation, artefact_upserts);
    enqueue_adjudication_requests(&requests, workplane, queue)
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
                target_kind: Some("artefact".to_string()),
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

fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn unix_timestamp_nanos_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

fn attempt_id_for(request: &RoleAdjudicationRequest, observed_at_unix_nanos: u128) -> String {
    crate::host::devql::deterministic_uuid(&format!(
        "architecture_role_adjudication_attempt|{}|{}|{}",
        request.repo_id,
        request.scope_key(),
        observed_at_unix_nanos
    ))
}

fn attempt_outcome_from_result(
    result: &super::contracts::RoleAdjudicationResult,
) -> RoleAdjudicationAttemptOutcome {
    match result.outcome {
        AdjudicationOutcome::Assigned => RoleAdjudicationAttemptOutcome::Assigned,
        AdjudicationOutcome::Unknown => RoleAdjudicationAttemptOutcome::Unknown,
        AdjudicationOutcome::NeedsReview => RoleAdjudicationAttemptOutcome::NeedsReview,
    }
}

#[allow(clippy::too_many_arguments)]
fn role_adjudication_attempt_event(
    request: &RoleAdjudicationRequest,
    dedupe_key: &str,
    packet: &RoleEvidencePacket,
    packet_json: &Value,
    packet_sha256: &str,
    attempt_id: &str,
    observed_at_unix: u64,
    model_descriptor: &str,
    outcome: RoleAdjudicationAttemptOutcome,
    raw_response_json: Option<Value>,
    validated_result_json: Option<Value>,
    failure_message: Option<String>,
    retryable: bool,
) -> Result<RoleAdjudicationAttemptEvent> {
    Ok(RoleAdjudicationAttemptEvent {
        attempt_id: attempt_id.to_string(),
        repo_id: request.repo_id.clone(),
        scope_key: dedupe_key.to_string(),
        generation: request.generation,
        target_kind: request.target_kind.clone(),
        artefact_id: request.artefact_id.clone(),
        symbol_id: request.symbol_id.clone(),
        path: request.path.clone(),
        reason: request.reason,
        deterministic_confidence: request.deterministic_confidence,
        candidate_roles: packet.candidate_roles.clone(),
        current_assignment: request.current_assignment.clone(),
        request_json: serde_json::to_value(request)?,
        evidence_packet_sha256: packet_sha256.to_string(),
        evidence_packet_json: packet_json.clone(),
        model_descriptor: model_descriptor.to_string(),
        slot_name: ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT.to_string(),
        outcome,
        raw_response_json,
        validated_result_json,
        failure_message,
        retryable,
        observed_at_unix,
    })
}

pub async fn run_adjudication_request(
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

    let packet = packet_builder.build(request).await?;
    let packet_json = json!(packet);
    let packet_sha256 = sha256_hex(&serde_json::to_vec(&packet_json)?);
    let observed_at_unix = unix_timestamp_now();
    let observed_at_unix_nanos = unix_timestamp_nanos_now();
    let attempt_id = attempt_id_for(request, observed_at_unix_nanos);
    let active_role_ids = packet
        .candidate_roles
        .iter()
        .map(|role| role.role_id.clone())
        .collect::<BTreeSet<_>>();

    if active_role_ids.is_empty() {
        let failure = RoleAdjudicationFailure {
            message: "no active taxonomy roles available for adjudication".to_string(),
            retryable: false,
        };
        let attempt_write = services
            .attempts
            .record_attempt(role_adjudication_attempt_event(
                request,
                &dedupe_key,
                &packet,
                &packet_json,
                &packet_sha256,
                &attempt_id,
                observed_at_unix,
                "unavailable",
                RoleAdjudicationAttemptOutcome::NoActiveRoles,
                None,
                None,
                Some(failure.message.clone()),
                failure.retryable,
            )?)
            .await?;
        services.queue.fail(&dedupe_key, &failure)?;
        let outcome = apply_adjudication_result(
            services.writer,
            request,
            Err(failure),
            "unavailable",
            ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
            &packet_json,
        )
        .await?;
        services
            .attempts
            .mark_assignment_write_result(
                &request.repo_id,
                &attempt_write.attempt_id,
                outcome.persisted,
                outcome.source,
            )
            .await?;
        return Ok(outcome);
    }

    let (raw_result, model_descriptor) =
        match execute_llm_adjudication(inference, repo_root, &packet) {
            Ok((raw_result, model_descriptor)) => (raw_result, model_descriptor),
            Err(err) => {
                let failure = RoleAdjudicationFailure {
                    message: format!("llm adjudication failed: {err:#}"),
                    retryable: true,
                };
                let attempt_write = services
                    .attempts
                    .record_attempt(role_adjudication_attempt_event(
                        request,
                        &dedupe_key,
                        &packet,
                        &packet_json,
                        &packet_sha256,
                        &attempt_id,
                        observed_at_unix,
                        "unknown",
                        RoleAdjudicationAttemptOutcome::LlmError,
                        None,
                        None,
                        Some(failure.message.clone()),
                        failure.retryable,
                    )?)
                    .await?;
                services.queue.fail(&dedupe_key, &failure)?;
                let outcome = apply_adjudication_result(
                    services.writer,
                    request,
                    Err(failure),
                    "unknown",
                    ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
                    &packet_json,
                )
                .await?;
                services
                    .attempts
                    .mark_assignment_write_result(
                        &request.repo_id,
                        &attempt_write.attempt_id,
                        outcome.persisted,
                        outcome.source,
                    )
                    .await?;
                return Ok(outcome);
            }
        };

    let validated = match validate_adjudication_result(raw_result.clone(), &active_role_ids) {
        Ok(result) => result,
        Err(err) => {
            let failure = RoleAdjudicationFailure {
                message: err.to_string(),
                retryable: false,
            };
            let attempt_write = services
                .attempts
                .record_attempt(role_adjudication_attempt_event(
                    request,
                    &dedupe_key,
                    &packet,
                    &packet_json,
                    &packet_sha256,
                    &attempt_id,
                    observed_at_unix,
                    &model_descriptor,
                    RoleAdjudicationAttemptOutcome::ValidationError,
                    Some(raw_result.clone()),
                    None,
                    Some(failure.message.clone()),
                    failure.retryable,
                )?)
                .await?;
            services.queue.fail(&dedupe_key, &failure)?;
            let outcome = apply_adjudication_result(
                services.writer,
                request,
                Err(failure),
                &model_descriptor,
                ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
                &packet_json,
            )
            .await?;
            services
                .attempts
                .mark_assignment_write_result(
                    &request.repo_id,
                    &attempt_write.attempt_id,
                    outcome.persisted,
                    outcome.source,
                )
                .await?;
            return Ok(outcome);
        }
    };

    let attempt_write = services
        .attempts
        .record_attempt(role_adjudication_attempt_event(
            request,
            &dedupe_key,
            &packet,
            &packet_json,
            &packet_sha256,
            &attempt_id,
            observed_at_unix,
            &model_descriptor,
            attempt_outcome_from_result(&validated),
            Some(raw_result.clone()),
            Some(serde_json::to_value(&validated)?),
            None,
            false,
        )?)
        .await?;

    if !matches!(validated.outcome, AdjudicationOutcome::Assigned) {
        services
            .attempts
            .mark_assignment_write_result(
                &request.repo_id,
                &attempt_write.attempt_id,
                false,
                SKIPPED_NON_ASSIGNMENT_WRITE_SOURCE,
            )
            .await?;
        services.queue.complete(
            &dedupe_key,
            &validated,
            &super::contracts::RoleAdjudicationProvenance {
                source: "llm".to_string(),
                model_descriptor,
                slot_name: ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT.to_string(),
                packet_sha256,
                adjudication_reason: request.reason,
                adjudicated_at_unix: observed_at_unix,
            },
        )?;
        return Ok(RoleAssignmentWriteOutcome {
            source: SKIPPED_NON_ASSIGNMENT_WRITE_SOURCE,
            persisted: false,
        });
    }

    let outcome = apply_adjudication_result(
        services.writer,
        request,
        Ok(validated.clone()),
        &model_descriptor,
        ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
        &packet_json,
    )
    .await?;
    services
        .attempts
        .mark_assignment_write_result(
            &request.repo_id,
            &attempt_write.attempt_id,
            outcome.persisted,
            outcome.source,
        )
        .await?;
    services.queue.complete(
        &dedupe_key,
        &validated,
        &super::contracts::RoleAdjudicationProvenance {
            source: "llm".to_string(),
            model_descriptor,
            slot_name: ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT.to_string(),
            packet_sha256,
            adjudication_reason: request.reason,
            adjudicated_at_unix: observed_at_unix,
        },
    )?;
    Ok(outcome)
}

#[cfg(test)]
mod tests;
