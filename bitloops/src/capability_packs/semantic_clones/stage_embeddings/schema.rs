use std::path::Path;

use anyhow::{Context, Result};

use crate::host::devql::{postgres_exec, sqlite_exec_path_allow_create};

pub(crate) fn semantic_embeddings_postgres_schema_sql() -> &'static str {
    r#"
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS semantic_embedding_setups (
    setup_fingerprint TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE IF NOT EXISTS symbol_embeddings (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    representation_kind TEXT NOT NULL DEFAULT 'code',
    setup_fingerprint TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding vector NOT NULL,
    generated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (artefact_id, representation_kind, setup_fingerprint)
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_artefact_idx
ON symbol_embeddings (repo_id, artefact_id, representation_kind, setup_fingerprint);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_model_idx
ON symbol_embeddings (repo_id, representation_kind, setup_fingerprint, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_embeddings_current (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    representation_kind TEXT NOT NULL DEFAULT 'code',
    setup_fingerprint TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding vector NOT NULL,
    generated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (artefact_id, representation_kind, setup_fingerprint)
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_current_repo_path_idx
ON symbol_embeddings_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_embeddings_current_repo_artefact_idx
ON symbol_embeddings_current (repo_id, artefact_id, representation_kind, setup_fingerprint);

CREATE TABLE IF NOT EXISTS semantic_clone_embedding_setup_state (
    repo_id TEXT NOT NULL,
    representation_kind TEXT NOT NULL DEFAULT 'code',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    setup_fingerprint TEXT NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (repo_id, representation_kind)
);
"#
}

pub(crate) fn semantic_embeddings_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS semantic_embedding_setups (
    setup_fingerprint TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS symbol_embeddings (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    representation_kind TEXT NOT NULL DEFAULT 'code',
    setup_fingerprint TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding TEXT NOT NULL,
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (artefact_id, representation_kind, setup_fingerprint)
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_artefact_idx
ON symbol_embeddings (repo_id, artefact_id, representation_kind, setup_fingerprint);

CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_model_idx
ON symbol_embeddings (repo_id, representation_kind, setup_fingerprint, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_embeddings_current (
    artefact_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    representation_kind TEXT NOT NULL DEFAULT 'code',
    setup_fingerprint TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    embedding_input_hash TEXT NOT NULL,
    embedding TEXT NOT NULL,
    generated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (artefact_id, representation_kind, setup_fingerprint)
);

CREATE INDEX IF NOT EXISTS symbol_embeddings_current_repo_path_idx
ON symbol_embeddings_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_embeddings_current_repo_artefact_idx
ON symbol_embeddings_current (repo_id, artefact_id, representation_kind, setup_fingerprint);

CREATE TABLE IF NOT EXISTS semantic_clone_embedding_setup_state (
    repo_id TEXT NOT NULL,
    representation_kind TEXT NOT NULL DEFAULT 'code',
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    setup_fingerprint TEXT NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (repo_id, representation_kind)
);
"#
}

fn semantic_embeddings_postgres_upgrade_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS semantic_embedding_setups (
    setup_fingerprint TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    created_at TIMESTAMPTZ DEFAULT now()
);

ALTER TABLE symbol_embeddings
    ADD COLUMN IF NOT EXISTS representation_kind TEXT NOT NULL DEFAULT 'code';
ALTER TABLE symbol_embeddings
    ADD COLUMN IF NOT EXISTS setup_fingerprint TEXT;
ALTER TABLE symbol_embeddings_current
    ADD COLUMN IF NOT EXISTS representation_kind TEXT NOT NULL DEFAULT 'code';
ALTER TABLE symbol_embeddings_current
    ADD COLUMN IF NOT EXISTS setup_fingerprint TEXT;
ALTER TABLE semantic_clone_embedding_setup_state
    ADD COLUMN IF NOT EXISTS representation_kind TEXT NOT NULL DEFAULT 'code';
ALTER TABLE semantic_clone_embedding_setup_state
    ADD COLUMN IF NOT EXISTS setup_fingerprint TEXT;

UPDATE symbol_embeddings
SET setup_fingerprint = 'provider=' || provider || '|model=' || model || '|dimension=' || dimension::text
WHERE setup_fingerprint IS NULL OR btrim(setup_fingerprint) = '';

UPDATE symbol_embeddings_current
SET setup_fingerprint = 'provider=' || provider || '|model=' || model || '|dimension=' || dimension::text
WHERE setup_fingerprint IS NULL OR btrim(setup_fingerprint) = '';

UPDATE semantic_clone_embedding_setup_state
SET setup_fingerprint = 'provider=' || provider || '|model=' || model || '|dimension=' || dimension::text
WHERE setup_fingerprint IS NULL OR btrim(setup_fingerprint) = '';

INSERT INTO semantic_embedding_setups (setup_fingerprint, provider, model, dimension)
SELECT DISTINCT setup_fingerprint, provider, model, dimension
FROM symbol_embeddings
WHERE setup_fingerprint IS NOT NULL AND btrim(setup_fingerprint) <> ''
ON CONFLICT (setup_fingerprint) DO UPDATE
SET provider = EXCLUDED.provider, model = EXCLUDED.model, dimension = EXCLUDED.dimension;

INSERT INTO semantic_embedding_setups (setup_fingerprint, provider, model, dimension)
SELECT DISTINCT setup_fingerprint, provider, model, dimension
FROM symbol_embeddings_current
WHERE setup_fingerprint IS NOT NULL AND btrim(setup_fingerprint) <> ''
ON CONFLICT (setup_fingerprint) DO UPDATE
SET provider = EXCLUDED.provider, model = EXCLUDED.model, dimension = EXCLUDED.dimension;

INSERT INTO semantic_embedding_setups (setup_fingerprint, provider, model, dimension)
SELECT DISTINCT setup_fingerprint, provider, model, dimension
FROM semantic_clone_embedding_setup_state
WHERE setup_fingerprint IS NOT NULL AND btrim(setup_fingerprint) <> ''
ON CONFLICT (setup_fingerprint) DO UPDATE
SET provider = EXCLUDED.provider, model = EXCLUDED.model, dimension = EXCLUDED.dimension;

ALTER TABLE symbol_embeddings
    ALTER COLUMN setup_fingerprint SET NOT NULL;
ALTER TABLE symbol_embeddings_current
    ALTER COLUMN setup_fingerprint SET NOT NULL;
ALTER TABLE semantic_clone_embedding_setup_state
    ALTER COLUMN setup_fingerprint SET NOT NULL;

DROP INDEX IF EXISTS symbol_embeddings_repo_artefact_idx;
CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_artefact_idx
ON symbol_embeddings (repo_id, artefact_id, representation_kind, setup_fingerprint);

DROP INDEX IF EXISTS symbol_embeddings_repo_model_idx;
CREATE INDEX IF NOT EXISTS symbol_embeddings_repo_model_idx
ON symbol_embeddings (repo_id, representation_kind, setup_fingerprint, blob_sha);

DROP INDEX IF EXISTS symbol_embeddings_current_repo_artefact_idx;
CREATE UNIQUE INDEX IF NOT EXISTS symbol_embeddings_current_repo_artefact_idx
ON symbol_embeddings_current (repo_id, artefact_id, representation_kind, setup_fingerprint);

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
        ADD CONSTRAINT symbol_embeddings_pkey PRIMARY KEY (artefact_id, representation_kind, setup_fingerprint);
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
        ADD CONSTRAINT symbol_embeddings_current_pkey PRIMARY KEY (artefact_id, representation_kind, setup_fingerprint);
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
        WHERE conrelid = 'semantic_clone_embedding_setup_state'::regclass
          AND conname = 'semantic_clone_embedding_setup_state_pkey'
    ) THEN
        ALTER TABLE semantic_clone_embedding_setup_state DROP CONSTRAINT semantic_clone_embedding_setup_state_pkey;
    END IF;
EXCEPTION WHEN undefined_table THEN
    NULL;
END $$;

DO $$
BEGIN
    ALTER TABLE semantic_clone_embedding_setup_state
        ADD CONSTRAINT semantic_clone_embedding_setup_state_pkey PRIMARY KEY (repo_id, representation_kind);
EXCEPTION WHEN duplicate_table THEN
    NULL;
WHEN duplicate_object THEN
    NULL;
END $$;
"#
}

pub(crate) async fn init_sqlite_semantic_embeddings_schema(sqlite_path: &Path) -> Result<()> {
    sqlite_exec_path_allow_create(sqlite_path, semantic_embeddings_sqlite_schema_sql())
        .await
        .context("creating SQLite semantic embedding tables")?;
    upgrade_sqlite_semantic_embeddings_schema(sqlite_path).await?;
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

        let symbol_embeddings_needs_migration =
            sqlite_table_has_column(&conn, "symbol_embeddings", "artefact_id")?
                && (!sqlite_table_has_column(&conn, "symbol_embeddings", "setup_fingerprint")?
                    || sqlite_table_primary_key_columns(&conn, "symbol_embeddings")?
                        != vec![
                            "artefact_id".to_string(),
                            "representation_kind".to_string(),
                            "setup_fingerprint".to_string(),
                        ]);
        if symbol_embeddings_needs_migration {
            migrate_sqlite_symbol_embeddings_table(&conn)?;
        }

        let symbol_embeddings_current_needs_migration =
            sqlite_table_has_column(&conn, "symbol_embeddings_current", "artefact_id")?
                && (!sqlite_table_has_column(
                    &conn,
                    "symbol_embeddings_current",
                    "setup_fingerprint",
                )? || sqlite_table_primary_key_columns(&conn, "symbol_embeddings_current")?
                    != vec![
                        "artefact_id".to_string(),
                        "representation_kind".to_string(),
                        "setup_fingerprint".to_string(),
                    ]);
        if symbol_embeddings_current_needs_migration {
            migrate_sqlite_current_symbol_embeddings_table(&conn)?;
        }

        let active_state_needs_migration =
            sqlite_table_has_column(&conn, "semantic_clone_embedding_setup_state", "repo_id")?
                && (!sqlite_table_has_column(
                    &conn,
                    "semantic_clone_embedding_setup_state",
                    "setup_fingerprint",
                )? || sqlite_table_primary_key_columns(
                    &conn,
                    "semantic_clone_embedding_setup_state",
                )? != vec!["repo_id".to_string(), "representation_kind".to_string()]);
        if active_state_needs_migration {
            migrate_sqlite_active_embedding_setup_table(&conn)?;
        }

        conn.execute_batch(semantic_embeddings_sqlite_schema_sql())
            .context("ensuring upgraded SQLite semantic embedding tables")?;
        backfill_sqlite_embedding_setup_catalog(&conn)?;
        Ok(())
    })
    .await
    .context("joining SQLite semantic embedding upgrade task")?
}

fn migrate_sqlite_symbol_embeddings_table(conn: &rusqlite::Connection) -> Result<()> {
    let has_representation =
        sqlite_table_has_column(conn, "symbol_embeddings", "representation_kind")?;
    conn.execute(
        "ALTER TABLE symbol_embeddings RENAME TO symbol_embeddings_legacy",
        [],
    )
    .context("renaming legacy symbol_embeddings table")?;
    conn.execute_batch(semantic_embeddings_sqlite_schema_sql())
        .context("creating upgraded semantic embedding tables")?;
    let representation_expr = if has_representation {
        "COALESCE(representation_kind, 'baseline')"
    } else {
        "'baseline'"
    };
    conn.execute(
        &format!(
            "INSERT INTO symbol_embeddings (
                artefact_id,
                repo_id,
                blob_sha,
                representation_kind,
                setup_fingerprint,
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
                {representation_expr},
                'provider=' || provider || '|model=' || model || '|dimension=' || dimension,
                provider,
                model,
                dimension,
                embedding_input_hash,
                embedding,
                generated_at
            FROM symbol_embeddings_legacy"
        ),
        [],
    )
    .context("copying symbol_embeddings rows into upgraded table")?;
    conn.execute("DROP TABLE symbol_embeddings_legacy", [])
        .context("dropping legacy symbol_embeddings table")?;
    Ok(())
}

fn migrate_sqlite_current_symbol_embeddings_table(conn: &rusqlite::Connection) -> Result<()> {
    let has_representation =
        sqlite_table_has_column(conn, "symbol_embeddings_current", "representation_kind")?;
    conn.execute(
        "ALTER TABLE symbol_embeddings_current RENAME TO symbol_embeddings_current_legacy",
        [],
    )
    .context("renaming legacy symbol_embeddings_current table")?;
    conn.execute_batch(semantic_embeddings_sqlite_schema_sql())
        .context("creating upgraded current semantic embedding tables")?;
    let representation_expr = if has_representation {
        "COALESCE(representation_kind, 'baseline')"
    } else {
        "'baseline'"
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
                setup_fingerprint,
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
                {representation_expr},
                'provider=' || provider || '|model=' || model || '|dimension=' || dimension,
                provider,
                model,
                dimension,
                embedding_input_hash,
                embedding,
                generated_at
            FROM symbol_embeddings_current_legacy"
        ),
        [],
    )
    .context("copying symbol_embeddings_current rows into upgraded table")?;
    conn.execute("DROP TABLE symbol_embeddings_current_legacy", [])
        .context("dropping legacy symbol_embeddings_current table")?;
    Ok(())
}

