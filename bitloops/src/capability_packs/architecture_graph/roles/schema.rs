pub const ARCHITECTURE_ROLE_TABLES: &[&str] = &[
    "architecture_roles",
    "architecture_role_aliases",
    "architecture_role_detection_rules",
    "architecture_role_assignments",
    "architecture_role_change_proposals",
    "architecture_role_assignment_migrations",
];

pub fn architecture_graph_roles_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS architecture_roles (
    repo_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    canonical_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    family TEXT,
    lifecycle_status TEXT NOT NULL DEFAULT 'active',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, role_id),
    UNIQUE (repo_id, canonical_key)
);

CREATE INDEX IF NOT EXISTS architecture_roles_repo_status_idx
ON architecture_roles (repo_id, lifecycle_status, canonical_key);

CREATE TABLE IF NOT EXISTS architecture_role_aliases (
    repo_id TEXT NOT NULL,
    alias_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    alias_key TEXT NOT NULL,
    alias_normalized TEXT NOT NULL,
    source_kind TEXT NOT NULL DEFAULT 'manual',
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, alias_id),
    UNIQUE (repo_id, alias_normalized)
);

CREATE INDEX IF NOT EXISTS architecture_role_aliases_role_idx
ON architecture_role_aliases (repo_id, role_id);

CREATE TABLE IF NOT EXISTS architecture_role_detection_rules (
    repo_id TEXT NOT NULL,
    rule_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    lifecycle_status TEXT NOT NULL DEFAULT 'draft',
    canonical_hash TEXT NOT NULL,
    candidate_selector_json TEXT NOT NULL DEFAULT '{}',
    positive_conditions_json TEXT NOT NULL DEFAULT '[]',
    negative_conditions_json TEXT NOT NULL DEFAULT '[]',
    score_json TEXT NOT NULL DEFAULT '{}',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    metadata_json TEXT NOT NULL DEFAULT '{}',
    supersedes_rule_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, rule_id)
);

CREATE INDEX IF NOT EXISTS architecture_role_rules_repo_role_idx
ON architecture_role_detection_rules (repo_id, role_id, lifecycle_status, version);

CREATE INDEX IF NOT EXISTS architecture_role_rules_repo_hash_idx
ON architecture_role_detection_rules (repo_id, role_id, canonical_hash);

CREATE TABLE IF NOT EXISTS architecture_role_assignments (
    repo_id TEXT NOT NULL,
    assignment_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    source_kind TEXT NOT NULL DEFAULT 'manual',
    confidence REAL NOT NULL DEFAULT 1.0,
    status TEXT NOT NULL DEFAULT 'active',
    status_reason TEXT NOT NULL DEFAULT '',
    rule_id TEXT,
    migration_id TEXT,
    migrated_to_assignment_id TEXT,
    provenance_json TEXT NOT NULL DEFAULT '{}',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, assignment_id)
);

CREATE INDEX IF NOT EXISTS architecture_role_assignments_repo_role_idx
ON architecture_role_assignments (repo_id, role_id, status);

CREATE INDEX IF NOT EXISTS architecture_role_assignments_repo_artefact_idx
ON architecture_role_assignments (repo_id, artefact_id, status);

CREATE TABLE IF NOT EXISTS architecture_role_change_proposals (
    repo_id TEXT NOT NULL,
    proposal_id TEXT NOT NULL,
    proposal_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft',
    request_payload_json TEXT NOT NULL DEFAULT '{}',
    preview_payload_json TEXT NOT NULL DEFAULT '{}',
    result_payload_json TEXT NOT NULL DEFAULT '{}',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    applied_at TEXT,
    PRIMARY KEY (repo_id, proposal_id)
);

CREATE INDEX IF NOT EXISTS architecture_role_proposals_repo_status_idx
ON architecture_role_change_proposals (repo_id, status, proposal_type);

CREATE TABLE IF NOT EXISTS architecture_role_assignment_migrations (
    repo_id TEXT NOT NULL,
    migration_id TEXT NOT NULL,
    proposal_id TEXT NOT NULL,
    migration_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    source_role_id TEXT,
    target_role_id TEXT,
    summary_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, migration_id)
);

CREATE INDEX IF NOT EXISTS architecture_role_migrations_repo_proposal_idx
ON architecture_role_assignment_migrations (repo_id, proposal_id, created_at);
"#
}

#[cfg(test)]
mod tests {
    use super::architecture_graph_roles_sqlite_schema_sql;

    #[test]
    fn schema_includes_role_storage_tables() {
        let sql = architecture_graph_roles_sqlite_schema_sql();
        for table in [
            "architecture_roles",
            "architecture_role_aliases",
            "architecture_role_detection_rules",
            "architecture_role_assignments",
            "architecture_role_change_proposals",
            "architecture_role_assignment_migrations",
        ] {
            assert!(sql.contains(table), "schema should include {table}");
        }
    }
}
