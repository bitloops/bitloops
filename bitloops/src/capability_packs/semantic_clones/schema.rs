pub fn semantic_clones_postgres_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_clone_edges (
    repo_id TEXT NOT NULL,
    source_symbol_id TEXT NOT NULL,
    source_artefact_id TEXT NOT NULL,
    target_symbol_id TEXT NOT NULL,
    target_artefact_id TEXT NOT NULL,
    relation_kind TEXT NOT NULL,
    score REAL NOT NULL,
    semantic_score REAL NOT NULL,
    lexical_score REAL NOT NULL,
    structural_score REAL NOT NULL,
    clone_input_hash TEXT NOT NULL,
    explanation_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    generated_at DATETIME DEFAULT now(),
    PRIMARY KEY (repo_id, source_artefact_id, target_artefact_id)
);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_source_idx
ON symbol_clone_edges (repo_id, source_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_target_idx
ON symbol_clone_edges (repo_id, target_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_relation_idx
ON symbol_clone_edges (repo_id, relation_kind);

CREATE TABLE IF NOT EXISTS symbol_clone_edges_current (
    repo_id TEXT NOT NULL,
    source_symbol_id TEXT NOT NULL,
    source_artefact_id TEXT NOT NULL,
    target_symbol_id TEXT NOT NULL,
    target_artefact_id TEXT NOT NULL,
    relation_kind TEXT NOT NULL,
    score REAL NOT NULL,
    semantic_score REAL NOT NULL,
    lexical_score REAL NOT NULL,
    structural_score REAL NOT NULL,
    clone_input_hash TEXT NOT NULL,
    explanation_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    generated_at DATETIME DEFAULT now(),
    PRIMARY KEY (repo_id, source_symbol_id, target_symbol_id)
);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_current_source_idx
ON symbol_clone_edges_current (repo_id, source_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_current_target_idx
ON symbol_clone_edges_current (repo_id, target_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_current_relation_idx
ON symbol_clone_edges_current (repo_id, relation_kind);
"#
}

pub fn semantic_clones_postgres_shared_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_clone_edges (
    repo_id TEXT NOT NULL,
    source_symbol_id TEXT NOT NULL,
    source_artefact_id TEXT NOT NULL,
    target_symbol_id TEXT NOT NULL,
    target_artefact_id TEXT NOT NULL,
    relation_kind TEXT NOT NULL,
    score REAL NOT NULL,
    semantic_score REAL NOT NULL,
    lexical_score REAL NOT NULL,
    structural_score REAL NOT NULL,
    clone_input_hash TEXT NOT NULL,
    explanation_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    generated_at DATETIME DEFAULT now(),
    PRIMARY KEY (repo_id, source_artefact_id, target_artefact_id)
);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_source_idx
ON symbol_clone_edges (repo_id, source_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_target_idx
ON symbol_clone_edges (repo_id, target_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_relation_idx
ON symbol_clone_edges (repo_id, relation_kind);
"#
}

pub fn semantic_clones_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_clone_edges (
    repo_id TEXT NOT NULL,
    source_symbol_id TEXT NOT NULL,
    source_artefact_id TEXT NOT NULL,
    target_symbol_id TEXT NOT NULL,
    target_artefact_id TEXT NOT NULL,
    relation_kind TEXT NOT NULL,
    score REAL NOT NULL,
    semantic_score REAL NOT NULL,
    lexical_score REAL NOT NULL,
    structural_score REAL NOT NULL,
    clone_input_hash TEXT NOT NULL,
    explanation_json TEXT NOT NULL DEFAULT '{}',
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (repo_id, source_artefact_id, target_artefact_id)
);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_source_idx
ON symbol_clone_edges (repo_id, source_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_target_idx
ON symbol_clone_edges (repo_id, target_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_relation_idx
ON symbol_clone_edges (repo_id, relation_kind);

CREATE TABLE IF NOT EXISTS symbol_clone_edges_current (
    repo_id TEXT NOT NULL,
    source_symbol_id TEXT NOT NULL,
    source_artefact_id TEXT NOT NULL,
    target_symbol_id TEXT NOT NULL,
    target_artefact_id TEXT NOT NULL,
    relation_kind TEXT NOT NULL,
    score REAL NOT NULL,
    semantic_score REAL NOT NULL,
    lexical_score REAL NOT NULL,
    structural_score REAL NOT NULL,
    clone_input_hash TEXT NOT NULL,
    explanation_json TEXT NOT NULL DEFAULT '{}',
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (repo_id, source_symbol_id, target_symbol_id)
);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_current_source_idx
ON symbol_clone_edges_current (repo_id, source_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_current_target_idx
ON symbol_clone_edges_current (repo_id, target_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_current_relation_idx
ON symbol_clone_edges_current (repo_id, relation_kind);
"#
}

pub fn semantic_clones_sqlite_shared_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_clone_edges (
    repo_id TEXT NOT NULL,
    source_symbol_id TEXT NOT NULL,
    source_artefact_id TEXT NOT NULL,
    target_symbol_id TEXT NOT NULL,
    target_artefact_id TEXT NOT NULL,
    relation_kind TEXT NOT NULL,
    score REAL NOT NULL,
    semantic_score REAL NOT NULL,
    lexical_score REAL NOT NULL,
    structural_score REAL NOT NULL,
    clone_input_hash TEXT NOT NULL,
    explanation_json TEXT NOT NULL DEFAULT '{}',
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (repo_id, source_artefact_id, target_artefact_id)
);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_source_idx
ON symbol_clone_edges (repo_id, source_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_target_idx
ON symbol_clone_edges (repo_id, target_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_relation_idx
ON symbol_clone_edges (repo_id, relation_kind);
"#
}

pub fn semantic_clones_sqlite_current_projection_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_clone_edges_current (
    repo_id TEXT NOT NULL,
    source_symbol_id TEXT NOT NULL,
    source_artefact_id TEXT NOT NULL,
    target_symbol_id TEXT NOT NULL,
    target_artefact_id TEXT NOT NULL,
    relation_kind TEXT NOT NULL,
    score REAL NOT NULL,
    semantic_score REAL NOT NULL,
    lexical_score REAL NOT NULL,
    structural_score REAL NOT NULL,
    clone_input_hash TEXT NOT NULL,
    explanation_json TEXT NOT NULL DEFAULT '{}',
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (repo_id, source_symbol_id, target_symbol_id)
);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_current_source_idx
ON symbol_clone_edges_current (repo_id, source_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_current_target_idx
ON symbol_clone_edges_current (repo_id, target_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_current_relation_idx
ON symbol_clone_edges_current (repo_id, relation_kind);
"#
}

#[cfg(test)]
mod tests {
    use super::{
        semantic_clones_postgres_schema_sql, semantic_clones_postgres_shared_schema_sql,
        semantic_clones_sqlite_current_projection_schema_sql, semantic_clones_sqlite_schema_sql,
        semantic_clones_sqlite_shared_schema_sql,
    };

    #[test]
    fn semantic_clone_legacy_schemas_still_include_both_table_families() {
        let postgres = semantic_clones_postgres_schema_sql();
        let sqlite = semantic_clones_sqlite_schema_sql();
        for sql in [postgres, sqlite] {
            assert!(sql.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges ("));
            assert!(sql.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges_current ("));
        }
    }

    #[test]
    fn semantic_clone_split_schemas_follow_current_vs_shared_boundary() {
        let postgres = semantic_clones_postgres_shared_schema_sql();
        assert!(postgres.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges ("));
        assert!(!postgres.contains("symbol_clone_edges_current"));

        let sqlite_shared = semantic_clones_sqlite_shared_schema_sql();
        assert!(sqlite_shared.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges ("));
        assert!(!sqlite_shared.contains("symbol_clone_edges_current"));

        let sqlite_current = semantic_clones_sqlite_current_projection_schema_sql();
        assert!(sqlite_current.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges_current ("));
        assert!(!sqlite_current.contains("CREATE TABLE IF NOT EXISTS symbol_clone_edges ("));
    }
}
