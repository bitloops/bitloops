// Database schema initialisation: ClickHouse and Postgres DDL.

async fn init_clickhouse_schema(cfg: &DevqlConfig) -> Result<()> {
    let sql = r#"
CREATE TABLE IF NOT EXISTS checkpoint_events (
    event_id String,
    event_time DateTime64(3, 'UTC'),
    repo_id String,
    checkpoint_id String,
    session_id String,
    commit_sha String,
    branch String,
    event_type String,
    agent String,
    strategy String,
    files_touched Array(String),
    payload String
)
ENGINE = ReplacingMergeTree(event_time)
ORDER BY (repo_id, event_time, event_id)
"#;

    clickhouse_exec(cfg, sql)
        .await
        .context("creating ClickHouse checkpoint_events table")?;
    Ok(())
}

async fn init_postgres_schema(
    _cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    let sql = postgres_schema_sql();
    postgres_exec(pg_client, sql)
        .await
        .context("creating Postgres DevQL tables")?;

    let artefacts_alter_sql = artefacts_upgrade_sql();
    postgres_exec(pg_client, artefacts_alter_sql)
        .await
        .context("updating Postgres artefacts columns for byte offsets/signature")?;

    let artefact_edges_hardening_sql = artefact_edges_hardening_sql();
    postgres_exec(pg_client, artefact_edges_hardening_sql)
        .await
        .context("updating Postgres artefact_edges constraints/indexes")?;
    Ok(())
}

fn postgres_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS repositories (
    repo_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    organization TEXT NOT NULL,
    name TEXT NOT NULL,
    default_branch TEXT,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE IF NOT EXISTS commits (
    commit_sha TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    author_name TEXT,
    author_email TEXT,
    commit_message TEXT,
    committed_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS file_state (
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    PRIMARY KEY (repo_id, commit_sha, path)
);

CREATE INDEX IF NOT EXISTS file_state_blob_idx
ON file_state (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS file_state_commit_idx
ON file_state (repo_id, commit_sha);

CREATE TABLE IF NOT EXISTS artefacts (
    artefact_id TEXT PRIMARY KEY,
    symbol_id TEXT,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT,
    content_hash TEXT,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS artefacts_blob_idx
ON artefacts (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS artefacts_path_idx
ON artefacts (repo_id, path);

CREATE INDEX IF NOT EXISTS artefacts_kind_idx
ON artefacts (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS artefacts_symbol_idx
ON artefacts (repo_id, symbol_id)
WHERE symbol_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS artefact_edges (
    edge_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    from_artefact_id TEXT NOT NULL,
    to_artefact_id TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata JSONB DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ DEFAULT now(),
    CONSTRAINT artefact_edges_target_chk
        CHECK (to_artefact_id IS NOT NULL OR to_symbol_ref IS NOT NULL),
    CONSTRAINT artefact_edges_line_range_chk
        CHECK (
            (start_line IS NULL AND end_line IS NULL)
            OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
        )
);

CREATE INDEX IF NOT EXISTS artefact_edges_blob_idx
ON artefact_edges (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS artefact_edges_from_idx
ON artefact_edges (repo_id, from_artefact_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_to_idx
ON artefact_edges (repo_id, to_artefact_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_kind_idx
ON artefact_edges (repo_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_symbol_ref_idx
ON artefact_edges (repo_id, edge_kind, to_symbol_ref)
WHERE to_symbol_ref IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_natural_uq
ON artefact_edges (
    repo_id,
    blob_sha,
    from_artefact_id,
    edge_kind,
    COALESCE(to_artefact_id, ''),
    COALESCE(to_symbol_ref, ''),
    COALESCE(start_line, -1),
    COALESCE(end_line, -1)
);
"#
}

fn artefacts_upgrade_sql() -> &'static str {
    r#"
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS start_byte INTEGER;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS end_byte INTEGER;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS signature TEXT;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS symbol_id TEXT;
ALTER TABLE artefacts ALTER COLUMN canonical_kind DROP NOT NULL;
UPDATE artefacts
SET start_byte = 0
WHERE start_byte IS NULL;
UPDATE artefacts
SET end_byte = 0
WHERE end_byte IS NULL;
ALTER TABLE artefacts ALTER COLUMN start_byte SET NOT NULL;
ALTER TABLE artefacts ALTER COLUMN end_byte SET NOT NULL;

CREATE INDEX IF NOT EXISTS artefacts_symbol_idx
ON artefacts (repo_id, symbol_id)
WHERE symbol_id IS NOT NULL;
"#
}

fn artefact_edges_hardening_sql() -> &'static str {
    r#"
ALTER TABLE artefact_edges ADD COLUMN IF NOT EXISTS metadata JSONB DEFAULT '{}'::jsonb;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'artefact_edges_target_chk'
    ) THEN
        ALTER TABLE artefact_edges
        ADD CONSTRAINT artefact_edges_target_chk
        CHECK (to_artefact_id IS NOT NULL OR to_symbol_ref IS NOT NULL);
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'artefact_edges_line_range_chk'
    ) THEN
        ALTER TABLE artefact_edges
        ADD CONSTRAINT artefact_edges_line_range_chk
        CHECK (
            (start_line IS NULL AND end_line IS NULL)
            OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
        );
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS artefact_edges_blob_idx
ON artefact_edges (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS artefact_edges_from_idx
ON artefact_edges (repo_id, from_artefact_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_to_idx
ON artefact_edges (repo_id, to_artefact_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_kind_idx
ON artefact_edges (repo_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_symbol_ref_idx
ON artefact_edges (repo_id, edge_kind, to_symbol_ref)
WHERE to_symbol_ref IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_natural_uq
ON artefact_edges (
    repo_id,
    blob_sha,
    from_artefact_id,
    edge_kind,
    COALESCE(to_artefact_id, ''),
    COALESCE(to_symbol_ref, ''),
    COALESCE(start_line, -1),
    COALESCE(end_line, -1)
);
"#
}
