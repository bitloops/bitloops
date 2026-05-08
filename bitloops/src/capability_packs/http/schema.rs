use crate::host::capability_host::SchemaModule;

use super::types::HTTP_CAPABILITY_ID;

pub static HTTP_SCHEMA_MODULE: SchemaModule = SchemaModule {
    capability_id: HTTP_CAPABILITY_ID,
    name: "http",
    description: "HTTP protocol primitive, bundle, evidence, and role-query projection schema",
};

pub fn http_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS http_primitives_current (
    repo_id TEXT NOT NULL,
    primitive_id TEXT NOT NULL,
    owner TEXT NOT NULL,
    primitive_type TEXT NOT NULL,
    subject TEXT NOT NULL,
    roles_json TEXT NOT NULL DEFAULT '[]',
    terms_json TEXT NOT NULL DEFAULT '[]',
    properties_json TEXT NOT NULL DEFAULT '{}',
    confidence_level TEXT NOT NULL DEFAULT 'MEDIUM',
    confidence_score REAL NOT NULL DEFAULT 0.5,
    status TEXT NOT NULL DEFAULT 'active',
    input_fingerprint TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, primitive_id)
);

CREATE INDEX IF NOT EXISTS http_primitives_type_idx
ON http_primitives_current (repo_id, primitive_type, status);

CREATE INDEX IF NOT EXISTS http_primitives_owner_idx
ON http_primitives_current (repo_id, owner);

CREATE TABLE IF NOT EXISTS http_primitive_evidence_current (
    repo_id TEXT NOT NULL,
    primitive_id TEXT NOT NULL,
    evidence_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    path TEXT,
    artefact_id TEXT,
    symbol_id TEXT,
    content_id TEXT,
    start_line INTEGER,
    end_line INTEGER,
    start_byte INTEGER,
    end_byte INTEGER,
    dependency_package TEXT,
    dependency_version TEXT,
    source_url TEXT,
    excerpt_hash TEXT,
    producer TEXT,
    model TEXT,
    prompt_hash TEXT,
    properties_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, evidence_id)
);

CREATE INDEX IF NOT EXISTS http_primitive_evidence_primitive_idx
ON http_primitive_evidence_current (repo_id, primitive_id);

CREATE INDEX IF NOT EXISTS http_primitive_evidence_artefact_idx
ON http_primitive_evidence_current (repo_id, artefact_id);

CREATE INDEX IF NOT EXISTS http_primitive_evidence_symbol_idx
ON http_primitive_evidence_current (repo_id, symbol_id);

CREATE INDEX IF NOT EXISTS http_primitive_evidence_path_idx
ON http_primitive_evidence_current (repo_id, path);

CREATE TABLE IF NOT EXISTS http_primitive_links_current (
    repo_id TEXT NOT NULL,
    from_primitive_id TEXT NOT NULL,
    to_primitive_id TEXT NOT NULL,
    link_kind TEXT NOT NULL,
    properties_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, from_primitive_id, to_primitive_id, link_kind)
);

CREATE INDEX IF NOT EXISTS http_primitive_links_to_idx
ON http_primitive_links_current (repo_id, to_primitive_id);

CREATE TABLE IF NOT EXISTS http_bundles_current (
    repo_id TEXT NOT NULL,
    bundle_id TEXT NOT NULL,
    bundle_kind TEXT NOT NULL,
    risk_kind TEXT,
    severity TEXT,
    matched_roles_json TEXT NOT NULL DEFAULT '[]',
    primitive_ids_json TEXT NOT NULL DEFAULT '[]',
    upstream_facts_json TEXT NOT NULL DEFAULT '[]',
    causal_chain_json TEXT NOT NULL DEFAULT '[]',
    invalidated_assumptions_json TEXT NOT NULL DEFAULT '[]',
    obligations_json TEXT NOT NULL DEFAULT '[]',
    confidence_level TEXT NOT NULL DEFAULT 'MEDIUM',
    confidence_score REAL NOT NULL DEFAULT 0.5,
    status TEXT NOT NULL DEFAULT 'active',
    input_fingerprint TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, bundle_id)
);

CREATE INDEX IF NOT EXISTS http_bundles_kind_idx
ON http_bundles_current (repo_id, bundle_kind, status);

CREATE INDEX IF NOT EXISTS http_bundles_risk_idx
ON http_bundles_current (repo_id, risk_kind, severity);

CREATE TABLE IF NOT EXISTS http_query_index_current (
    repo_id TEXT NOT NULL,
    owner TEXT NOT NULL,
    fact_id TEXT NOT NULL,
    bundle_id TEXT,
    terms_json TEXT NOT NULL DEFAULT '[]',
    roles_json TEXT NOT NULL DEFAULT '[]',
    subject TEXT NOT NULL,
    path TEXT,
    symbol_id TEXT,
    artefact_id TEXT,
    rank_signals_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, owner, fact_id)
);

CREATE INDEX IF NOT EXISTS http_query_index_bundle_idx
ON http_query_index_current (repo_id, bundle_id);

CREATE INDEX IF NOT EXISTS http_query_index_path_idx
ON http_query_index_current (repo_id, path);

CREATE INDEX IF NOT EXISTS http_query_index_symbol_idx
ON http_query_index_current (repo_id, symbol_id);

CREATE INDEX IF NOT EXISTS http_query_index_artefact_idx
ON http_query_index_current (repo_id, artefact_id);

CREATE TABLE IF NOT EXISTS http_runs_current (
    repo_id TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    warnings_json TEXT NOT NULL DEFAULT '[]',
    metrics_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
"#
}

#[cfg(test)]
mod tests {
    use super::http_sqlite_schema_sql;

    #[test]
    fn schema_includes_http_primitive_bundle_and_query_projection_tables() {
        let sql = http_sqlite_schema_sql();
        for table in [
            "http_primitives_current",
            "http_primitive_evidence_current",
            "http_primitive_links_current",
            "http_bundles_current",
            "http_query_index_current",
            "http_runs_current",
        ] {
            assert!(sql.contains(table), "schema should include {table}");
        }
    }
}