fn migrate_sqlite_active_embedding_setup_table(conn: &rusqlite::Connection) -> Result<()> {
    let has_representation = sqlite_table_has_column(
        conn,
        "semantic_clone_embedding_setup_state",
        "representation_kind",
    )?;
    conn.execute(
        "ALTER TABLE semantic_clone_embedding_setup_state RENAME TO semantic_clone_embedding_setup_state_legacy",
        [],
    )
    .context("renaming legacy semantic_clone_embedding_setup_state table")?;
    conn.execute_batch(semantic_embeddings_sqlite_schema_sql())
        .context("creating upgraded semantic_clone_embedding_setup_state table")?;
    let representation_expr = if has_representation {
        "COALESCE(representation_kind, 'baseline')"
    } else {
        "'baseline'"
    };
    conn.execute(
        &format!(
            "INSERT INTO semantic_clone_embedding_setup_state (
                repo_id,
                representation_kind,
                provider,
                model,
                dimension,
                setup_fingerprint,
                updated_at
            )
            SELECT
                repo_id,
                {representation_expr},
                provider,
                model,
                dimension,
                'provider=' || provider || '|model=' || model || '|dimension=' || dimension,
                updated_at
            FROM semantic_clone_embedding_setup_state_legacy"
        ),
        [],
    )
    .context("copying semantic_clone_embedding_setup_state rows into upgraded table")?;
    conn.execute("DROP TABLE semantic_clone_embedding_setup_state_legacy", [])
        .context("dropping legacy semantic_clone_embedding_setup_state table")?;
    Ok(())
}

