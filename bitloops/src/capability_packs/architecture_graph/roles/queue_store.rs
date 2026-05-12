use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::contracts::{
    AdjudicationOutcome, RoleAdjudicationAttemptEvent, RoleAdjudicationAttemptWriteResult,
    RoleAdjudicationAttemptWriter, RoleAdjudicationFailure, RoleAdjudicationProvenance,
    RoleAdjudicationRequest, RoleAdjudicationResult, RoleAssignmentWriteEvent,
    RoleAssignmentWriteOutcome, RoleAssignmentWriter, RoleBoxFuture, RoleCandidateDescriptor,
    RoleFactsBundle, RoleFactsReader, RoleQueueEnqueueResult, RoleQueueJobStatus,
    RoleTaxonomyReader,
};
use anyhow::Result;

pub use super::contracts::RoleAdjudicationQueueStore;

#[derive(Debug, Clone)]
struct QueueEntry {
    status: RoleQueueJobStatus,
    attempts: u32,
    last_error: Option<RoleAdjudicationFailure>,
}

#[derive(Default)]
pub struct InMemoryRoleAdjudicationQueueStore {
    entries: Mutex<HashMap<String, QueueEntry>>,
}

impl InMemoryRoleAdjudicationQueueStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attempts_for(&self, dedupe_key: &str) -> Option<u32> {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(dedupe_key).map(|entry| entry.attempts))
    }
}

impl RoleAdjudicationQueueStore for InMemoryRoleAdjudicationQueueStore {
    fn enqueue(
        &self,
        _request: &RoleAdjudicationRequest,
        dedupe_key: &str,
    ) -> Result<RoleQueueEnqueueResult> {
        let mut entries = self.entries.lock().expect("queue mutex poisoned");
        let Some(existing) = entries.get(dedupe_key) else {
            entries.insert(
                dedupe_key.to_string(),
                QueueEntry {
                    status: RoleQueueJobStatus::Queued,
                    attempts: 0,
                    last_error: None,
                },
            );
            return Ok(RoleQueueEnqueueResult::Enqueued);
        };

        Ok(match existing.status {
            RoleQueueJobStatus::Completed => RoleQueueEnqueueResult::AlreadyCompleted,
            RoleQueueJobStatus::Queued
            | RoleQueueJobStatus::Running
            | RoleQueueJobStatus::Failed => RoleQueueEnqueueResult::AlreadyQueued,
        })
    }

    fn claim(&self, dedupe_key: &str) -> Result<Option<RoleQueueJobStatus>> {
        let mut entries = self.entries.lock().expect("queue mutex poisoned");
        let Some(entry) = entries.get_mut(dedupe_key) else {
            return Ok(None);
        };
        entry.status = RoleQueueJobStatus::Running;
        entry.attempts = entry.attempts.saturating_add(1);
        Ok(Some(entry.status))
    }

    fn complete(
        &self,
        dedupe_key: &str,
        _result: &RoleAdjudicationResult,
        _provenance: &RoleAdjudicationProvenance,
    ) -> Result<()> {
        let mut entries = self.entries.lock().expect("queue mutex poisoned");
        if let Some(entry) = entries.get_mut(dedupe_key) {
            entry.status = RoleQueueJobStatus::Completed;
            entry.last_error = None;
        }
        Ok(())
    }

    fn fail(&self, dedupe_key: &str, failure: &RoleAdjudicationFailure) -> Result<()> {
        let mut entries = self.entries.lock().expect("queue mutex poisoned");
        if let Some(entry) = entries.get_mut(dedupe_key) {
            entry.status = RoleQueueJobStatus::Failed;
            entry.last_error = Some(failure.clone());
        }
        Ok(())
    }

    fn retry(&self, dedupe_key: &str) -> Result<bool> {
        let mut entries = self.entries.lock().expect("queue mutex poisoned");
        let Some(entry) = entries.get_mut(dedupe_key) else {
            return Ok(false);
        };
        if !matches!(entry.status, RoleQueueJobStatus::Failed) {
            return Ok(false);
        }
        entry.status = RoleQueueJobStatus::Queued;
        entry.last_error = None;
        Ok(true)
    }
}

static DEFAULT_QUEUE: OnceLock<Arc<dyn RoleAdjudicationQueueStore>> = OnceLock::new();

