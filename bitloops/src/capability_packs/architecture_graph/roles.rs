pub mod adjudication_selector;
pub mod assignment_applier;
pub mod classifier;
pub mod contracts;
pub mod current_state_consumer;
pub mod evidence_packet_builder;
pub mod fact_extraction;
pub mod llm_adjudication;
pub mod llm_executor;
pub mod migrations;
pub mod orchestrator;
pub mod queue_store;
pub mod response_validator;
pub mod rules;
pub mod schema;
pub mod storage;
pub mod taxonomy;

pub use adjudication_selector::{
    DeterministicRoleOutcomeInput, HIGH_CONFIDENCE_THRESHOLD, select_adjudication_reason,
};
pub use assignment_applier::apply_adjudication_result;
pub use classifier::ARCHITECTURE_ROLE_CLASSIFIER_VERSION;
pub use contracts::{
    AdjudicationOutcome, AdjudicationReason, RoleAdjudicationFailure,
    RoleAdjudicationMailboxPayload, RoleAdjudicationProvenance, RoleAdjudicationRequest,
    RoleAdjudicationResult, RoleAdjudicationRuleSuggestion, RoleAdjudicationValidationError,
    RoleAssignmentDecision, RoleAssignmentWriteEvent, RoleAssignmentWriteOutcome,
    RoleAssignmentWriter, RoleFactsBundle, RoleFactsReader, RoleQueueEnqueueResult,
    RoleQueueJobStatus, RoleTaxonomyReader, RuleSignalFact,
};
pub use current_state_consumer::ArchitectureGraphRoleCurrentStateConsumer;
pub use evidence_packet_builder::{
    EvidencePacketLimits, RoleEvidencePacket, RoleEvidencePacketBuilder,
};
pub use llm_executor::execute_llm_adjudication;
pub use orchestrator::{
    RoleAdjudicationEnqueueMetrics, RoleAdjudicationServices, enqueue_adjudication_jobs_for_delta,
    enqueue_adjudication_requests, run_adjudication_request,
};
pub use queue_store::{
    InMemoryRoleAdjudicationAttemptWriter, InMemoryRoleAdjudicationQueueStore,
    InMemoryRoleAssignmentWriter, InMemoryRoleFactsReader, InMemoryRoleTaxonomyReader,
    RoleAdjudicationQueueStore, default_queue_store,
};
pub use response_validator::validate_adjudication_result;
pub use storage::{
    DbRoleAdjudicationAttemptWriter, DbRoleAssignmentWriter, DbRoleFactsReader,
    DbRoleTaxonomyReader,
};
pub use taxonomy::{
    ArchitectureArtefactFact, ArchitectureRole, ArchitectureRoleAssignment,
    ArchitectureRoleAssignmentHistory, ArchitectureRoleChangeProposal,
    ArchitectureRoleDetectionRule, ArchitectureRoleReconcileMetrics,
    ArchitectureRoleReconcileOutcome, ArchitectureRoleRuleSignal, AssignmentPriority,
    AssignmentSource, AssignmentStatus, ProposalStatus, RoleCandidateSelector, RoleFactCondition,
    RoleFactConditionOp, RoleLifecycle, RoleRuleCandidateSelector, RoleRuleCondition,
    RoleRuleLifecycle, RoleRuleScore, RoleSignalPolarity, RoleSplitSpecFile, RoleSplitTargetRole,
    RoleTarget, RuleSpecFile, SeededArchitectureRole, SeededArchitectureRuleCandidate,
    SeededArchitectureTaxonomy, TargetKind,
};

#[cfg(test)]
mod tests {
    #[test]
    fn roles_facade_exposes_expected_submodules() {
        let submodules = [
            "adjudication_selector",
            "assignment_applier",
            "schema",
            "storage",
            "taxonomy",
            "fact_extraction",
            "rules",
            "classifier",
            "current_state_consumer",
            "llm_adjudication",
            "llm_executor",
            "migrations",
            "orchestrator",
            "queue_store",
            "response_validator",
        ];
        assert_eq!(submodules.len(), 15);
    }
}
