use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::interactions::types::{
    InteractionEvent, InteractionSession, InteractionSubagentRun, InteractionToolInvocation,
    InteractionTurn,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionBrowseFilter {
    pub since: Option<String>,
    pub until: Option<String>,
    pub actor: Option<String>,
    pub actor_id: Option<String>,
    pub actor_email: Option<String>,
    pub commit_author: Option<String>,
    pub commit_author_email: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub branch: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub checkpoint_id: Option<String>,
    pub tool_use_id: Option<String>,
    pub tool_kind: Option<String>,
    pub has_checkpoint: Option<bool>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionSearchInput {
    pub filter: InteractionBrowseFilter,
    pub query: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionLinkedCheckpoint {
    pub checkpoint_id: String,
    pub commit_sha: String,
    pub author_name: String,
    pub author_email: String,
    pub committed_at: String,
}

#[derive(Debug, Clone)]
pub struct InteractionSessionSummary {
    pub session: InteractionSession,
    pub turn_count: usize,
    pub turn_ids: Vec<String>,
    pub checkpoint_count: usize,
    pub checkpoint_ids: Vec<String>,
    pub token_usage: Option<TokenUsageMetadata>,
    pub file_paths: Vec<String>,
    pub tool_uses: Vec<InteractionToolInvocation>,
    pub subagent_runs: Vec<InteractionSubagentRun>,
    pub linked_checkpoints: Vec<InteractionLinkedCheckpoint>,
    pub latest_commit_author: Option<InteractionLinkedCheckpoint>,
}

#[derive(Debug, Clone)]
pub struct InteractionTurnSummary {
    pub turn: InteractionTurn,
    pub tool_uses: Vec<InteractionToolInvocation>,
    pub subagent_runs: Vec<InteractionSubagentRun>,
    pub linked_checkpoints: Vec<InteractionLinkedCheckpoint>,
    pub latest_commit_author: Option<InteractionLinkedCheckpoint>,
}

#[derive(Debug, Clone)]
pub struct InteractionSessionDetail {
    pub summary: InteractionSessionSummary,
    pub turns: Vec<InteractionTurnSummary>,
    pub raw_events: Vec<InteractionEvent>,
}

#[derive(Debug, Clone)]
pub struct InteractionSessionSearchHit {
    pub session: InteractionSessionSummary,
    pub score: i64,
    pub matched_fields: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InteractionTurnSearchHit {
    pub turn: InteractionTurnSummary,
    pub session: InteractionSessionSummary,
    pub score: i64,
    pub matched_fields: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionKpis {
    pub total_sessions: usize,
    pub total_turns: usize,
    pub total_checkpoints: usize,
    pub total_tool_uses: usize,
    pub total_subagent_runs: usize,
    pub total_actors: usize,
    pub total_agents: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub api_call_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionChangeSnapshot {
    pub repo_id: String,
    pub session_count: usize,
    pub turn_count: usize,
    pub latest_session_id: Option<String>,
    pub latest_session_updated_at: Option<String>,
    pub latest_turn_id: Option<String>,
    pub latest_turn_updated_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionActorBucket {
    pub actor_id: String,
    pub actor_name: String,
    pub actor_email: String,
    pub actor_source: String,
    pub session_count: usize,
    pub turn_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionCommitAuthorBucket {
    pub author_name: String,
    pub author_email: String,
    pub checkpoint_count: usize,
    pub session_count: usize,
    pub turn_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionAgentBucket {
    pub key: String,
    pub session_count: usize,
    pub turn_count: usize,
}
