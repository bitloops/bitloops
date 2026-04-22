use super::types::{ColumnKind, ColumnSpec, TableSpec};

pub(super) const CACHE_REPOSITORIES_SPEC: TableSpec = TableSpec {
    name: "cache_repositories",
    columns: &[
        ColumnSpec {
            name: "repo_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "repo_root",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "provider",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "organization",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "name",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "identity",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "default_branch",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "metadata_json",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "created_at",
            kind: ColumnKind::Text,
        },
    ],
    key_columns: &["repo_id"],
};

pub(super) const CACHE_REPO_SYNC_STATE_SPEC: TableSpec = TableSpec {
    name: "cache_repo_sync_state",
    columns: &[
        ColumnSpec {
            name: "repo_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "repo_root",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "active_branch",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "head_commit_sha",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "head_tree_sha",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "parser_version",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "extractor_version",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "scope_exclusions_fingerprint",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "last_sync_started_at",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "last_sync_completed_at",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "last_sync_status",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "last_sync_reason",
            kind: ColumnKind::Text,
        },
    ],
    key_columns: &["repo_id"],
};

pub(super) const CACHE_CURRENT_FILE_STATE_SPEC: TableSpec = TableSpec {
    name: "cache_current_file_state",
    columns: &[
        ColumnSpec {
            name: "repo_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "path",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "analysis_mode",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "file_role",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "text_index_mode",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "language",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "resolved_language",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "dialect",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "primary_context_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "secondary_context_ids_json",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "frameworks_json",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "runtime_profile",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "classification_reason",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "context_fingerprint",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "extraction_fingerprint",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "head_content_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "index_content_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "worktree_content_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "effective_content_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "effective_source",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "parser_version",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "extractor_version",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "exists_in_head",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "exists_in_index",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "exists_in_worktree",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "last_synced_at",
            kind: ColumnKind::Text,
        },
    ],
    key_columns: &["repo_id", "path"],
};

pub(super) const CACHE_INTERACTION_SESSIONS_SPEC: TableSpec = TableSpec {
    name: "cache_interaction_sessions",
    columns: &[
        ColumnSpec {
            name: "session_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "repo_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "branch",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "actor_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "actor_name",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "actor_email",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "actor_source",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "agent_type",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "model",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "first_prompt",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "transcript_path",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "worktree_path",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "worktree_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "started_at",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "ended_at",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "last_event_at",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "updated_at",
            kind: ColumnKind::Text,
        },
    ],
    key_columns: &["repo_id", "session_id"],
};

pub(super) const CACHE_INTERACTION_TURNS_SPEC: TableSpec = TableSpec {
    name: "cache_interaction_turns",
    columns: &[
        ColumnSpec {
            name: "turn_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "session_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "repo_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "branch",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "actor_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "actor_name",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "actor_email",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "actor_source",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "turn_number",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "prompt",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "agent_type",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "model",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "started_at",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "ended_at",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "has_token_usage",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "input_tokens",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "cache_creation_tokens",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "cache_read_tokens",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "output_tokens",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "api_call_count",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "summary",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "prompt_count",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "transcript_offset_start",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "transcript_offset_end",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "transcript_fragment",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "files_modified",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "checkpoint_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "updated_at",
            kind: ColumnKind::Text,
        },
    ],
    key_columns: &["repo_id", "turn_id"],
};

pub(super) const CACHE_INTERACTION_EVENTS_SPEC: TableSpec = TableSpec {
    name: "cache_interaction_events",
    columns: &[
        ColumnSpec {
            name: "event_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "event_time",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "repo_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "session_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "turn_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "branch",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "actor_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "actor_name",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "actor_email",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "actor_source",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "event_type",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "source",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "sequence_number",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "agent_type",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "model",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "tool_use_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "tool_kind",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "task_description",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "subagent_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "payload",
            kind: ColumnKind::Text,
        },
    ],
    key_columns: &["repo_id", "event_id"],
};

pub(super) const CACHE_TOOL_INVOCATIONS_SPEC: TableSpec = TableSpec {
    name: "cache_interaction_tool_invocations",
    columns: &[
        ColumnSpec {
            name: "tool_invocation_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "repo_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "session_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "turn_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "tool_use_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "tool_name",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "source",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "input_summary",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "output_summary",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "command",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "command_binary",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "command_argv",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "transcript_path",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "started_at",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "ended_at",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "started_sequence_number",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "ended_sequence_number",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "updated_at",
            kind: ColumnKind::Text,
        },
    ],
    key_columns: &["repo_id", "tool_invocation_id"],
};

pub(super) const CACHE_SUBAGENT_RUNS_SPEC: TableSpec = TableSpec {
    name: "cache_interaction_subagent_runs",
    columns: &[
        ColumnSpec {
            name: "subagent_run_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "repo_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "session_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "turn_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "tool_use_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "subagent_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "subagent_type",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "task_description",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "source",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "transcript_path",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "child_session_id",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "started_at",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "ended_at",
            kind: ColumnKind::Text,
        },
        ColumnSpec {
            name: "started_sequence_number",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "ended_sequence_number",
            kind: ColumnKind::Integer,
        },
        ColumnSpec {
            name: "updated_at",
            kind: ColumnKind::Text,
        },
    ],
    key_columns: &["repo_id", "subagent_run_id"],
};
