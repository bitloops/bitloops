use std::path::Path;

use anyhow::{Context, Result};

use super::persistence_sql::build_repair_all_current_semantic_projection_from_historical_sql;
use crate::host::devql::RelationalDialect;

pub(crate) fn semantic_features_postgres_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_semantics (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL,
    source_model TEXT,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_semantics_repo_blob_idx
ON symbol_semantics (repo_id, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_features (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    modifiers JSONB NOT NULL DEFAULT '[]'::jsonb,
    identifier_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    normalized_body_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    parent_kind TEXT,
    context_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_features_repo_blob_idx
ON symbol_features (repo_id, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_semantics_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL,
    source_model TEXT,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_semantics_current_repo_path_idx
ON symbol_semantics_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_semantics_current_repo_artefact_idx
ON symbol_semantics_current (repo_id, artefact_id);

CREATE TABLE IF NOT EXISTS symbol_features_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    semantic_features_input_hash TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    modifiers JSONB NOT NULL DEFAULT '[]'::jsonb,
    identifier_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    normalized_body_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    parent_kind TEXT,
    context_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_features_current_repo_path_idx
ON symbol_features_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_features_current_repo_artefact_idx
ON symbol_features_current (repo_id, artefact_id);
"#
}

pub(crate) fn semantic_features_postgres_shared_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_semantics (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL,
    source_model TEXT,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_semantics_repo_blob_idx
ON symbol_semantics (repo_id, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_features (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    modifiers JSONB NOT NULL DEFAULT '[]'::jsonb,
    identifier_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    normalized_body_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    parent_kind TEXT,
    context_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_features_repo_blob_idx
ON symbol_features (repo_id, blob_sha);
"#
}

pub(crate) fn semantic_features_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_semantics (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL,
    source_model TEXT,
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_semantics_repo_blob_idx
ON symbol_semantics (repo_id, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_features (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    identifier_tokens TEXT NOT NULL DEFAULT '[]',
    normalized_body_tokens TEXT NOT NULL DEFAULT '[]',
    parent_kind TEXT,
    context_tokens TEXT NOT NULL DEFAULT '[]',
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_features_repo_blob_idx
ON symbol_features (repo_id, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_semantics_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL,
    source_model TEXT,
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_semantics_current_repo_path_idx
ON symbol_semantics_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_semantics_current_repo_artefact_idx
ON symbol_semantics_current (repo_id, artefact_id);

CREATE TABLE IF NOT EXISTS symbol_features_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    semantic_features_input_hash TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    identifier_tokens TEXT NOT NULL DEFAULT '[]',
    normalized_body_tokens TEXT NOT NULL DEFAULT '[]',
    parent_kind TEXT,
    context_tokens TEXT NOT NULL DEFAULT '[]',
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_features_current_repo_path_idx
ON symbol_features_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_features_current_repo_artefact_idx
ON symbol_features_current (repo_id, artefact_id);
"#
}

pub(crate) fn semantic_features_sqlite_shared_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_semantics (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL,
    source_model TEXT,
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_semantics_repo_blob_idx
ON symbol_semantics (repo_id, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_features (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    identifier_tokens TEXT NOT NULL DEFAULT '[]',
    normalized_body_tokens TEXT NOT NULL DEFAULT '[]',
    parent_kind TEXT,
    context_tokens TEXT NOT NULL DEFAULT '[]',
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_features_repo_blob_idx
ON symbol_features (repo_id, blob_sha);
"#
}

pub(crate) fn semantic_features_sqlite_current_projection_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_semantics_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL,
    source_model TEXT,
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_semantics_current_repo_path_idx
ON symbol_semantics_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_semantics_current_repo_artefact_idx
ON symbol_semantics_current (repo_id, artefact_id);

CREATE TABLE IF NOT EXISTS symbol_features_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    semantic_features_input_hash TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    identifier_tokens TEXT NOT NULL DEFAULT '[]',
    normalized_body_tokens TEXT NOT NULL DEFAULT '[]',
    parent_kind TEXT,
    context_tokens TEXT NOT NULL DEFAULT '[]',
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_features_current_repo_path_idx
ON symbol_features_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_features_current_repo_artefact_idx
ON symbol_features_current (repo_id, artefact_id);
"#
}

pub(crate) fn semantic_features_postgres_upgrade_sql() -> &'static str {
    r#"
ALTER TABLE symbol_semantics ADD COLUMN IF NOT EXISTS docstring_summary TEXT;
ALTER TABLE symbol_features ADD COLUMN IF NOT EXISTS modifiers JSONB DEFAULT '[]'::jsonb;
ALTER TABLE IF EXISTS symbol_semantics ALTER COLUMN confidence DROP NOT NULL;
ALTER TABLE IF EXISTS symbol_semantics_current ALTER COLUMN confidence DROP NOT NULL;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'symbol_semantics'
          AND column_name = 'doc_comment_summary'
    ) THEN
        UPDATE symbol_semantics
        SET docstring_summary = doc_comment_summary
        WHERE docstring_summary IS NULL AND doc_comment_summary IS NOT NULL;
    END IF;
END $$;

DO $$
BEGIN
    IF to_regclass('artefacts_current') IS NOT NULL
       AND to_regclass('current_file_state') IS NOT NULL THEN
        INSERT INTO symbol_features_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, normalized_name, normalized_signature, modifiers, identifier_tokens, normalized_body_tokens, parent_kind, context_tokens)
        SELECT a.artefact_id, a.repo_id, a.path, a.content_id, a.symbol_id, f.semantic_features_input_hash, f.normalized_name, f.normalized_signature, f.modifiers, f.identifier_tokens, f.normalized_body_tokens, f.parent_kind, f.context_tokens
        FROM artefacts_current a
        JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path AND cfs.effective_content_id = a.content_id
        JOIN symbol_features f
          ON f.repo_id = a.repo_id
         AND f.artefact_id = a.artefact_id
         AND f.blob_sha = a.content_id
        WHERE cfs.analysis_mode = 'code'
        ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, path = EXCLUDED.path, content_id = EXCLUDED.content_id, symbol_id = EXCLUDED.symbol_id, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, modifiers = EXCLUDED.modifiers, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, context_tokens = EXCLUDED.context_tokens, generated_at = now();

        INSERT INTO symbol_semantics_current (artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash, docstring_summary, llm_summary, template_summary, summary, confidence, source_model)
        SELECT a.artefact_id, a.repo_id, a.path, a.content_id, a.symbol_id, s.semantic_features_input_hash, s.docstring_summary, s.llm_summary, s.template_summary, s.summary, s.confidence, s.source_model
        FROM artefacts_current a
        JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path AND cfs.effective_content_id = a.content_id
        JOIN symbol_features f
          ON f.repo_id = a.repo_id
         AND f.artefact_id = a.artefact_id
         AND f.blob_sha = a.content_id
        JOIN symbol_semantics s
          ON s.repo_id = f.repo_id
         AND s.artefact_id = f.artefact_id
         AND s.blob_sha = f.blob_sha
         AND s.semantic_features_input_hash = f.semantic_features_input_hash
        WHERE cfs.analysis_mode = 'code'
        ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, path = EXCLUDED.path, content_id = EXCLUDED.content_id, symbol_id = EXCLUDED.symbol_id, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, docstring_summary = EXCLUDED.docstring_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, confidence = EXCLUDED.confidence, source_model = EXCLUDED.source_model, generated_at = now();
    END IF;
END $$;
"#
}

pub(crate) async fn upgrade_sqlite_semantic_features_schema(sqlite_path: &Path) -> Result<()> {
    let db_path = sqlite_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        crate::sqlite_vec_auto_extension::register_sqlite_vec_auto_extension()
            .context("registering sqlite-vec auto-extension for semantic feature schema upgrade")?;
        let conn = rusqlite::Connection::open(&db_path)
            .with_context(|| format!("opening SQLite database at {}", db_path.display()))?;

        if !sqlite_table_has_column(&conn, "symbol_semantics", "docstring_summary")? {
            conn.execute(
                "ALTER TABLE symbol_semantics ADD COLUMN docstring_summary TEXT",
                [],
            )
            .context("adding symbol_semantics.docstring_summary column")?;
        }

        if sqlite_table_has_column(&conn, "symbol_semantics", "doc_comment_summary")?
            && sqlite_table_has_column(&conn, "symbol_semantics", "docstring_summary")?
        {
            conn.execute(
                "UPDATE symbol_semantics \
SET docstring_summary = doc_comment_summary \
WHERE docstring_summary IS NULL AND doc_comment_summary IS NOT NULL",
                [],
            )
            .context("backfilling legacy symbol_semantics.doc_comment_summary values")?;
        }

        if !sqlite_table_has_column(&conn, "symbol_features", "modifiers")? {
            conn.execute(
                "ALTER TABLE symbol_features ADD COLUMN modifiers TEXT NOT NULL DEFAULT '[]'",
                [],
            )
            .context("adding symbol_features.modifiers column")?;
        }

        relax_sqlite_semantics_confidence_not_null(&conn, "symbol_semantics")?;
        relax_sqlite_semantics_confidence_not_null(&conn, "symbol_semantics_current")?;

        if sqlite_table_exists(&conn, "artefacts_current")?
            && sqlite_table_exists(&conn, "current_file_state")?
            && sqlite_table_has_column(&conn, "current_file_state", "effective_content_id")?
        {
            conn.execute_batch(
                &build_repair_all_current_semantic_projection_from_historical_sql(
                    RelationalDialect::Sqlite,
                ),
            )
            .context("repairing stranded current semantic projection rows")?;
        }

        Ok(())
    })
    .await
    .context("joining SQLite semantic feature upgrade task")?
}

fn relax_sqlite_semantics_confidence_not_null(
    conn: &rusqlite::Connection,
    table_name: &str,
) -> Result<()> {
    if !sqlite_table_exists(conn, table_name)?
        || !sqlite_column_is_not_null(conn, table_name, "confidence")?
    {
        return Ok(());
    }

    let sql = match table_name {
        "symbol_semantics" => {
            r#"
ALTER TABLE symbol_semantics RENAME TO symbol_semantics_old_confidence_not_null;
CREATE TABLE symbol_semantics (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL,
    source_model TEXT,
    generated_at TEXT DEFAULT (datetime('now'))
);
INSERT INTO symbol_semantics (
    artefact_id, repo_id, blob_sha, semantic_features_input_hash, docstring_summary,
    llm_summary, template_summary, summary, confidence, source_model, generated_at
)
SELECT
    artefact_id, repo_id, blob_sha, semantic_features_input_hash, docstring_summary,
    llm_summary, template_summary, summary, confidence, source_model, generated_at
FROM symbol_semantics_old_confidence_not_null;
DROP TABLE symbol_semantics_old_confidence_not_null;
CREATE INDEX IF NOT EXISTS symbol_semantics_repo_blob_idx
ON symbol_semantics (repo_id, blob_sha);
"#
        }
        "symbol_semantics_current" => {
            r#"
ALTER TABLE symbol_semantics_current RENAME TO symbol_semantics_current_old_confidence_not_null;
CREATE TABLE symbol_semantics_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL,
    source_model TEXT,
    generated_at TEXT DEFAULT (datetime('now'))
);
INSERT INTO symbol_semantics_current (
    artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
    docstring_summary, llm_summary, template_summary, summary, confidence, source_model, generated_at
)
SELECT
    artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
    docstring_summary, llm_summary, template_summary, summary, confidence, source_model, generated_at
FROM symbol_semantics_current_old_confidence_not_null;
DROP TABLE symbol_semantics_current_old_confidence_not_null;
CREATE INDEX IF NOT EXISTS symbol_semantics_current_repo_path_idx
ON symbol_semantics_current (repo_id, path);
CREATE UNIQUE INDEX IF NOT EXISTS symbol_semantics_current_repo_artefact_idx
ON symbol_semantics_current (repo_id, artefact_id);
"#
        }
        _ => return Ok(()),
    };

    conn.execute_batch(sql)
        .with_context(|| format!("relaxing {table_name}.confidence NOT NULL constraint"))
}

fn sqlite_table_exists(conn: &rusqlite::Connection, table_name: &str) -> Result<bool> {
    let exists = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM sqlite_master
                WHERE type = 'table' AND name = ?1
            )",
            [table_name],
            |row| row.get::<_, i64>(0),
        )
        .with_context(|| format!("checking whether SQLite table {table_name} exists"))?;
    Ok(exists != 0)
}

fn sqlite_table_has_column(
    conn: &rusqlite::Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .with_context(|| format!("preparing PRAGMA table_info({table_name})"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info({table_name})"))?;
    while let Some(row) = rows
        .next()
        .with_context(|| format!("iterating PRAGMA table_info({table_name})"))?
    {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from PRAGMA table_info({table_name})"))?;
        if name == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn sqlite_column_is_not_null(
    conn: &rusqlite::Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .with_context(|| format!("preparing PRAGMA table_info({table_name})"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info({table_name})"))?;
    while let Some(row) = rows
        .next()
        .with_context(|| format!("iterating PRAGMA table_info({table_name})"))?
    {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from PRAGMA table_info({table_name})"))?;
        if name == column_name {
            let not_null: i64 = row.get(3).with_context(|| {
                format!("reading NOT NULL flag from PRAGMA table_info({table_name})")
            })?;
            return Ok(not_null != 0);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::{
        semantic_features_postgres_shared_schema_sql,
        semantic_features_sqlite_current_projection_schema_sql,
        semantic_features_sqlite_shared_schema_sql,
    };

    #[test]
    fn semantic_feature_split_schemas_follow_current_vs_shared_boundary() {
        let postgres = semantic_features_postgres_shared_schema_sql();
        assert!(postgres.contains("CREATE TABLE IF NOT EXISTS symbol_semantics ("));
        assert!(postgres.contains("CREATE TABLE IF NOT EXISTS symbol_features ("));
        assert!(!postgres.contains("symbol_semantics_current"));
        assert!(!postgres.contains("symbol_features_current"));

        let sqlite_shared = semantic_features_sqlite_shared_schema_sql();
        assert!(sqlite_shared.contains("CREATE TABLE IF NOT EXISTS symbol_semantics ("));
        assert!(sqlite_shared.contains("CREATE TABLE IF NOT EXISTS symbol_features ("));
        assert!(!sqlite_shared.contains("symbol_semantics_current"));
        assert!(!sqlite_shared.contains("symbol_features_current"));

        let sqlite_current = semantic_features_sqlite_current_projection_schema_sql();
        assert!(sqlite_current.contains("CREATE TABLE IF NOT EXISTS symbol_semantics_current ("));
        assert!(sqlite_current.contains("CREATE TABLE IF NOT EXISTS symbol_features_current ("));
        assert!(!sqlite_current.contains("CREATE TABLE IF NOT EXISTS symbol_semantics ("));
        assert!(!sqlite_current.contains("CREATE TABLE IF NOT EXISTS symbol_features ("));
    }
}
