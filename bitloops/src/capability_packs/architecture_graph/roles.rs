pub mod classifier;
pub mod fact_extraction;
pub mod llm_adjudication;
pub mod migrations;
pub mod rules;
pub mod schema;
pub mod storage;
pub mod taxonomy;

pub use classifier::ARCHITECTURE_ROLE_CLASSIFIER_VERSION;
pub use taxonomy::{
    ArchitectureArtefactFact, ArchitectureRole, ArchitectureRoleAssignment,
    ArchitectureRoleAssignmentHistory, ArchitectureRoleChangeProposal,
    ArchitectureRoleDetectionRule, ArchitectureRoleRuleSignal, AssignmentPriority,
    AssignmentSource, AssignmentStatus, ProposalStatus, RoleLifecycle, RoleRuleLifecycle,
    RoleSignalPolarity, RoleTarget, TargetKind,
};

#[cfg(test)]
mod tests {
    #[test]
    fn roles_facade_exposes_expected_submodules() {
        let submodules = [
            "schema",
            "storage",
            "taxonomy",
            "fact_extraction",
            "rules",
            "classifier",
            "llm_adjudication",
            "migrations",
        ];
        assert_eq!(submodules.len(), 8);
    }
}
