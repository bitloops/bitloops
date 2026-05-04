pub fn context_guidance_initial_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS context_guidance_distillation_runs (
    run_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    capability_id TEXT NOT NULL DEFAULT 'context_guidance',
    capability_version TEXT DEFAULT '',
    source_scope_key TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    summary_json TEXT NOT NULL DEFAULT '{}',
    source_model TEXT DEFAULT '',
    source_profile TEXT DEFAULT '',
    status TEXT NOT NULL DEFAULT 'completed',
    generated_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS context_guidance_runs_scope_input_idx
ON context_guidance_distillation_runs (repo_id, source_scope_key, input_hash);

CREATE INDEX IF NOT EXISTS context_guidance_runs_scope_idx
ON context_guidance_distillation_runs (repo_id, source_scope_key, generated_at);

CREATE TABLE IF NOT EXISTS context_guidance_facts (
    guidance_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1,
    category TEXT NOT NULL,
    kind TEXT NOT NULL,
    guidance TEXT NOT NULL,
    evidence_excerpt TEXT NOT NULL,
    confidence TEXT NOT NULL,
    generated_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_facts_repo_category_idx
ON context_guidance_facts (repo_id, active, category, kind);

CREATE INDEX IF NOT EXISTS context_guidance_facts_run_idx
ON context_guidance_facts (run_id);

CREATE TABLE IF NOT EXISTS context_guidance_sources (
    source_row_id TEXT PRIMARY KEY,
    guidance_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    checkpoint_id TEXT,
    session_id TEXT,
    turn_id TEXT,
    tool_invocation_id TEXT,
    tool_kind TEXT,
    event_time TEXT,
    agent_type TEXT,
    model TEXT,
    evidence_kind TEXT,
    match_strength TEXT,
    knowledge_item_id TEXT,
    knowledge_item_version_id TEXT,
    relation_assertion_id TEXT,
    provider TEXT,
    source_kind TEXT,
    title TEXT,
    url TEXT,
    excerpt TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_sources_guidance_idx
ON context_guidance_sources (guidance_id);

CREATE INDEX IF NOT EXISTS context_guidance_sources_history_idx
ON context_guidance_sources (repo_id, checkpoint_id, session_id, turn_id);

CREATE INDEX IF NOT EXISTS context_guidance_sources_filter_idx
ON context_guidance_sources (repo_id, source_type, agent_type, event_time, evidence_kind);

CREATE INDEX IF NOT EXISTS context_guidance_sources_knowledge_idx
ON context_guidance_sources (repo_id, knowledge_item_id, knowledge_item_version_id, relation_assertion_id);

CREATE TABLE IF NOT EXISTS context_guidance_targets (
    target_row_id TEXT PRIMARY KEY,
    guidance_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_value TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_targets_lookup_idx
ON context_guidance_targets (repo_id, target_type, target_value);

CREATE INDEX IF NOT EXISTS context_guidance_targets_guidance_idx
ON context_guidance_targets (guidance_id);
"#
}

pub fn context_guidance_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS context_guidance_distillation_runs (
    run_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    capability_id TEXT NOT NULL DEFAULT 'context_guidance',
    capability_version TEXT DEFAULT '',
    source_scope_key TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    summary_json TEXT NOT NULL DEFAULT '{}',
    source_model TEXT DEFAULT '',
    source_profile TEXT DEFAULT '',
    status TEXT NOT NULL DEFAULT 'completed',
    generated_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS context_guidance_runs_scope_input_idx
ON context_guidance_distillation_runs (repo_id, source_scope_key, input_hash);

CREATE INDEX IF NOT EXISTS context_guidance_runs_scope_idx
ON context_guidance_distillation_runs (repo_id, source_scope_key, generated_at);

CREATE TABLE IF NOT EXISTS context_guidance_facts (
    guidance_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1,
    category TEXT NOT NULL,
    kind TEXT NOT NULL,
    guidance TEXT NOT NULL,
    evidence_excerpt TEXT NOT NULL,
    confidence TEXT NOT NULL,
    lifecycle_status TEXT NOT NULL DEFAULT 'active',
    fact_fingerprint TEXT NOT NULL DEFAULT '',
    value_score REAL NOT NULL DEFAULT 0.0,
    superseded_by_guidance_id TEXT,
    lifecycle_reason TEXT NOT NULL DEFAULT '',
    generated_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_facts_repo_category_idx
ON context_guidance_facts (repo_id, active, category, kind);

CREATE INDEX IF NOT EXISTS context_guidance_facts_run_idx
ON context_guidance_facts (run_id);

CREATE INDEX IF NOT EXISTS context_guidance_facts_lifecycle_idx
ON context_guidance_facts (repo_id, active, lifecycle_status, value_score);

CREATE INDEX IF NOT EXISTS context_guidance_facts_fingerprint_idx
ON context_guidance_facts (repo_id, fact_fingerprint, lifecycle_status);

CREATE TABLE IF NOT EXISTS context_guidance_sources (
    source_row_id TEXT PRIMARY KEY,
    guidance_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    checkpoint_id TEXT,
    session_id TEXT,
    turn_id TEXT,
    tool_invocation_id TEXT,
    tool_kind TEXT,
    event_time TEXT,
    agent_type TEXT,
    model TEXT,
    evidence_kind TEXT,
    match_strength TEXT,
    knowledge_item_id TEXT,
    knowledge_item_version_id TEXT,
    relation_assertion_id TEXT,
    provider TEXT,
    source_kind TEXT,
    title TEXT,
    url TEXT,
    excerpt TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_sources_guidance_idx
ON context_guidance_sources (guidance_id);

CREATE INDEX IF NOT EXISTS context_guidance_sources_history_idx
ON context_guidance_sources (repo_id, checkpoint_id, session_id, turn_id);

CREATE INDEX IF NOT EXISTS context_guidance_sources_filter_idx
ON context_guidance_sources (repo_id, source_type, agent_type, event_time, evidence_kind);

CREATE INDEX IF NOT EXISTS context_guidance_sources_knowledge_idx
ON context_guidance_sources (repo_id, knowledge_item_id, knowledge_item_version_id, relation_assertion_id);

CREATE TABLE IF NOT EXISTS context_guidance_targets (
    target_row_id TEXT PRIMARY KEY,
    guidance_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_value TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_targets_lookup_idx
ON context_guidance_targets (repo_id, target_type, target_value);

CREATE INDEX IF NOT EXISTS context_guidance_targets_guidance_idx
ON context_guidance_targets (guidance_id);

CREATE TABLE IF NOT EXISTS context_guidance_compaction_runs (
    compaction_run_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_value TEXT NOT NULL,
    source_fact_count INTEGER NOT NULL DEFAULT 0,
    retained_fact_count INTEGER NOT NULL DEFAULT 0,
    compacted_fact_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'completed',
    summary_json TEXT NOT NULL DEFAULT '{}',
    source_model TEXT DEFAULT '',
    source_profile TEXT DEFAULT '',
    generated_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_compaction_runs_target_idx
ON context_guidance_compaction_runs (repo_id, target_type, target_value, generated_at);

CREATE TABLE IF NOT EXISTS context_guidance_compaction_members (
    compaction_member_id TEXT PRIMARY KEY,
    compaction_run_id TEXT NOT NULL,
    guidance_id TEXT NOT NULL,
    action TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_compaction_members_run_idx
ON context_guidance_compaction_members (compaction_run_id);

CREATE INDEX IF NOT EXISTS context_guidance_compaction_members_guidance_idx
ON context_guidance_compaction_members (guidance_id);

CREATE TABLE IF NOT EXISTS context_guidance_target_summaries (
    target_summary_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_value TEXT NOT NULL,
    summary_json TEXT NOT NULL DEFAULT '{}',
    active_guidance_count INTEGER NOT NULL DEFAULT 0,
    latest_compaction_run_id TEXT,
    generated_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS context_guidance_target_summaries_target_idx
ON context_guidance_target_summaries (repo_id, target_type, target_value);
"#
}
