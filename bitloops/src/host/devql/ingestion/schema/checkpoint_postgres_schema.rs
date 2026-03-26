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
    ended_at TEXT,
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
