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

    let current_state_hardening_sql = current_state_hardening_sql();
    postgres_exec(pg_client, current_state_hardening_sql)
        .await
        .context("updating Postgres current-state DevQL tables")?;

    let checkpoint_schema_sql = checkpoint_schema_sql_postgres();
    postgres_exec(pg_client, checkpoint_schema_sql)
        .await
        .context("creating Postgres checkpoint migration tables")?;
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

CREATE TABLE IF NOT EXISTS current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    committed_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (repo_id, path)
);

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
    modifiers JSONB NOT NULL DEFAULT '[]'::jsonb,
    docstring TEXT,
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

CREATE TABLE IF NOT EXISTS artefacts_current (
    repo_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    parent_symbol_id TEXT,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT,
    modifiers JSONB NOT NULL DEFAULT '[]'::jsonb,
    docstring TEXT,
    content_hash TEXT,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (repo_id, symbol_id)
);

CREATE INDEX IF NOT EXISTS artefacts_current_path_idx
ON artefacts_current (repo_id, path);

CREATE INDEX IF NOT EXISTS artefacts_current_kind_idx
ON artefacts_current (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS artefacts_current_artefact_idx
ON artefacts_current (repo_id, artefact_id);

CREATE INDEX IF NOT EXISTS artefacts_current_symbol_fqn_idx
ON artefacts_current (repo_id, symbol_fqn);

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

CREATE TABLE IF NOT EXISTS artefact_edges_current (
    edge_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    from_symbol_id TEXT NOT NULL,
    from_artefact_id TEXT NOT NULL,
    to_symbol_id TEXT,
    to_artefact_id TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata JSONB DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ DEFAULT now(),
    CONSTRAINT artefact_edges_current_target_chk
        CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL),
    CONSTRAINT artefact_edges_current_line_range_chk
        CHECK (
            (start_line IS NULL AND end_line IS NULL)
            OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
        )
);

CREATE INDEX IF NOT EXISTS artefact_edges_current_path_idx
ON artefact_edges_current (repo_id, path);

