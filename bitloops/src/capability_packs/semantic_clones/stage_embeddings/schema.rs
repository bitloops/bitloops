use std::path::Path;

use anyhow::{Context, Result};

use crate::host::devql::{postgres_exec, sqlite_exec_path_allow_create};

pub(crate) fn semantic_embeddings_postgres_schema_sql() -> &'static str {
    r#"
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS symbol_embeddings (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding vector NOT NULL,
    generated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (artefact_id, representation_kind)
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_artefact_idx
ON symbol_embeddings (repo_id, artefact_id, representation_kind);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_model_idx
ON symbol_embeddings (repo_id, representation_kind, provider, model, dimension, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_embeddings_current (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding vector NOT NULL,
    generated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (artefact_id, representation_kind)
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_current_repo_path_idx
ON symbol_embeddings_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_embeddings_current_repo_artefact_idx
ON symbol_embeddings_current (repo_id, artefact_id, representation_kind);
CREATE TABLE IF NOT EXISTS semantic_clone_embedding_setup_state (
    repo_id TEXT PRIMARY KEY,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    setup_fingerprint TEXT NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT now()
);
"#
}

pub(crate) fn semantic_embeddings_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_embeddings (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding TEXT NOT NULL,
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (artefact_id, representation_kind)
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_artefact_idx
ON symbol_embeddings (repo_id, artefact_id, representation_kind);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_model_idx
ON symbol_embeddings (repo_id, representation_kind, provider, model, dimension, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_embeddings_current (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding TEXT NOT NULL,
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (artefact_id, representation_kind)
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_current_repo_path_idx
ON symbol_embeddings_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_embeddings_current_repo_artefact_idx
ON symbol_embeddings_current (repo_id, artefact_id, representation_kind);
CREATE TABLE IF NOT EXISTS semantic_clone_embedding_setup_state (
    repo_id TEXT PRIMARY KEY,
    representation_kind TEXT NOT NULL DEFAULT 'baseline',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    setup_fingerprint TEXT NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
"#
}

fn semantic_embeddings_postgres_upgrade_sql() -> &'static str {
    r#"
ALTER TABLE symbol_embeddings
    ADD COLUMN IF NOT EXISTS representation_kind TEXT NOT NULL DEFAULT 'baseline';
ALTER TABLE symbol_embeddings_current
    ADD COLUMN IF NOT EXISTS representation_kind TEXT NOT NULL DEFAULT 'baseline';
ALTER TABLE semantic_clone_embedding_setup_state
    ADD COLUMN IF NOT EXISTS representation_kind TEXT NOT NULL DEFAULT 'baseline';

DROP INDEX IF EXISTS symbol_embeddings_repo_artefact_idx;
CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_artefact_idx
ON symbol_embeddings (repo_id, artefact_id, representation_kind);

DROP INDEX IF EXISTS symbol_embeddings_repo_model_idx;
CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_model_idx
ON symbol_embeddings (repo_id, representation_kind, provider, model, dimension, blob_sha);

DROP INDEX IF EXISTS symbol_embeddings_current_repo_artefact_idx;
CREATE UNIQUE INDEX IF NOT EXISTS symbol_embeddings_current_repo_artefact_idx
ON symbol_embeddings_current (repo_id, artefact_id, representation_kind);

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'symbol_embeddings'::regclass
          AND conname = 'symbol_embeddings_pkey'
    ) THEN
        ALTER TABLE symbol_embeddings DROP CONSTRAINT symbol_embeddings_pkey;
    END IF;
EXCEPTION WHEN undefined_table THEN
    NULL;
END $$;

DO $$
BEGIN
    ALTER TABLE symbol_embeddings
        ADD CONSTRAINT symbol_embeddings_pkey PRIMARY KEY (artefact_id, representation_kind);
EXCEPTION WHEN duplicate_table THEN
    NULL;
WHEN duplicate_object THEN
    NULL;
END $$;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'symbol_embeddings_current'::regclass
          AND conname = 'symbol_embeddings_current_pkey'
    ) THEN
        ALTER TABLE symbol_embeddings_current DROP CONSTRAINT symbol_embeddings_current_pkey;
    END IF;
EXCEPTION WHEN undefined_table THEN
    NULL;
END $$;

DO $$
BEGIN
    ALTER TABLE symbol_embeddings_current
        ADD CONSTRAINT symbol_embeddings_current_pkey PRIMARY KEY (artefact_id, representation_kind);
EXCEPTION WHEN duplicate_table THEN
    NULL;
WHEN duplicate_object THEN
    NULL;
END $$;
"#
}

pub(crate) async fn init_sqlite_semantic_embeddings_schema(sqlite_path: &Path) -> Result<()> {
    upgrade_sqlite_semantic_embeddings_schema(sqlite_path).await?;
    sqlite_exec_path_allow_create(sqlite_path, semantic_embeddings_sqlite_schema_sql())
        .await
        .context("creating SQLite semantic embedding tables")?;
    Ok(())
}

pub(crate) async fn init_postgres_semantic_embeddings_schema(
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    postgres_exec(pg_client, semantic_embeddings_postgres_schema_sql())
        .await
        .context("creating Postgres semantic embedding tables")?;
    postgres_exec(pg_client, semantic_embeddings_postgres_upgrade_sql())
        .await
        .context("upgrading Postgres semantic embedding tables")?;
    Ok(())
}

async fn upgrade_sqlite_semantic_embeddings_schema(sqlite_path: &Path) -> Result<()> {
    let db_path = sqlite_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = rusqlite::Connection::open(&db_path)
            .with_context(|| format!("opening SQLite database at {}", db_path.display()))?;

        let symbol_embeddings_needs_upgrade =
            sqlite_table_has_column(&conn, "symbol_embeddings", "artefact_id")?
                && !sqlite_table_has_column(&conn, "symbol_embeddings", "representation_kind")?;
        let symbol_embeddings_has_generated_at =
            sqlite_table_has_column(&conn, "symbol_embeddings", "generated_at")?;
        let symbol_embeddings_current_needs_upgrade =
            sqlite_table_has_column(&conn, "symbol_embeddings_current", "artefact_id")?
                && !sqlite_table_has_column(
                    &conn,
                    "symbol_embeddings_current",
                    "representation_kind",
                )?;
        let symbol_embeddings_current_has_generated_at =
            sqlite_table_has_column(&conn, "symbol_embeddings_current", "generated_at")?;

        if symbol_embeddings_needs_upgrade {
            conn.execute(
                "ALTER TABLE symbol_embeddings RENAME TO symbol_embeddings_legacy",
                [],
            )
            .context("renaming legacy symbol_embeddings table")?;
        }

        if symbol_embeddings_current_needs_upgrade {
            conn.execute(
                "ALTER TABLE symbol_embeddings_current RENAME TO symbol_embeddings_current_legacy",
                [],
            )
            .context("renaming legacy symbol_embeddings_current table")?;
        }

        if symbol_embeddings_needs_upgrade || symbol_embeddings_current_needs_upgrade {
            conn.execute_batch(semantic_embeddings_sqlite_schema_sql())
                .context("creating upgraded semantic embedding tables")?;
        }

        if symbol_embeddings_needs_upgrade {
            let generated_at_select = if symbol_embeddings_has_generated_at {
                "generated_at"
            } else {
                "CURRENT_TIMESTAMP"
            };
            conn.execute(
                &format!(
                    "INSERT INTO symbol_embeddings (
                    artefact_id,
                    repo_id,
                    blob_sha,
                    representation_kind,
                    provider,
                    model,
                    dimension,
                    embedding_input_hash,
                    embedding,
                    generated_at
                )
                SELECT
                    artefact_id,
                    repo_id,
                    blob_sha,
                    'baseline',
                    provider,
                    model,
                    dimension,
                    embedding_input_hash,
                    embedding,
                    {generated_at_select}
                FROM symbol_embeddings_legacy"
                ),
                [],
            )
            .context("copying legacy symbol_embeddings rows into upgraded table")?;
            conn.execute("DROP TABLE symbol_embeddings_legacy", [])
                .context("dropping legacy symbol_embeddings table")?;
        }

        if symbol_embeddings_current_needs_upgrade {
            let generated_at_select = if symbol_embeddings_current_has_generated_at {
                "generated_at"
            } else {
                "CURRENT_TIMESTAMP"
            };
            conn.execute(
                &format!(
                    "INSERT INTO symbol_embeddings_current (
                    artefact_id,
                    repo_id,
                    path,
                    content_id,
                    symbol_id,
                    representation_kind,
                    provider,
                    model,
                    dimension,
                    embedding_input_hash,
                    embedding,
                    generated_at
                )
                SELECT
                    artefact_id,
                    repo_id,
                    path,
                    content_id,
                    symbol_id,
                    'baseline',
                    provider,
                    model,
                    dimension,
                    embedding_input_hash,
                    embedding,
                    {generated_at_select}
                FROM symbol_embeddings_current_legacy"
                ),
                [],
            )
            .context("copying legacy symbol_embeddings_current rows into upgraded table")?;
            conn.execute("DROP TABLE symbol_embeddings_current_legacy", [])
                .context("dropping legacy symbol_embeddings_current table")?;
        }

        if sqlite_table_has_column(&conn, "semantic_clone_embedding_setup_state", "repo_id")?
            && !sqlite_table_has_column(
                &conn,
                "semantic_clone_embedding_setup_state",
                "representation_kind",
            )?
        {
            conn.execute(
                "ALTER TABLE semantic_clone_embedding_setup_state
                 ADD COLUMN representation_kind TEXT NOT NULL DEFAULT 'baseline'",
                [],
            )
            .context("adding representation_kind to semantic_clone_embedding_setup_state")?;
        }

        Ok(())
    })
    .await
    .context("joining SQLite semantic embedding upgrade task")?
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
