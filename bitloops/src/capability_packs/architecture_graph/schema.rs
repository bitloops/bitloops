use std::sync::OnceLock;

use crate::host::capability_host::SchemaModule;

use super::roles::schema::architecture_graph_roles_sqlite_schema_sql;
use super::types::ARCHITECTURE_GRAPH_CAPABILITY_ID;

pub static ARCHITECTURE_GRAPH_SCHEMA_MODULE: SchemaModule = SchemaModule {
    capability_id: ARCHITECTURE_GRAPH_CAPABILITY_ID,
    name: "architecture_graph",
    description: "Architecture graph capability fact and assertion schema",
};

const ARCHITECTURE_GRAPH_CORE_SQLITE_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS architecture_graph_nodes_current (
    repo_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    node_kind TEXT NOT NULL,
    label TEXT NOT NULL,
    artefact_id TEXT,
    symbol_id TEXT,
    path TEXT,
    entry_kind TEXT,
    source_kind TEXT NOT NULL,
    confidence REAL NOT NULL,
    provenance_json TEXT NOT NULL DEFAULT '{}',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    properties_json TEXT NOT NULL DEFAULT '{}',
    last_observed_generation INTEGER,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, node_id)
);

CREATE INDEX IF NOT EXISTS architecture_graph_nodes_kind_idx
ON architecture_graph_nodes_current (repo_id, node_kind);

CREATE INDEX IF NOT EXISTS architecture_graph_nodes_path_idx
ON architecture_graph_nodes_current (repo_id, path);

CREATE INDEX IF NOT EXISTS architecture_graph_nodes_artefact_idx
ON architecture_graph_nodes_current (repo_id, artefact_id);

CREATE TABLE IF NOT EXISTS architecture_graph_edges_current (
    repo_id TEXT NOT NULL,
    edge_id TEXT NOT NULL,
    edge_kind TEXT NOT NULL,
    from_node_id TEXT NOT NULL,
    to_node_id TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    confidence REAL NOT NULL,
    provenance_json TEXT NOT NULL DEFAULT '{}',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    properties_json TEXT NOT NULL DEFAULT '{}',
    last_observed_generation INTEGER,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, edge_id)
);

CREATE INDEX IF NOT EXISTS architecture_graph_edges_kind_idx
ON architecture_graph_edges_current (repo_id, edge_kind);

CREATE INDEX IF NOT EXISTS architecture_graph_edges_from_idx
ON architecture_graph_edges_current (repo_id, from_node_id);

CREATE INDEX IF NOT EXISTS architecture_graph_edges_to_idx
ON architecture_graph_edges_current (repo_id, to_node_id);

CREATE TABLE IF NOT EXISTS architecture_graph_assertions (
    assertion_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    action TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    node_id TEXT,
    node_kind TEXT,
    edge_id TEXT,
    edge_kind TEXT,
    from_node_id TEXT,
    to_node_id TEXT,
    label TEXT,
    artefact_id TEXT,
    symbol_id TEXT,
    path TEXT,
    entry_kind TEXT,
    reason TEXT NOT NULL,
    source TEXT NOT NULL,
    confidence REAL,
    provenance_json TEXT NOT NULL DEFAULT '{}',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    properties_json TEXT NOT NULL DEFAULT '{}',
    revoked_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK (target_kind IN ('NODE', 'EDGE')),
    CHECK (action IN ('ASSERT', 'SUPPRESS', 'ANNOTATE'))
);

CREATE INDEX IF NOT EXISTS architecture_graph_assertions_repo_idx
ON architecture_graph_assertions (repo_id, target_kind, action, revoked_at);

CREATE INDEX IF NOT EXISTS architecture_graph_assertions_node_idx
ON architecture_graph_assertions (repo_id, node_id, revoked_at);

CREATE INDEX IF NOT EXISTS architecture_graph_assertions_edge_idx
ON architecture_graph_assertions (repo_id, edge_id, revoked_at);

CREATE TABLE IF NOT EXISTS architecture_graph_runs_current (
    repo_id TEXT PRIMARY KEY,
    last_generation_seq INTEGER,
    status TEXT NOT NULL,
    warnings_json TEXT NOT NULL DEFAULT '[]',
    metrics_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

pub fn architecture_graph_sqlite_schema_sql() -> &'static str {
    static ARCHITECTURE_GRAPH_SCHEMA_SQL: OnceLock<String> = OnceLock::new();
    ARCHITECTURE_GRAPH_SCHEMA_SQL
        .get_or_init(|| {
            format!(
                "{}{}",
                ARCHITECTURE_GRAPH_CORE_SQLITE_SCHEMA_SQL,
                architecture_graph_roles_sqlite_schema_sql()
            )
        })
        .as_str()
}

#[cfg(test)]
mod tests {
    use super::architecture_graph_sqlite_schema_sql;

    #[test]
    fn schema_includes_graph_and_assertion_tables() {
        let sql = architecture_graph_sqlite_schema_sql();
        for table in [
            "architecture_graph_nodes_current",
            "architecture_graph_edges_current",
            "architecture_graph_assertions",
            "architecture_graph_runs_current",
            "architecture_roles",
            "architecture_role_aliases",
        ] {
            assert!(sql.contains(table), "schema should include {table}");
        }
    }

    #[test]
    fn schema_includes_role_tables() {
        let sql = architecture_graph_sqlite_schema_sql();
        for table in
            crate::capability_packs::architecture_graph::roles::schema::ARCHITECTURE_ROLE_TABLES
        {
            assert!(sql.contains(table), "schema should include {table}");
        }
    }
}
