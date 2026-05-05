pub const ARCHITECTURE_ROLE_TABLES: &[&str] = &[
    "architecture_roles",
    "architecture_role_aliases",
    "architecture_role_detection_rules",
    "architecture_artefact_facts_current",
    "architecture_role_rule_signals_current",
    "architecture_role_assignments_current",
    "architecture_role_assignment_history",
    "architecture_role_change_proposals",
];

pub fn architecture_roles_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS architecture_roles (
    repo_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    family TEXT NOT NULL,
    slug TEXT NOT NULL,
    display_name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    lifecycle TEXT NOT NULL,
    provenance_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, role_id),
    UNIQUE (repo_id, family, slug),
    CHECK (lifecycle IN ('active', 'deprecated', 'removed'))
);

CREATE INDEX IF NOT EXISTS architecture_roles_lifecycle_idx
ON architecture_roles (repo_id, lifecycle);

CREATE TABLE IF NOT EXISTS architecture_role_aliases (
    repo_id TEXT NOT NULL,
    alias_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    alias TEXT NOT NULL,
    provenance_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, alias_id),
    UNIQUE (repo_id, alias)
);

CREATE INDEX IF NOT EXISTS architecture_role_aliases_role_idx
ON architecture_role_aliases (repo_id, role_id);

CREATE TABLE IF NOT EXISTS architecture_role_detection_rules (
    repo_id TEXT NOT NULL,
    rule_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    lifecycle TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 100,
    score REAL NOT NULL,
    candidate_selector_json TEXT NOT NULL,
    positive_conditions_json TEXT NOT NULL DEFAULT '[]',
    negative_conditions_json TEXT NOT NULL DEFAULT '[]',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, rule_id, version),
    CHECK (lifecycle IN ('draft', 'active', 'disabled', 'deprecated'))
);

CREATE INDEX IF NOT EXISTS architecture_role_detection_rules_active_idx
ON architecture_role_detection_rules (repo_id, lifecycle, priority);

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
    PRIMARY KEY (repo_id, fact_id),
    CHECK (target_kind IN ('file', 'artefact', 'symbol'))
);

CREATE INDEX IF NOT EXISTS architecture_artefact_facts_target_idx
ON architecture_artefact_facts_current (repo_id, target_kind, artefact_id, symbol_id, path);

CREATE INDEX IF NOT EXISTS architecture_artefact_facts_lookup_idx
ON architecture_artefact_facts_current (repo_id, fact_kind, fact_key, fact_value);

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
    PRIMARY KEY (repo_id, signal_id),
    CHECK (target_kind IN ('file', 'artefact', 'symbol')),
    CHECK (polarity IN ('positive', 'negative'))
);

CREATE INDEX IF NOT EXISTS architecture_role_rule_signals_target_idx
ON architecture_role_rule_signals_current (repo_id, target_kind, artefact_id, symbol_id, path);

CREATE INDEX IF NOT EXISTS architecture_role_rule_signals_role_idx
ON architecture_role_rule_signals_current (repo_id, role_id, polarity);

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
    PRIMARY KEY (repo_id, assignment_id),
    CHECK (target_kind IN ('file', 'artefact', 'symbol')),
    CHECK (priority IN ('primary', 'secondary')),
    CHECK (status IN ('active', 'stale', 'needs_review', 'rejected')),
    CHECK (source IN ('rule', 'llm', 'human', 'migration'))
);

CREATE INDEX IF NOT EXISTS architecture_role_assignments_target_idx
ON architecture_role_assignments_current (repo_id, target_kind, artefact_id, symbol_id, path, status);

CREATE INDEX IF NOT EXISTS architecture_role_assignments_role_idx
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
    changed_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, history_id),
    CHECK (target_kind IN ('file', 'artefact', 'symbol')),
    CHECK (previous_status IS NULL OR previous_status IN ('active', 'stale', 'needs_review', 'rejected')),
    CHECK (new_status IN ('active', 'stale', 'needs_review', 'rejected'))
);

CREATE INDEX IF NOT EXISTS architecture_role_assignment_history_assignment_idx
ON architecture_role_assignment_history (repo_id, assignment_id, generation_seq);

CREATE TABLE IF NOT EXISTS architecture_role_change_proposals (
    repo_id TEXT NOT NULL,
    proposal_id TEXT NOT NULL,
    proposal_kind TEXT NOT NULL,
    status TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    impact_preview_json TEXT NOT NULL DEFAULT '{}',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    applied_at TEXT,
    PRIMARY KEY (repo_id, proposal_id),
    CHECK (status IN ('draft', 'previewed', 'applied', 'rejected'))
);

CREATE INDEX IF NOT EXISTS architecture_role_change_proposals_status_idx
ON architecture_role_change_proposals (repo_id, status, proposal_kind);
"#
}

#[cfg(test)]
mod tests {
    use super::{ARCHITECTURE_ROLE_TABLES, architecture_roles_sqlite_schema_sql};

    #[test]
    fn role_schema_includes_all_contract_tables() {
        let sql = architecture_roles_sqlite_schema_sql();
        for table in ARCHITECTURE_ROLE_TABLES {
            assert!(sql.contains(table), "schema should include {table}");
        }
    }

    #[test]
    fn role_schema_includes_lifecycle_and_status_checks() {
        let sql = architecture_roles_sqlite_schema_sql();
        assert!(sql.contains("CHECK (lifecycle IN ('active', 'deprecated', 'removed'))"));
        assert!(sql.contains("CHECK (lifecycle IN ('draft', 'active', 'disabled', 'deprecated'))"));
        assert!(sql.contains("CHECK (status IN ('active', 'stale', 'needs_review', 'rejected'))"));
        assert!(
            sql.contains(
                "CHECK (previous_status IS NULL OR previous_status IN ('active', 'stale', 'needs_review', 'rejected'))"
            )
        );
        assert!(
            sql.contains("CHECK (new_status IN ('active', 'stale', 'needs_review', 'rejected'))")
        );
        assert!(sql.contains("CHECK (source IN ('rule', 'llm', 'human', 'migration'))"));
    }

    #[test]
    fn role_schema_includes_query_indexes() {
        let sql = architecture_roles_sqlite_schema_sql();
        for index in [
            "architecture_roles_lifecycle_idx",
            "architecture_role_detection_rules_active_idx",
            "architecture_artefact_facts_target_idx",
            "architecture_role_rule_signals_target_idx",
            "architecture_role_assignments_target_idx",
            "architecture_role_assignment_history_assignment_idx",
            "architecture_role_change_proposals_status_idx",
        ] {
            assert!(sql.contains(index), "schema should include {index}");
        }
    }

    #[test]
    fn role_schema_applies_to_empty_sqlite_database() -> anyhow::Result<()> {
        let conn = rusqlite::Connection::open_in_memory()?;
        conn.execute_batch(architecture_roles_sqlite_schema_sql())?;

        for table in ARCHITECTURE_ROLE_TABLES {
            let table_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )?;
            assert_eq!(table_count, 1, "SQLite schema should create {table}");
        }
        Ok(())
    }
}