fn backfill_sqlite_embedding_setup_catalog(conn: &rusqlite::Connection) -> Result<()> {
    for table in [
        "symbol_embeddings",
        "symbol_embeddings_current",
        "semantic_clone_embedding_setup_state",
    ] {
        if !sqlite_table_has_column(conn, table, "setup_fingerprint")? {
            continue;
        }
        let sql = format!(
            "INSERT INTO semantic_embedding_setups (setup_fingerprint, provider, model, dimension)
            SELECT DISTINCT setup_fingerprint, provider, model, dimension
            FROM {table}
            WHERE setup_fingerprint IS NOT NULL AND TRIM(setup_fingerprint) <> ''
            ON CONFLICT(setup_fingerprint) DO UPDATE SET
                provider = excluded.provider,
                model = excluded.model,
                dimension = excluded.dimension"
        );
        conn.execute(&sql, [])
            .with_context(|| format!("backfilling semantic_embedding_setups from {table}"))?;
    }
    Ok(())
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

fn sqlite_table_primary_key_columns(
    conn: &rusqlite::Connection,
    table_name: &str,
) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .with_context(|| format!("preparing PRAGMA table_info({table_name})"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info({table_name})"))?;
    let mut columns = Vec::<(i64, String)>::new();
    while let Some(row) = rows
        .next()
        .with_context(|| format!("iterating PRAGMA table_info({table_name})"))?
    {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from PRAGMA table_info({table_name})"))?;
        let pk_position: i64 = row
            .get(5)
            .with_context(|| format!("reading pk position from PRAGMA table_info({table_name})"))?;
        if pk_position > 0 {
            columns.push((pk_position, name));
        }
    }
    columns.sort_by_key(|(position, _)| *position);
    Ok(columns.into_iter().map(|(_, name)| name).collect())
}

#[cfg(test)]
mod tests {
    use crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup;

    #[test]
    fn embedding_setup_fingerprint_format_matches_schema_backfill() {
        let setup = EmbeddingSetup::new("provider-x", "model-y", 42);
        assert_eq!(
            setup.setup_fingerprint,
            "provider=provider-x|model=model-y|dimension=42"
        );
    }
}
