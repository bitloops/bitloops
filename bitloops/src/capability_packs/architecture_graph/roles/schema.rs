pub const ARCHITECTURE_ROLE_TABLES: &[&str] = &[
    "architecture_roles",
    "architecture_role_aliases",
    "architecture_role_detection_rules",
    "architecture_artefact_facts_current",
    "architecture_role_rule_signals_current",
    "architecture_role_assignments_current",
    "architecture_role_assignment_history",
    "architecture_role_assignments",
    "architecture_role_change_proposals",
    "architecture_role_assignment_migrations",
    "architecture_role_adjudication_attempts",
];

pub fn architecture_graph_roles_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS architecture_roles (
    repo_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    family TEXT NOT NULL DEFAULT 'unclassified',
    canonical_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    lifecycle_status TEXT NOT NULL DEFAULT 'active',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, role_id),
    UNIQUE (repo_id, canonical_key)
);

CREATE INDEX IF NOT EXISTS architecture_roles_repo_lifecycle_idx
ON architecture_roles (repo_id, lifecycle_status, family, canonical_key);

CREATE INDEX IF NOT EXISTS architecture_roles_repo_status_idx
ON architecture_roles (repo_id, lifecycle_status, canonical_key);

CREATE TABLE IF NOT EXISTS architecture_role_aliases (
    repo_id TEXT NOT NULL,
    alias_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    alias_key TEXT NOT NULL DEFAULT '',
    alias_normalized TEXT NOT NULL,
    source_kind TEXT NOT NULL DEFAULT 'manual',
    provenance_json TEXT NOT NULL DEFAULT '{}',
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
    canonical_hash TEXT NOT NULL DEFAULT '',
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
    PRIMARY KEY (repo_id, rule_id, version)
);

CREATE INDEX IF NOT EXISTS architecture_role_rules_repo_role_idx
ON architecture_role_detection_rules (repo_id, role_id, version);

CREATE INDEX IF NOT EXISTS architecture_role_rules_repo_status_idx
ON architecture_role_detection_rules (repo_id, role_id, lifecycle_status, version);

CREATE INDEX IF NOT EXISTS architecture_role_rules_repo_hash_idx
ON architecture_role_detection_rules (repo_id, role_id, canonical_hash);

CREATE TABLE IF NOT EXISTS architecture_artefact_facts_current (
    repo_id TEXT NOT NULL,
    fact_id TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    artefact_id TEXT,
    symbol_id TEXT,
    path TEXT NOT NULL,
    language TEXT,
    fact_kind TEXT NOT NULL,
    fact_key TEXT NOT NULL,
    fact_value TEXT NOT NULL,
    source TEXT NOT NULL,
    confidence REAL NOT NULL,
    evidence_json TEXT NOT NULL DEFAULT '[]',
    generation_seq INTEGER NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, fact_id)
);

CREATE INDEX IF NOT EXISTS architecture_role_facts_path_idx
ON architecture_artefact_facts_current (repo_id, path, fact_kind, fact_key);

CREATE TABLE IF NOT EXISTS architecture_role_rule_signals_current (
    repo_id TEXT NOT NULL,
    signal_id TEXT NOT NULL,
    rule_id TEXT NOT NULL,
    rule_version INTEGER NOT NULL,
    role_id TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    artefact_id TEXT,
    symbol_id TEXT,
    path TEXT NOT NULL,
    polarity TEXT NOT NULL,
    score REAL NOT NULL,
    evidence_json TEXT NOT NULL DEFAULT '[]',
    generation_seq INTEGER NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, signal_id)
);

CREATE INDEX IF NOT EXISTS architecture_role_rule_signals_path_idx
ON architecture_role_rule_signals_current (repo_id, path, role_id, polarity);

CREATE TABLE IF NOT EXISTS architecture_role_assignments_current (
    repo_id TEXT NOT NULL,
    assignment_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    artefact_id TEXT,
    symbol_id TEXT,
    path TEXT NOT NULL,
    priority TEXT NOT NULL,
    status TEXT NOT NULL,
    source TEXT NOT NULL,
    confidence REAL NOT NULL,
    evidence_json TEXT NOT NULL DEFAULT '[]',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    classifier_version TEXT NOT NULL,
    rule_version INTEGER,
    generation_seq INTEGER NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, assignment_id)
);

CREATE INDEX IF NOT EXISTS architecture_role_assignments_current_path_idx
ON architecture_role_assignments_current (repo_id, path, status, priority);

CREATE INDEX IF NOT EXISTS architecture_role_assignments_current_role_idx
ON architecture_role_assignments_current (repo_id, role_id, status);

CREATE TABLE IF NOT EXISTS architecture_role_assignment_history (
    repo_id TEXT NOT NULL,
    history_id TEXT NOT NULL,
    assignment_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    artefact_id TEXT,
    symbol_id TEXT,
    path TEXT NOT NULL,
    previous_status TEXT,
    new_status TEXT NOT NULL,
    previous_confidence REAL,
    new_confidence REAL NOT NULL,
    change_kind TEXT NOT NULL,
    evidence_json TEXT NOT NULL DEFAULT '[]',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    generation_seq INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, history_id)
);

CREATE INDEX IF NOT EXISTS architecture_role_assignment_history_assignment_idx
ON architecture_role_assignment_history (repo_id, assignment_id, generation_seq);

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
    proposal_type TEXT NOT NULL DEFAULT '',
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

CREATE TABLE IF NOT EXISTS architecture_role_adjudication_attempts (
    repo_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    generation_seq INTEGER NOT NULL,
    target_kind TEXT,
    artefact_id TEXT,
    symbol_id TEXT,
    path TEXT,
    reason TEXT NOT NULL,
    deterministic_confidence REAL,
    candidate_roles_json TEXT NOT NULL DEFAULT '[]',
    current_assignment_json TEXT,
    request_json TEXT NOT NULL DEFAULT '{}',
    evidence_packet_sha256 TEXT NOT NULL,
    evidence_packet_json TEXT NOT NULL DEFAULT '{}',
    model_descriptor TEXT NOT NULL DEFAULT '',
    slot_name TEXT NOT NULL DEFAULT '',
    outcome TEXT NOT NULL,
    raw_response_json TEXT,
    validated_result_json TEXT,
    failure_message TEXT,
    retryable INTEGER NOT NULL DEFAULT 0,
    assignment_write_persisted INTEGER NOT NULL DEFAULT 0,
    assignment_write_source TEXT,
    observed_at_unix INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, attempt_id)
);

CREATE INDEX IF NOT EXISTS architecture_role_adjudication_attempts_scope_idx
ON architecture_role_adjudication_attempts (repo_id, scope_key, observed_at_unix DESC);

CREATE INDEX IF NOT EXISTS architecture_role_adjudication_attempts_outcome_idx
ON architecture_role_adjudication_attempts (repo_id, outcome, observed_at_unix DESC);

CREATE INDEX IF NOT EXISTS architecture_role_adjudication_attempts_path_idx
ON architecture_role_adjudication_attempts (repo_id, path, observed_at_unix DESC);
"#
}

#[cfg(test)]
mod tests {
    use super::{ARCHITECTURE_ROLE_TABLES, architecture_graph_roles_sqlite_schema_sql};

    #[test]
    fn schema_includes_role_storage_tables() {
        let sql = architecture_graph_roles_sqlite_schema_sql();
        for table in ARCHITECTURE_ROLE_TABLES {
            assert!(sql.contains(table), "schema should include {table}");
        }
    }
}