pub fn default_queue_store() -> Arc<dyn RoleAdjudicationQueueStore> {
    DEFAULT_QUEUE
        .get_or_init(|| Arc::new(InMemoryRoleAdjudicationQueueStore::new()))
        .clone()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InMemoryAttemptWriteResult {
    pub repo_id: String,
    pub attempt_id: String,
    pub assignment_write_persisted: bool,
    pub assignment_write_source: String,
}

#[derive(Default)]
pub struct InMemoryRoleAdjudicationAttemptWriter {
    attempts: Mutex<Vec<RoleAdjudicationAttemptEvent>>,
    write_results: Mutex<Vec<InMemoryAttemptWriteResult>>,
}

impl InMemoryRoleAdjudicationAttemptWriter {
    pub fn recorded_attempts(&self) -> Vec<RoleAdjudicationAttemptEvent> {
        self.attempts
            .lock()
            .expect("attempt mutex poisoned")
            .clone()
    }

    pub fn write_results(&self) -> Vec<InMemoryAttemptWriteResult> {
        self.write_results
            .lock()
            .expect("attempt write result mutex poisoned")
            .clone()
    }
}

impl RoleAdjudicationAttemptWriter for InMemoryRoleAdjudicationAttemptWriter {
    fn record_attempt<'a>(
        &'a self,
        event: RoleAdjudicationAttemptEvent,
    ) -> RoleBoxFuture<'a, RoleAdjudicationAttemptWriteResult> {
        Box::pin(async move {
            let attempt_id = event.attempt_id.clone();
            self.attempts
                .lock()
                .expect("attempt mutex poisoned")
                .push(event);
            Ok(RoleAdjudicationAttemptWriteResult { attempt_id })
        })
    }

    fn mark_assignment_write_result<'a>(
        &'a self,
        repo_id: &'a str,
        attempt_id: &'a str,
        assignment_write_persisted: bool,
        assignment_write_source: &'a str,
    ) -> RoleBoxFuture<'a, ()> {
        Box::pin(async move {
            self.write_results
                .lock()
                .expect("attempt write result mutex poisoned")
                .push(InMemoryAttemptWriteResult {
                    repo_id: repo_id.to_string(),
                    attempt_id: attempt_id.to_string(),
                    assignment_write_persisted,
                    assignment_write_source: assignment_write_source.to_string(),
                });
            Ok(())
        })
    }
}

#[derive(Default)]
pub struct InMemoryRoleAssignmentWriter {
    applied: Mutex<Vec<RoleAssignmentWriteEvent>>,
    review: Mutex<
        Vec<(
            RoleAdjudicationRequest,
            RoleAdjudicationFailure,
            RoleAdjudicationProvenance,
        )>,
    >,
}

impl InMemoryRoleAssignmentWriter {
    pub fn applied_events(&self) -> Vec<RoleAssignmentWriteEvent> {
        self.applied.lock().expect("writer mutex poisoned").clone()
    }

    pub fn needs_review_events(
        &self,
    ) -> Vec<(
        RoleAdjudicationRequest,
        RoleAdjudicationFailure,
        RoleAdjudicationProvenance,
    )> {
        self.review.lock().expect("writer mutex poisoned").clone()
    }
}

impl RoleAssignmentWriter for InMemoryRoleAssignmentWriter {
    fn apply_llm_assignment<'a>(
        &'a self,
        event: RoleAssignmentWriteEvent,
    ) -> RoleBoxFuture<'a, RoleAssignmentWriteOutcome> {
        Box::pin(async move {
            if event.result.outcome != AdjudicationOutcome::Assigned
                || event.result.assignments.is_empty()
            {
                return Ok(RoleAssignmentWriteOutcome {
                    source: "in_memory",
                    persisted: false,
                });
            }
            self.applied
                .lock()
                .expect("writer mutex poisoned")
                .push(event);
            Ok(RoleAssignmentWriteOutcome {
                source: "in_memory",
                persisted: true,
            })
        })
    }

    fn mark_needs_review<'a>(
        &'a self,
        request: &'a RoleAdjudicationRequest,
        failure: &'a RoleAdjudicationFailure,
        provenance: &'a RoleAdjudicationProvenance,
    ) -> RoleBoxFuture<'a, RoleAssignmentWriteOutcome> {
        Box::pin(async move {
            self.review.lock().expect("writer mutex poisoned").push((
                request.clone(),
                failure.clone(),
                provenance.clone(),
            ));
            Ok(RoleAssignmentWriteOutcome {
                source: "in_memory",
                persisted: true,
            })
        })
    }
}

#[derive(Default)]
pub struct InMemoryRoleTaxonomyReader {
    active_roles: Mutex<Vec<RoleCandidateDescriptor>>,
}

impl InMemoryRoleTaxonomyReader {
    pub fn with_roles(roles: &[&str]) -> Self {
        let active_roles = roles
            .iter()
            .map(|role| RoleCandidateDescriptor {
                role_id: (*role).to_string(),
                canonical_key: (*role).to_string(),
                family: "test".to_string(),
                display_name: (*role).to_string(),
                description: String::new(),
            })
            .collect();
        Self {
            active_roles: Mutex::new(active_roles),
        }
    }

