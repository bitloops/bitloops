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
    PRIMARY KEY (repo_id, source_symbol_id, target_symbol_id)
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
    PRIMARY KEY (repo_id, source_symbol_id, target_symbol_id)
);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_source_idx
ON symbol_clone_edges (repo_id, source_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_target_idx
ON symbol_clone_edges (repo_id, target_artefact_id);

CREATE INDEX IF NOT EXISTS symbol_clone_edges_relation_idx
ON symbol_clone_edges (repo_id, relation_kind);
"#
}