CREATE INDEX IF NOT EXISTS artefact_edges_current_from_idx
ON artefact_edges_current (repo_id, from_symbol_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_to_idx
ON artefact_edges_current (repo_id, to_symbol_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_kind_idx
ON artefact_edges_current (repo_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_symbol_ref_idx
ON artefact_edges_current (repo_id, to_symbol_ref)
WHERE to_symbol_ref IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_current_natural_uq
ON artefact_edges_current (
    repo_id,
    from_symbol_id,
    edge_kind,
    COALESCE(to_symbol_id, ''),
    COALESCE(to_symbol_ref, ''),
    COALESCE(start_line, -1),
    COALESCE(end_line, -1),
    md5(metadata::text)
);
"#
}

pub(crate) fn checkpoint_schema_sql_postgres() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    cli_version TEXT DEFAULT '',
    base_commit TEXT DEFAULT '',
    attribution_base_commit TEXT DEFAULT '',
    worktree_path TEXT DEFAULT '',
    worktree_id TEXT DEFAULT '',
    started_at TEXT,
    phase TEXT DEFAULT 'active',
    turn_id TEXT DEFAULT '',
    step_count INTEGER DEFAULT 0,
    checkpoint_transcript_start INTEGER DEFAULT 0,
    transcript_path TEXT DEFAULT '',
    first_prompt TEXT DEFAULT '',
    agent_type TEXT DEFAULT '',
    last_checkpoint_id TEXT DEFAULT '',
    last_interaction_time TEXT,
    files_touched TEXT DEFAULT '[]',
    untracked_files_at_start TEXT DEFAULT '[]',
    turn_checkpoint_ids TEXT DEFAULT '[]',
    transcript_identifier_at_start TEXT DEFAULT '',
    token_usage TEXT,
    prompt_attributions TEXT DEFAULT '[]',
    pending_prompt_attribution TEXT,
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS sessions_repo_idx
ON sessions (repo_id);

CREATE TABLE IF NOT EXISTS temporary_checkpoints (
    id BIGSERIAL PRIMARY KEY,
    session_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    tree_hash TEXT NOT NULL,
    step_number INTEGER NOT NULL,
    modified_files TEXT DEFAULT '[]',
    new_files TEXT DEFAULT '[]',
    deleted_files TEXT DEFAULT '[]',
    author_name TEXT DEFAULT '',
    author_email TEXT DEFAULT '',
    tool_use_id TEXT,
    agent_id TEXT,
    is_incremental INTEGER DEFAULT 0,
    incremental_sequence INTEGER,
    incremental_type TEXT,
    incremental_data TEXT,
    commit_message TEXT DEFAULT '',
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS temporary_checkpoints_session_step_idx
ON temporary_checkpoints (session_id, step_number);

CREATE INDEX IF NOT EXISTS temporary_checkpoints_session_tree_idx
ON temporary_checkpoints (session_id, tree_hash);

CREATE TABLE IF NOT EXISTS checkpoints (
    checkpoint_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    strategy TEXT DEFAULT 'manual-commit',
    branch TEXT DEFAULT '',
    cli_version TEXT DEFAULT '',
    files_touched TEXT DEFAULT '[]',
    checkpoints_count INTEGER DEFAULT 0,
    token_usage TEXT,
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS checkpoints_repo_idx
ON checkpoints (repo_id, created_at);

CREATE TABLE IF NOT EXISTS checkpoint_sessions (
    checkpoint_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    session_index INTEGER NOT NULL DEFAULT 0,
    agent TEXT DEFAULT '',
    turn_id TEXT DEFAULT '',
    checkpoints_count INTEGER DEFAULT 0,
    files_touched TEXT DEFAULT '[]',
    is_task INTEGER DEFAULT 0,
    tool_use_id TEXT DEFAULT '',
    transcript_identifier_at_start TEXT DEFAULT '',
    checkpoint_transcript_start INTEGER DEFAULT 0,
    initial_attribution TEXT,
    token_usage TEXT,
    summary TEXT,
    author_name TEXT DEFAULT '',
    author_email TEXT DEFAULT '',
    transcript_path TEXT DEFAULT '',
    subagent_transcript_path TEXT DEFAULT '',
    content_hash TEXT DEFAULT '',
    created_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (checkpoint_id, session_index)
);

CREATE INDEX IF NOT EXISTS checkpoint_sessions_session_idx
ON checkpoint_sessions (session_id, checkpoint_id);

CREATE TABLE IF NOT EXISTS commit_checkpoints (
    commit_sha TEXT NOT NULL,
    checkpoint_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (commit_sha, checkpoint_id)
);

CREATE INDEX IF NOT EXISTS commit_checkpoints_repo_commit_idx
ON commit_checkpoints (repo_id, commit_sha);

CREATE TABLE IF NOT EXISTS pre_prompt_states (
    session_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS pre_prompt_states_repo_idx
ON pre_prompt_states (repo_id);

CREATE TABLE IF NOT EXISTS pre_task_markers (
    tool_use_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS pre_task_markers_session_idx
ON pre_task_markers (session_id);

CREATE TABLE IF NOT EXISTS checkpoint_blobs (
    blob_id TEXT PRIMARY KEY,
    checkpoint_id TEXT NOT NULL,
    session_index INTEGER NOT NULL,
    blob_type TEXT NOT NULL,
    storage_backend TEXT NOT NULL,
    storage_path TEXT NOT NULL,
    content_hash TEXT DEFAULT '',
    size_bytes BIGINT DEFAULT 0,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS checkpoint_blobs_lookup_idx
ON checkpoint_blobs (checkpoint_id, session_index, blob_type);
"#
}

fn artefacts_upgrade_sql() -> &'static str {
    r#"
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS start_byte INTEGER;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS end_byte INTEGER;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS signature TEXT;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS symbol_id TEXT;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS modifiers JSONB DEFAULT '[]'::jsonb;
ALTER TABLE artefacts ADD COLUMN IF NOT EXISTS docstring TEXT;
ALTER TABLE artefacts ALTER COLUMN canonical_kind DROP NOT NULL;
UPDATE artefacts
SET start_byte = 0
WHERE start_byte IS NULL;
UPDATE artefacts
SET end_byte = 0
WHERE end_byte IS NULL;
UPDATE artefacts
SET modifiers = '[]'::jsonb
WHERE modifiers IS NULL;
ALTER TABLE artefacts ALTER COLUMN start_byte SET NOT NULL;
ALTER TABLE artefacts ALTER COLUMN end_byte SET NOT NULL;
ALTER TABLE artefacts ALTER COLUMN modifiers SET NOT NULL;

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

fn current_state_hardening_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    committed_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (repo_id, path)
);

ALTER TABLE current_file_state ADD COLUMN IF NOT EXISTS commit_sha TEXT;
ALTER TABLE current_file_state ADD COLUMN IF NOT EXISTS blob_sha TEXT;
ALTER TABLE current_file_state ADD COLUMN IF NOT EXISTS committed_at TIMESTAMPTZ;
ALTER TABLE current_file_state ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ DEFAULT now();

CREATE TABLE IF NOT EXISTS artefacts_current (
    repo_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    parent_symbol_id TEXT,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT,
    content_hash TEXT,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (repo_id, symbol_id)
);

ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS artefact_id TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS commit_sha TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS blob_sha TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS path TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS language TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS canonical_kind TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS language_kind TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS symbol_fqn TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS parent_symbol_id TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS parent_artefact_id TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS start_line INTEGER;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS end_line INTEGER;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS start_byte INTEGER;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS end_byte INTEGER;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS signature TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS modifiers JSONB DEFAULT '[]'::jsonb;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS docstring TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS content_hash TEXT;
ALTER TABLE artefacts_current ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ DEFAULT now();
ALTER TABLE artefacts_current ALTER COLUMN canonical_kind DROP NOT NULL;
UPDATE artefacts_current
SET modifiers = '[]'::jsonb
WHERE modifiers IS NULL;
ALTER TABLE artefacts_current ALTER COLUMN modifiers SET NOT NULL;

CREATE INDEX IF NOT EXISTS artefacts_current_path_idx
ON artefacts_current (repo_id, path);

CREATE INDEX IF NOT EXISTS artefacts_current_kind_idx
ON artefacts_current (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS artefacts_current_artefact_idx
ON artefacts_current (repo_id, artefact_id);

CREATE INDEX IF NOT EXISTS artefacts_current_symbol_fqn_idx
ON artefacts_current (repo_id, symbol_fqn);

CREATE TABLE IF NOT EXISTS artefact_edges_current (
    edge_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    from_symbol_id TEXT NOT NULL,
    from_artefact_id TEXT NOT NULL,
    to_symbol_id TEXT,
    to_artefact_id TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata JSONB DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ DEFAULT now()
);

ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS commit_sha TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS blob_sha TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS path TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS from_symbol_id TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS from_artefact_id TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS to_symbol_id TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS to_artefact_id TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS to_symbol_ref TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS edge_kind TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS language TEXT;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS start_line INTEGER;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS end_line INTEGER;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS metadata JSONB DEFAULT '{}'::jsonb;
ALTER TABLE artefact_edges_current ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ DEFAULT now();

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'artefact_edges_current_target_chk'
    ) THEN
        ALTER TABLE artefact_edges_current
        ADD CONSTRAINT artefact_edges_current_target_chk
        CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL);
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'artefact_edges_current_line_range_chk'
    ) THEN
        ALTER TABLE artefact_edges_current
        ADD CONSTRAINT artefact_edges_current_line_range_chk
        CHECK (
            (start_line IS NULL AND end_line IS NULL)
            OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
        );
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS artefact_edges_current_path_idx
ON artefact_edges_current (repo_id, path);

CREATE INDEX IF NOT EXISTS artefact_edges_current_from_idx
ON artefact_edges_current (repo_id, from_symbol_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_to_idx
ON artefact_edges_current (repo_id, to_symbol_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_kind_idx
ON artefact_edges_current (repo_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_symbol_ref_idx
ON artefact_edges_current (repo_id, to_symbol_ref)
WHERE to_symbol_ref IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_current_natural_uq
ON artefact_edges_current (
    repo_id,
    from_symbol_id,
    edge_kind,
    COALESCE(to_symbol_id, ''),
    COALESCE(to_symbol_ref, ''),
    COALESCE(start_line, -1),
    COALESCE(end_line, -1),
    md5(metadata::text)
);
"#
}

pub(crate) fn checkpoint_schema_sql_sqlite() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    cli_version TEXT DEFAULT '',
    base_commit TEXT DEFAULT '',
    attribution_base_commit TEXT DEFAULT '',
    worktree_path TEXT DEFAULT '',
    worktree_id TEXT DEFAULT '',
    started_at TEXT,
    phase TEXT DEFAULT 'active',
    turn_id TEXT DEFAULT '',
    step_count INTEGER DEFAULT 0,
    checkpoint_transcript_start INTEGER DEFAULT 0,
    transcript_path TEXT DEFAULT '',
    first_prompt TEXT DEFAULT '',
    agent_type TEXT DEFAULT '',
    last_checkpoint_id TEXT DEFAULT '',
    last_interaction_time TEXT,
    files_touched TEXT DEFAULT '[]',
    untracked_files_at_start TEXT DEFAULT '[]',
    turn_checkpoint_ids TEXT DEFAULT '[]',
    transcript_identifier_at_start TEXT DEFAULT '',
    token_usage TEXT,
    prompt_attributions TEXT DEFAULT '[]',
    pending_prompt_attribution TEXT,
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS sessions_repo_idx
ON sessions (repo_id);

CREATE TABLE IF NOT EXISTS temporary_checkpoints (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    tree_hash TEXT NOT NULL,
    step_number INTEGER NOT NULL,
    modified_files TEXT DEFAULT '[]',
    new_files TEXT DEFAULT '[]',
    deleted_files TEXT DEFAULT '[]',
    author_name TEXT DEFAULT '',
    author_email TEXT DEFAULT '',
    tool_use_id TEXT,
    agent_id TEXT,
    is_incremental INTEGER DEFAULT 0,
    incremental_sequence INTEGER,
    incremental_type TEXT,
    incremental_data TEXT,
    commit_message TEXT DEFAULT '',
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS temporary_checkpoints_session_step_idx
ON temporary_checkpoints (session_id, step_number);

CREATE INDEX IF NOT EXISTS temporary_checkpoints_session_tree_idx
ON temporary_checkpoints (session_id, tree_hash);

CREATE TABLE IF NOT EXISTS checkpoints (
    checkpoint_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    strategy TEXT DEFAULT 'manual-commit',
    branch TEXT DEFAULT '',
    cli_version TEXT DEFAULT '',
    files_touched TEXT DEFAULT '[]',
    checkpoints_count INTEGER DEFAULT 0,
    token_usage TEXT,
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS checkpoints_repo_idx
ON checkpoints (repo_id, created_at);

CREATE TABLE IF NOT EXISTS checkpoint_sessions (
    checkpoint_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    session_index INTEGER NOT NULL DEFAULT 0,
    agent TEXT DEFAULT '',
    turn_id TEXT DEFAULT '',
    checkpoints_count INTEGER DEFAULT 0,
    files_touched TEXT DEFAULT '[]',
    is_task INTEGER DEFAULT 0,
    tool_use_id TEXT DEFAULT '',
    transcript_identifier_at_start TEXT DEFAULT '',
    checkpoint_transcript_start INTEGER DEFAULT 0,
    initial_attribution TEXT,
    token_usage TEXT,
    summary TEXT,
    author_name TEXT DEFAULT '',
    author_email TEXT DEFAULT '',
    transcript_path TEXT DEFAULT '',
    subagent_transcript_path TEXT DEFAULT '',
    content_hash TEXT DEFAULT '',
    created_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (checkpoint_id, session_index)
);

CREATE INDEX IF NOT EXISTS checkpoint_sessions_session_idx
ON checkpoint_sessions (session_id, checkpoint_id);

CREATE TABLE IF NOT EXISTS commit_checkpoints (
    commit_sha TEXT NOT NULL,
    checkpoint_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (commit_sha, checkpoint_id)
);

CREATE INDEX IF NOT EXISTS commit_checkpoints_repo_commit_idx
ON commit_checkpoints (repo_id, commit_sha);

CREATE TABLE IF NOT EXISTS pre_prompt_states (
    session_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS pre_prompt_states_repo_idx
ON pre_prompt_states (repo_id);

CREATE TABLE IF NOT EXISTS pre_task_markers (
    tool_use_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS pre_task_markers_session_idx
ON pre_task_markers (session_id);

CREATE TABLE IF NOT EXISTS checkpoint_blobs (
    blob_id TEXT PRIMARY KEY,
    checkpoint_id TEXT NOT NULL,
    session_index INTEGER NOT NULL,
    blob_type TEXT NOT NULL,
    storage_backend TEXT NOT NULL,
    storage_path TEXT NOT NULL,
    content_hash TEXT DEFAULT '',
    size_bytes INTEGER DEFAULT 0,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS checkpoint_blobs_lookup_idx
ON checkpoint_blobs (checkpoint_id, session_index, blob_type);
"#
}
