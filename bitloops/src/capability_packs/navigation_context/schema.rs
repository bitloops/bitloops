use crate::host::capability_host::SchemaModule;

use super::types::NAVIGATION_CONTEXT_CAPABILITY_ID;

pub static NAVIGATION_CONTEXT_SCHEMA_MODULE: SchemaModule = SchemaModule {
    capability_id: NAVIGATION_CONTEXT_CAPABILITY_ID,
    name: "navigation_context",
    description: "Navigation primitive, edge, view signature, and view dependency schema",
};

pub fn navigation_context_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS navigation_context_primitives_current (
    repo_id TEXT NOT NULL,
    primitive_id TEXT NOT NULL,
    primitive_kind TEXT NOT NULL,
    identity_key TEXT NOT NULL,
    label TEXT NOT NULL,
    path TEXT,
    artefact_id TEXT,
    symbol_id TEXT,
    source_kind TEXT NOT NULL,
    confidence REAL NOT NULL,
    primitive_hash TEXT NOT NULL,
    hash_version TEXT NOT NULL,
    properties_json TEXT NOT NULL DEFAULT '{}',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    last_observed_generation INTEGER,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, primitive_id)
);

CREATE INDEX IF NOT EXISTS navigation_context_primitives_kind_idx
ON navigation_context_primitives_current (repo_id, primitive_kind);

CREATE INDEX IF NOT EXISTS navigation_context_primitives_path_idx
ON navigation_context_primitives_current (repo_id, path);

CREATE INDEX IF NOT EXISTS navigation_context_primitives_artefact_idx
ON navigation_context_primitives_current (repo_id, artefact_id);

CREATE INDEX IF NOT EXISTS navigation_context_primitives_symbol_idx
ON navigation_context_primitives_current (repo_id, symbol_id);

CREATE TABLE IF NOT EXISTS navigation_context_edges_current (
    repo_id TEXT NOT NULL,
    edge_id TEXT NOT NULL,
    edge_kind TEXT NOT NULL,
    from_primitive_id TEXT NOT NULL,
    to_primitive_id TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    confidence REAL NOT NULL,
    edge_hash TEXT NOT NULL,
    hash_version TEXT NOT NULL,
    properties_json TEXT NOT NULL DEFAULT '{}',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    last_observed_generation INTEGER,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, edge_id)
);

CREATE INDEX IF NOT EXISTS navigation_context_edges_kind_idx
ON navigation_context_edges_current (repo_id, edge_kind);

CREATE INDEX IF NOT EXISTS navigation_context_edges_from_idx
ON navigation_context_edges_current (repo_id, from_primitive_id);

CREATE INDEX IF NOT EXISTS navigation_context_edges_to_idx
ON navigation_context_edges_current (repo_id, to_primitive_id);

CREATE TABLE IF NOT EXISTS navigation_context_views_current (
    repo_id TEXT NOT NULL,
    view_id TEXT NOT NULL,
    view_kind TEXT NOT NULL,
    label TEXT NOT NULL,
    view_query_version TEXT NOT NULL,
    dependency_query_json TEXT NOT NULL DEFAULT '{}',
    accepted_signature TEXT NOT NULL,
    current_signature TEXT NOT NULL,
    status TEXT NOT NULL,
    stale_reason_json TEXT NOT NULL DEFAULT '{}',
    materialised_ref TEXT,
    last_observed_generation INTEGER,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, view_id),
    CHECK (status IN ('fresh', 'stale'))
);

CREATE INDEX IF NOT EXISTS navigation_context_views_status_idx
ON navigation_context_views_current (repo_id, status, view_kind);

CREATE TABLE IF NOT EXISTS navigation_context_view_dependencies_current (
    repo_id TEXT NOT NULL,
    view_id TEXT NOT NULL,
    primitive_id TEXT NOT NULL,
    primitive_kind TEXT NOT NULL,
    primitive_hash TEXT NOT NULL,
    dependency_role TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, view_id, primitive_id)
);

CREATE INDEX IF NOT EXISTS navigation_context_view_dependencies_primitive_idx
ON navigation_context_view_dependencies_current (repo_id, primitive_id);
"#
}

#[cfg(test)]
mod tests {
    use super::navigation_context_sqlite_schema_sql;

    #[test]
    fn schema_includes_primitive_edge_view_and_dependency_tables() {
        let sql = navigation_context_sqlite_schema_sql();
        for table in [
            "navigation_context_primitives_current",
            "navigation_context_edges_current",
            "navigation_context_views_current",
            "navigation_context_view_dependencies_current",
        ] {
            assert!(sql.contains(table), "schema should include {table}");
        }
    }
}
