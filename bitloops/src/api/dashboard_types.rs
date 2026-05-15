use async_graphql::{Enum, InputObject, SimpleObject};

use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::interactions::query::{
    InteractionActorBucket, InteractionAgentBucket, InteractionChangeSnapshot,
    InteractionCommitAuthorBucket, InteractionKpis, InteractionLinkedCheckpoint,
    InteractionSessionDetail, InteractionSessionSearchHit, InteractionSessionSummary,
    InteractionTurnSearchHit, InteractionTurnSummary,
};
use crate::host::interactions::transcript_entry::{
    TranscriptActor, TranscriptEntry, TranscriptSource, TranscriptVariant,
};
use crate::host::interactions::types::{
    InteractionEvent, InteractionSubagentRun, InteractionToolInvocation,
};

type DashboardJsonScalar = async_graphql::types::Json<serde_json::Value>;

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardTokenUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_creation_tokens: u64,
    pub(crate) cache_read_tokens: u64,
    pub(crate) api_call_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, SimpleObject)]
pub(crate) struct DashboardCommitFileDiff {
    pub(crate) filepath: String,
    pub(crate) additions_count: u64,
    pub(crate) deletions_count: u64,
    pub(crate) change_kind: Option<String>,
    pub(crate) copied_from_path: Option<String>,
    pub(crate) copied_from_blob_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardCommit {
    pub(crate) sha: String,
    pub(crate) parents: Vec<String>,
    pub(crate) author_name: String,
    pub(crate) author_email: String,
    pub(crate) timestamp: i64,
    pub(crate) message: String,
    pub(crate) files_touched: Vec<DashboardCommitFileDiff>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardCheckpoint {
    pub(crate) checkpoint_id: String,
    pub(crate) strategy: String,
    pub(crate) branch: String,
    pub(crate) checkpoints_count: u32,
    pub(crate) files_touched: Vec<DashboardCommitFileDiff>,
    pub(crate) session_count: usize,
    pub(crate) token_usage: Option<DashboardTokenUsage>,
    pub(crate) session_id: String,
    pub(crate) agents: Vec<String>,
    pub(crate) first_prompt_preview: String,
    pub(crate) created_at: String,
    pub(crate) is_task: bool,
    pub(crate) tool_use_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardCommitRow {
    pub(crate) commit: DashboardCommit,
    pub(crate) checkpoint: DashboardCheckpoint,
    pub(crate) checkpoints: Vec<DashboardCheckpoint>,
}

#[derive(Debug, Clone, PartialEq, SimpleObject)]
pub(crate) struct DashboardKpis {
    pub(crate) total_commits: usize,
    pub(crate) total_checkpoints: usize,
    pub(crate) total_agents: usize,
    pub(crate) total_sessions: usize,
    pub(crate) files_touched_count: usize,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_creation_tokens: u64,
    pub(crate) cache_read_tokens: u64,
    pub(crate) api_call_count: u64,
    pub(crate) average_tokens_per_checkpoint: f64,
    pub(crate) average_sessions_per_checkpoint: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardBranchSummary {
    pub(crate) branch: String,
    pub(crate) checkpoint_commits: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardUser {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardAgent {
    pub(crate) key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardRepository {
    pub(crate) repo_id: String,
    pub(crate) identity: String,
    pub(crate) name: String,
    pub(crate) provider: String,
    pub(crate) organization: String,
    pub(crate) default_branch: Option<String>,
}

#[derive(Debug, Clone, Default, InputObject)]
pub(crate) struct DashboardAnalyticsSqlInput {
    pub(crate) sql: String,
    #[graphql(name = "repoIds")]
    pub(crate) repo_ids: Option<Vec<String>>,
    #[graphql(name = "allRepos")]
    pub(crate) all_repos: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardAnalyticsColumn {
    pub(crate) name: String,
    pub(crate) logical_type: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct DashboardAnalyticsSqlResult {
    pub(crate) columns: Vec<DashboardAnalyticsColumn>,
    pub(crate) rows: DashboardJsonScalar,
    pub(crate) row_count: i32,
    pub(crate) truncated: bool,
    pub(crate) duration_ms: i32,
    pub(crate) repo_ids: Vec<String>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardCheckpointSessionDetail {
    pub(crate) session_index: usize,
    pub(crate) session_id: String,
    pub(crate) agent: String,
    pub(crate) created_at: String,
    pub(crate) is_task: bool,
    pub(crate) tool_use_id: String,
    pub(crate) metadata_json: String,
    pub(crate) transcript_jsonl: String,
    pub(crate) prompts_text: String,
    pub(crate) context_text: String,
    /// Canonical transcript rows for this session, derived from `transcript_jsonl`
    /// by the agent's `TranscriptEntryDeriver`. Empty when the agent has no
    /// deriver or the transcript could not be parsed.
    pub(crate) transcript_entries: Vec<DashboardTranscriptEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardCheckpointDetail {
    pub(crate) checkpoint_id: String,
    pub(crate) strategy: String,
    pub(crate) branch: String,
    pub(crate) checkpoints_count: u32,
    pub(crate) files_touched: Vec<DashboardCommitFileDiff>,
    pub(crate) session_count: usize,
    pub(crate) token_usage: Option<DashboardTokenUsage>,
    pub(crate) sessions: Vec<DashboardCheckpointSessionDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardBundleVersion {
    pub(crate) current_version: Option<String>,
    pub(crate) latest_applicable_version: Option<String>,
    pub(crate) install_available: bool,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardFetchBundleResult {
    pub(crate) installed_version: String,
    pub(crate) bundle_dir: String,
    pub(crate) status: String,
    pub(crate) checksum_verified: bool,
}

#[derive(Debug, Clone, Default, InputObject)]
pub(crate) struct DashboardInteractionFilterInput {
    pub(crate) since: Option<String>,
    pub(crate) until: Option<String>,
    pub(crate) actor: Option<String>,
    pub(crate) actor_id: Option<String>,
    pub(crate) actor_email: Option<String>,
    pub(crate) commit_author: Option<String>,
    pub(crate) commit_author_email: Option<String>,
    pub(crate) agent: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) turn_id: Option<String>,
    pub(crate) checkpoint_id: Option<String>,
    pub(crate) tool_use_id: Option<String>,
    pub(crate) tool_kind: Option<String>,
    pub(crate) has_checkpoint: Option<bool>,
    pub(crate) path: Option<String>,
}

#[derive(Debug, Clone, Default, InputObject)]
pub(crate) struct DashboardInteractionSearchInput {
    pub(crate) filter: Option<DashboardInteractionFilterInput>,
    pub(crate) query: String,
    pub(crate) limit: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionActor {
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionCommitAuthor {
    pub(crate) checkpoint_id: String,
    pub(crate) commit_sha: String,
    pub(crate) name: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) committed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionToolUse {
    pub(crate) tool_invocation_id: String,
    pub(crate) tool_use_id: String,
    pub(crate) session_id: String,
    pub(crate) turn_id: Option<String>,
    pub(crate) tool_kind: Option<String>,
    pub(crate) task_description: Option<String>,
    pub(crate) input_summary: Option<String>,
    pub(crate) output_summary: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) command: Option<String>,
    pub(crate) command_binary: Option<String>,
    pub(crate) command_argv: Vec<String>,
    pub(crate) transcript_path: Option<String>,
    pub(crate) started_at: Option<String>,
    pub(crate) ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionSubagentRun {
    pub(crate) subagent_run_id: String,
    pub(crate) session_id: String,
    pub(crate) turn_id: Option<String>,
    pub(crate) tool_use_id: Option<String>,
    pub(crate) subagent_id: Option<String>,
    pub(crate) subagent_type: Option<String>,
    pub(crate) task_description: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) transcript_path: Option<String>,
    pub(crate) child_session_id: Option<String>,
    pub(crate) started_at: Option<String>,
    pub(crate) ended_at: Option<String>,
}

/// Who emitted a transcript entry, from the reader's perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub(crate) enum DashboardTranscriptActor {
    User,
    Assistant,
    System,
}

impl From<TranscriptActor> for DashboardTranscriptActor {
    fn from(value: TranscriptActor) -> Self {
        match value {
            TranscriptActor::User => Self::User,
            TranscriptActor::Assistant => Self::Assistant,
            TranscriptActor::System => Self::System,
        }
    }
}

/// The semantic kind of a transcript entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub(crate) enum DashboardTranscriptVariant {
    Chat,
    Thinking,
    ToolUse,
    ToolResult,
}

impl From<TranscriptVariant> for DashboardTranscriptVariant {
    fn from(value: TranscriptVariant) -> Self {
        match value {
            TranscriptVariant::Chat => Self::Chat,
            TranscriptVariant::Thinking => Self::Thinking,
            TranscriptVariant::ToolUse => Self::ToolUse,
            TranscriptVariant::ToolResult => Self::ToolResult,
        }
    }
}

/// Where a transcript entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub(crate) enum DashboardTranscriptSource {
    Transcript,
    PromptFallback,
}

impl From<TranscriptSource> for DashboardTranscriptSource {
    fn from(value: TranscriptSource) -> Self {
        match value {
            TranscriptSource::Transcript => Self::Transcript,
            TranscriptSource::PromptFallback => Self::PromptFallback,
        }
    }
}

/// A single canonical transcript row. Replaces the agent-specific JSONL parsing
/// that the dashboard frontend used to do for each agent.
#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardTranscriptEntry {
    pub(crate) entry_id: String,
    pub(crate) session_id: String,
    pub(crate) turn_id: Option<String>,
    pub(crate) order: i32,
    pub(crate) timestamp: Option<String>,
    pub(crate) actor: DashboardTranscriptActor,
    pub(crate) variant: DashboardTranscriptVariant,
    pub(crate) source: DashboardTranscriptSource,
    pub(crate) text: String,
    pub(crate) tool_use_id: Option<String>,
    pub(crate) tool_kind: Option<String>,
    pub(crate) is_error: bool,
}

impl From<TranscriptEntry> for DashboardTranscriptEntry {
    fn from(value: TranscriptEntry) -> Self {
        Self {
            entry_id: value.entry_id,
            session_id: value.session_id,
            turn_id: value.turn_id,
            order: value.order,
            timestamp: value.timestamp,
            actor: value.actor.into(),
            variant: value.variant.into(),
            source: value.source.into(),
            text: value.text,
            tool_use_id: value.tool_use_id,
            tool_kind: value.tool_kind,
            is_error: value.is_error,
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct DashboardInteractionEvent {
    pub(crate) event_id: String,
    pub(crate) session_id: String,
    pub(crate) turn_id: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) actor: Option<DashboardInteractionActor>,
    pub(crate) event_type: String,
    pub(crate) event_time: String,
    pub(crate) source: Option<String>,
    pub(crate) sequence_number: i64,
    pub(crate) agent_type: String,
    pub(crate) model: Option<String>,
    pub(crate) tool_use_id: Option<String>,
    pub(crate) tool_kind: Option<String>,
    pub(crate) task_description: Option<String>,
    pub(crate) subagent_id: Option<String>,
    pub(crate) payload: Option<DashboardJsonScalar>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionSession {
    pub(crate) session_id: String,
    pub(crate) branch: Option<String>,
    pub(crate) actor: Option<DashboardInteractionActor>,
    pub(crate) agent_type: String,
    pub(crate) model: Option<String>,
    pub(crate) first_prompt: Option<String>,
    pub(crate) started_at: String,
    pub(crate) ended_at: Option<String>,
    pub(crate) last_event_at: Option<String>,
    pub(crate) turn_count: i32,
    pub(crate) checkpoint_count: i32,
    pub(crate) token_usage: Option<DashboardTokenUsage>,
    pub(crate) file_paths: Vec<String>,
    pub(crate) tool_uses: Vec<DashboardInteractionToolUse>,
    pub(crate) subagent_runs: Vec<DashboardInteractionSubagentRun>,
    pub(crate) linked_checkpoints: Vec<DashboardInteractionCommitAuthor>,
    pub(crate) latest_commit_author: Option<DashboardInteractionCommitAuthor>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionUpdate {
    pub(crate) repo_id: String,
    pub(crate) session_count: usize,
    pub(crate) turn_count: usize,
    pub(crate) latest_session_id: Option<String>,
    pub(crate) latest_session_updated_at: Option<String>,
    pub(crate) latest_turn_id: Option<String>,
    pub(crate) latest_turn_updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionTurn {
    pub(crate) turn_id: String,
    pub(crate) session_id: String,
    pub(crate) branch: Option<String>,
    pub(crate) actor: Option<DashboardInteractionActor>,
    pub(crate) turn_number: i32,
    pub(crate) prompt: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) agent_type: String,
    pub(crate) model: Option<String>,
    pub(crate) started_at: String,
    pub(crate) ended_at: Option<String>,
    pub(crate) token_usage: Option<DashboardTokenUsage>,
    pub(crate) files_modified: Vec<String>,
    pub(crate) checkpoint_id: Option<String>,
    pub(crate) tool_uses: Vec<DashboardInteractionToolUse>,
    pub(crate) subagent_runs: Vec<DashboardInteractionSubagentRun>,
    pub(crate) linked_checkpoints: Vec<DashboardInteractionCommitAuthor>,
    pub(crate) latest_commit_author: Option<DashboardInteractionCommitAuthor>,
    /// Canonical transcript rows for this turn, derived from the per-turn
    /// transcript fragment (with prompt fallback when no fragment exists).
    /// Empty by default; populated by the session-detail resolver via
    /// `host::interactions::derive_turn_transcript_entries`.
    pub(crate) transcript_entries: Vec<DashboardTranscriptEntry>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct DashboardInteractionSessionDetail {
    pub(crate) summary: DashboardInteractionSession,
    pub(crate) turns: Vec<DashboardInteractionTurn>,
    pub(crate) raw_events: Vec<DashboardInteractionEvent>,
    /// Canonical transcript rows for the whole session, derived from the
    /// session transcript file. Used by the session sidebar and tool-use tab.
    /// Empty by default; populated by the resolver via
    /// `host::interactions::derive_session_transcript_entries`.
    pub(crate) session_transcript_entries: Vec<DashboardTranscriptEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionSessionSearchHit {
    pub(crate) session: DashboardInteractionSession,
    pub(crate) score: i64,
    pub(crate) matched_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionTurnSearchHit {
    pub(crate) turn: DashboardInteractionTurn,
    pub(crate) session: DashboardInteractionSession,
    pub(crate) score: i64,
    pub(crate) matched_fields: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionKpis {
    pub(crate) total_sessions: usize,
    pub(crate) total_turns: usize,
    pub(crate) total_checkpoints: usize,
    pub(crate) total_tool_uses: usize,
    pub(crate) total_subagent_runs: usize,
    pub(crate) total_actors: usize,
    pub(crate) total_agents: usize,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_creation_tokens: u64,
    pub(crate) cache_read_tokens: u64,
    pub(crate) api_call_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionActorBucket {
    pub(crate) actor_id: String,
    pub(crate) actor_name: String,
    pub(crate) actor_email: String,
    pub(crate) actor_source: String,
    pub(crate) session_count: usize,
    pub(crate) turn_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionCommitAuthorBucket {
    pub(crate) author_name: String,
    pub(crate) author_email: String,
    pub(crate) checkpoint_count: usize,
    pub(crate) session_count: usize,
    pub(crate) turn_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, SimpleObject)]
pub(crate) struct DashboardInteractionAgentBucket {
    pub(crate) key: String,
    pub(crate) session_count: usize,
    pub(crate) turn_count: usize,
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

impl DashboardTokenUsage {
    pub(crate) fn from_metadata(metadata: &TokenUsageMetadata) -> Self {
        Self {
            input_tokens: metadata.input_tokens,
            output_tokens: metadata.output_tokens,
            cache_creation_tokens: metadata.cache_creation_tokens,
            cache_read_tokens: metadata.cache_read_tokens,
            api_call_count: metadata.api_call_count,
        }
    }
}

impl DashboardInteractionActor {
    fn from_parts(id: &str, name: &str, email: &str, source: &str) -> Option<Self> {
        if id.trim().is_empty()
            && name.trim().is_empty()
            && email.trim().is_empty()
            && source.trim().is_empty()
        {
            return None;
        }
        Some(Self {
            id: non_empty(id),
            name: non_empty(name),
            email: non_empty(email),
            source: non_empty(source),
        })
    }
}

impl DashboardInteractionCommitAuthor {
    fn from_link(link: &InteractionLinkedCheckpoint) -> Self {
        Self {
            checkpoint_id: link.checkpoint_id.clone(),
            commit_sha: link.commit_sha.clone(),
            name: non_empty(&link.author_name),
            email: non_empty(&link.author_email),
            committed_at: non_empty(&link.committed_at),
        }
    }
}

impl DashboardInteractionToolUse {
    fn from_domain(tool_use: &InteractionToolInvocation) -> Self {
        Self {
            tool_invocation_id: tool_use.tool_invocation_id.clone(),
            tool_use_id: tool_use.tool_use_id.clone(),
            session_id: tool_use.session_id.clone(),
            turn_id: non_empty(&tool_use.turn_id),
            tool_kind: non_empty(&tool_use.tool_name),
            task_description: non_empty(&tool_use.input_summary)
                .or_else(|| non_empty(&tool_use.output_summary)),
            input_summary: non_empty(&tool_use.input_summary),
            output_summary: non_empty(&tool_use.output_summary),
            source: non_empty(&tool_use.source),
            command: non_empty(&tool_use.command),
            command_binary: non_empty(&tool_use.command_binary),
            command_argv: tool_use.command_argv.clone(),
            transcript_path: non_empty(&tool_use.transcript_path),
            started_at: tool_use.started_at.clone(),
            ended_at: tool_use.ended_at.clone(),
        }
    }
}

impl DashboardInteractionSubagentRun {
    fn from_domain(subagent_run: &InteractionSubagentRun) -> Self {
        Self {
            subagent_run_id: subagent_run.subagent_run_id.clone(),
            session_id: subagent_run.session_id.clone(),
            turn_id: non_empty(&subagent_run.turn_id),
            tool_use_id: non_empty(&subagent_run.tool_use_id),
            subagent_id: non_empty(&subagent_run.subagent_id),
            subagent_type: non_empty(&subagent_run.subagent_type),
            task_description: non_empty(&subagent_run.task_description),
            source: non_empty(&subagent_run.source),
            transcript_path: non_empty(&subagent_run.transcript_path),
            child_session_id: non_empty(&subagent_run.child_session_id),
            started_at: subagent_run.started_at.clone(),
            ended_at: subagent_run.ended_at.clone(),
        }
    }
}

impl DashboardInteractionSession {
    pub(crate) fn from_summary(summary: &InteractionSessionSummary) -> Self {
        Self {
            session_id: summary.session.session_id.clone(),
            branch: non_empty(&summary.session.branch),
            actor: DashboardInteractionActor::from_parts(
                &summary.session.actor_id,
                &summary.session.actor_name,
                &summary.session.actor_email,
                &summary.session.actor_source,
            ),
            agent_type: summary.session.agent_type.clone(),
            model: non_empty(&summary.session.model),
            first_prompt: non_empty(&summary.session.first_prompt),
            started_at: summary.session.started_at.clone(),
            ended_at: summary.session.ended_at.clone(),
            last_event_at: non_empty(&summary.session.last_event_at),
            turn_count: summary.turn_count.try_into().unwrap_or(i32::MAX),
            checkpoint_count: summary.checkpoint_count.try_into().unwrap_or(i32::MAX),
            token_usage: summary
                .token_usage
                .as_ref()
                .map(DashboardTokenUsage::from_metadata),
            file_paths: summary.file_paths.clone(),
            tool_uses: summary
                .tool_uses
                .iter()
                .map(DashboardInteractionToolUse::from_domain)
                .collect(),
            subagent_runs: summary
                .subagent_runs
                .iter()
                .map(DashboardInteractionSubagentRun::from_domain)
                .collect(),
            linked_checkpoints: summary
                .linked_checkpoints
                .iter()
                .map(DashboardInteractionCommitAuthor::from_link)
                .collect(),
            latest_commit_author: summary
                .latest_commit_author
                .as_ref()
                .map(DashboardInteractionCommitAuthor::from_link),
        }
    }
}

impl DashboardInteractionTurn {
    pub(crate) fn from_summary(summary: &InteractionTurnSummary) -> Self {
        Self {
            turn_id: summary.turn.turn_id.clone(),
            session_id: summary.turn.session_id.clone(),
            branch: non_empty(&summary.turn.branch),
            actor: DashboardInteractionActor::from_parts(
                &summary.turn.actor_id,
                &summary.turn.actor_name,
                &summary.turn.actor_email,
                &summary.turn.actor_source,
            ),
            turn_number: i32::try_from(summary.turn.turn_number).unwrap_or(i32::MAX),
            prompt: non_empty(&summary.turn.prompt),
            summary: non_empty(&summary.turn.summary),
            agent_type: summary.turn.agent_type.clone(),
            model: non_empty(&summary.turn.model),
            started_at: summary.turn.started_at.clone(),
            ended_at: summary.turn.ended_at.clone(),
            token_usage: summary
                .turn
                .token_usage
                .as_ref()
                .map(DashboardTokenUsage::from_metadata),
            files_modified: summary.turn.files_modified.clone(),
            checkpoint_id: summary.turn.checkpoint_id.clone(),
            tool_uses: summary
                .tool_uses
                .iter()
                .map(DashboardInteractionToolUse::from_domain)
                .collect(),
            subagent_runs: summary
                .subagent_runs
                .iter()
                .map(DashboardInteractionSubagentRun::from_domain)
                .collect(),
            linked_checkpoints: summary
                .linked_checkpoints
                .iter()
                .map(DashboardInteractionCommitAuthor::from_link)
                .collect(),
            latest_commit_author: summary
                .latest_commit_author
                .as_ref()
                .map(DashboardInteractionCommitAuthor::from_link),
            transcript_entries: Vec::new(),
        }
    }
}

impl DashboardInteractionEvent {
    pub(crate) fn from_domain(event: &InteractionEvent) -> Self {
        Self {
            event_id: event.event_id.clone(),
            session_id: event.session_id.clone(),
            turn_id: event.turn_id.clone(),
            branch: non_empty(&event.branch),
            actor: DashboardInteractionActor::from_parts(
                &event.actor_id,
                &event.actor_name,
                &event.actor_email,
                &event.actor_source,
            ),
            event_type: event.event_type.as_str().to_string(),
            event_time: event.event_time.clone(),
            source: non_empty(&event.source),
            sequence_number: event.sequence_number,
            agent_type: event.agent_type.clone(),
            model: non_empty(&event.model),
            tool_use_id: non_empty(&event.tool_use_id),
            tool_kind: non_empty(&event.tool_kind),
            task_description: non_empty(&event.task_description),
            subagent_id: non_empty(&event.subagent_id),
            payload: Some(async_graphql::types::Json(event.payload.clone())),
        }
    }
}

impl DashboardInteractionSessionDetail {
    pub(crate) fn from_domain(detail: &InteractionSessionDetail) -> Self {
        Self {
            summary: DashboardInteractionSession::from_summary(&detail.summary),
            turns: detail
                .turns
                .iter()
                .map(DashboardInteractionTurn::from_summary)
                .collect(),
            raw_events: detail
                .raw_events
                .iter()
                .map(DashboardInteractionEvent::from_domain)
                .collect(),
            session_transcript_entries: Vec::new(),
        }
    }

    /// Replace `session_transcript_entries` with caller-derived rows.
    pub(crate) fn with_session_transcript_entries(
        mut self,
        entries: Vec<DashboardTranscriptEntry>,
    ) -> Self {
        self.session_transcript_entries = entries;
        self
    }
}

impl DashboardInteractionSessionSearchHit {
    pub(crate) fn from_domain(hit: &InteractionSessionSearchHit) -> Self {
        Self {
            session: DashboardInteractionSession::from_summary(&hit.session),
            score: hit.score,
            matched_fields: hit.matched_fields.clone(),
        }
    }
}

impl DashboardInteractionTurnSearchHit {
    pub(crate) fn from_domain(hit: &InteractionTurnSearchHit) -> Self {
        Self {
            turn: DashboardInteractionTurn::from_summary(&hit.turn),
            session: DashboardInteractionSession::from_summary(&hit.session),
            score: hit.score,
            matched_fields: hit.matched_fields.clone(),
        }
    }
}

impl DashboardInteractionUpdate {
    pub(crate) fn from_domain(snapshot: &InteractionChangeSnapshot) -> Self {
        Self {
            repo_id: snapshot.repo_id.clone(),
            session_count: snapshot.session_count,
            turn_count: snapshot.turn_count,
            latest_session_id: snapshot.latest_session_id.clone(),
            latest_session_updated_at: snapshot.latest_session_updated_at.clone(),
            latest_turn_id: snapshot.latest_turn_id.clone(),
            latest_turn_updated_at: snapshot.latest_turn_updated_at.clone(),
        }
    }
}

impl DashboardInteractionKpis {
    pub(crate) fn from_domain(kpis: &InteractionKpis) -> Self {
        Self {
            total_sessions: kpis.total_sessions,
            total_turns: kpis.total_turns,
            total_checkpoints: kpis.total_checkpoints,
            total_tool_uses: kpis.total_tool_uses,
            total_subagent_runs: kpis.total_subagent_runs,
            total_actors: kpis.total_actors,
            total_agents: kpis.total_agents,
            input_tokens: kpis.input_tokens,
            output_tokens: kpis.output_tokens,
            cache_creation_tokens: kpis.cache_creation_tokens,
            cache_read_tokens: kpis.cache_read_tokens,
            api_call_count: kpis.api_call_count,
        }
    }
}

impl DashboardInteractionActorBucket {
    pub(crate) fn from_domain(bucket: &InteractionActorBucket) -> Self {
        Self {
            actor_id: bucket.actor_id.clone(),
            actor_name: bucket.actor_name.clone(),
            actor_email: bucket.actor_email.clone(),
            actor_source: bucket.actor_source.clone(),
            session_count: bucket.session_count,
            turn_count: bucket.turn_count,
        }
    }
}

impl DashboardInteractionCommitAuthorBucket {
    pub(crate) fn from_domain(bucket: &InteractionCommitAuthorBucket) -> Self {
        Self {
            author_name: bucket.author_name.clone(),
            author_email: bucket.author_email.clone(),
            checkpoint_count: bucket.checkpoint_count,
            session_count: bucket.session_count,
            turn_count: bucket.turn_count,
        }
    }
}

impl DashboardInteractionAgentBucket {
    pub(crate) fn from_domain(bucket: &InteractionAgentBucket) -> Self {
        Self {
            key: bucket.key.clone(),
            session_count: bucket.session_count,
            turn_count: bucket.turn_count,
        }
    }
}