    pub fn with_role_descriptors(active_roles: Vec<RoleCandidateDescriptor>) -> Self {
        Self {
            active_roles: Mutex::new(active_roles),
        }
    }
}

impl RoleTaxonomyReader for InMemoryRoleTaxonomyReader {
    fn load_active_roles<'a>(
        &'a self,
        _repo_id: &'a str,
        _generation: u64,
    ) -> RoleBoxFuture<'a, Vec<RoleCandidateDescriptor>> {
        Box::pin(async move {
            Ok(self
                .active_roles
                .lock()
                .expect("taxonomy mutex poisoned")
                .clone())
        })
    }
}

pub struct InMemoryRoleFactsReader {
    bundle: Mutex<RoleFactsBundle>,
}

impl InMemoryRoleFactsReader {
    pub fn with_bundle(bundle: RoleFactsBundle) -> Self {
        Self {
            bundle: Mutex::new(bundle),
        }
    }
}

impl Default for InMemoryRoleFactsReader {
    fn default() -> Self {
        Self {
            bundle: Mutex::new(RoleFactsBundle {
                facts: Vec::new(),
                rule_signals: Vec::new(),
                dependency_context: Vec::new(),
                related_artefacts: Vec::new(),
                source_snippets: Vec::new(),
                reachability: None,
            }),
        }
    }
}

impl RoleFactsReader for InMemoryRoleFactsReader {
    fn load_facts<'a>(
        &'a self,
        _request: &'a RoleAdjudicationRequest,
    ) -> RoleBoxFuture<'a, RoleFactsBundle> {
        Box::pin(async move { Ok(self.bundle.lock().expect("facts mutex poisoned").clone()) })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::capability_packs::architecture_graph::roles::contracts::{
        AdjudicationOutcome, AdjudicationReason,
    };

    fn request() -> RoleAdjudicationRequest {
        RoleAdjudicationRequest {
            repo_id: "repo".to_string(),
            generation: 1,
            target_kind: Some("artefact".to_string()),
            artefact_id: Some("artefact-1".to_string()),
            symbol_id: None,
            path: Some("src/main.rs".to_string()),
            language: Some("rust".to_string()),
            canonical_kind: Some("function".to_string()),
            reason: AdjudicationReason::Unknown,
            deterministic_confidence: None,
            candidate_role_ids: vec![],
            current_assignment: None,
        }
    }

    #[test]
    fn queue_store_dedupes_and_allows_retry_after_failure() {
        let store = InMemoryRoleAdjudicationQueueStore::new();
        let request = request();
        let key = request.scope_key();

        assert_eq!(
            store.enqueue(&request, &key).expect("enqueue should work"),
            RoleQueueEnqueueResult::Enqueued
        );
        assert_eq!(
            store
                .enqueue(&request, &key)
                .expect("second enqueue should dedupe"),
            RoleQueueEnqueueResult::AlreadyQueued
        );

        assert_eq!(
            store.claim(&key).expect("claim should succeed"),
            Some(RoleQueueJobStatus::Running)
        );
        assert_eq!(store.attempts_for(&key), Some(1));

        store
            .fail(
                &key,
                &RoleAdjudicationFailure {
                    message: "temporary provider error".to_string(),
                    retryable: true,
                },
            )
            .expect("fail should work");

        assert!(store.retry(&key).expect("retry should succeed"));
        assert_eq!(
            store.claim(&key).expect("reclaim should succeed"),
            Some(RoleQueueJobStatus::Running)
        );
        assert_eq!(store.attempts_for(&key), Some(2));

        store
            .complete(
                &key,
                &RoleAdjudicationResult {
                    outcome: AdjudicationOutcome::Unknown,
                    assignments: vec![],
                    confidence: 0.1,
                    evidence: json!([]),
                    reasoning_summary: "not enough evidence".to_string(),
                    rule_suggestions: vec![],
                },
                &RoleAdjudicationProvenance {
                    source: "llm".to_string(),
                    model_descriptor: "x:y".to_string(),
                    slot_name: "role_adjudication".to_string(),
                    packet_sha256: "abc".to_string(),
                    adjudication_reason: AdjudicationReason::Unknown,
                    adjudicated_at_unix: 1,
                },
            )
            .expect("complete should work");

        assert_eq!(
            store
                .enqueue(&request, &key)
                .expect("completed dedupe should be stable"),
            RoleQueueEnqueueResult::AlreadyCompleted
        );
    }
}
