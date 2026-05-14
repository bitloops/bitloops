mod adjudication;
mod adjudication_attempts;
mod assignments;
mod facts;
mod management;
mod management_rows;
mod proposals;
mod roles;
mod rows;
mod rules;
mod signals;

pub use adjudication::{DbRoleAssignmentWriter, DbRoleFactsReader, DbRoleTaxonomyReader};
pub use adjudication_attempts::{
    DbRoleAdjudicationAttemptWriter, RoleAdjudicationAttemptRecord,
    list_recent_role_adjudication_attempts,
};
pub use assignments::{
    AssignmentHistoryWrite, RoleClassificationStateReplacement, RoleClassificationStateWriteCounts,
    list_active_current_assignments_for_role, list_current_assignments_for_role,
    load_active_assignment_paths_not_in, load_assignments_for_path, load_assignments_for_paths,
    load_current_assignment_by_id, mark_assignments_for_paths_stale,
    migrate_current_assignment_to_role, record_assignment_history, replace_assignments_for_paths,
    replace_assignments_for_paths_with_history, replace_role_classification_state,
    retire_role_and_mark_assignments, update_current_assignment_status, upsert_assignment,
};
pub use facts::{delete_role_facts_for_paths, load_facts_for_paths, replace_facts_for_paths};
pub use management::{
    AliasConflict, ArchitectureRoleAliasRecord, ArchitectureRoleAssignmentMigrationRecord,
    ArchitectureRoleProposalRecord, ArchitectureRoleRecord, ArchitectureRoleRuleRecord,
    create_role_alias, deterministic_alias_id, deterministic_migration_id,
    deterministic_proposal_id, deterministic_role_id, deterministic_rule_id,
    insert_assignment_migration_record, insert_role_proposal, insert_role_rule,
    list_assignment_migrations_for_proposal, list_role_aliases, list_roles, load_role_by_alias,
    load_role_by_canonical_key, load_role_by_id, load_role_proposal_by_id, load_role_rule_by_id,
    load_role_rules, mark_role_proposal_applied, next_role_rule_version, normalize_role_alias,
    normalize_role_key, update_role_proposal_preview, update_role_rule_lifecycle, upsert_role,
};
pub use proposals::insert_change_proposal;
pub use roles::{load_roles, rename_role, set_role_lifecycle, upsert_classification_role};
pub use rules::{load_active_detection_rules, upsert_detection_rule};
pub use signals::replace_signals_for_paths;

#[cfg(test)]
mod management_tests;
#[cfg(test)]
mod tests;
