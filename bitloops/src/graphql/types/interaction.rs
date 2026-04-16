use async_graphql::{ID, InputObject, SimpleObject};

use crate::graphql::types::{DateTimeScalar, JsonScalar};
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::interactions::query::{
    InteractionBrowseFilter, InteractionLinkedCheckpoint, InteractionSessionSearchHit,
    InteractionSessionSummary, InteractionTurnSearchHit, InteractionTurnSummary,
};
use crate::host::interactions::types::{InteractionEvent, InteractionToolUse};

#[derive(Debug, Clone, Default, InputObject)]
pub struct InteractionFilterInput {
    pub since: Option<DateTimeScalar>,
    pub until: Option<DateTimeScalar>,
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

#[derive(Debug, Clone, Default, InputObject)]
pub struct InteractionSearchInputObject {
    pub filter: Option<InteractionFilterInput>,
    pub query: String,
    pub limit: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct InteractionActorObject {
    pub id: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct InteractionCommitAuthorObject {
    pub checkpoint_id: String,
    pub commit_sha: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub committed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct InteractionToolUseObject {
    pub id: ID,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub tool_kind: Option<String>,
    pub task_description: Option<String>,
    pub subagent_id: Option<String>,
    pub transcript_path: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct InteractionSessionObject {
    pub id: ID,
    pub branch: Option<String>,
    pub actor: Option<InteractionActorObject>,
    pub agent_type: String,
    pub model: Option<String>,
    pub first_prompt: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub last_event_at: Option<String>,
    pub turn_count: i32,
    pub checkpoint_count: i32,
    pub token_usage: Option<InteractionTokenUsage>,
    pub file_paths: Vec<String>,
    pub tool_uses: Vec<InteractionToolUseObject>,
    pub linked_checkpoints: Vec<InteractionCommitAuthorObject>,
    pub latest_commit_author: Option<InteractionCommitAuthorObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct InteractionTurnObject {
    pub id: ID,
    pub session_id: String,
    pub branch: Option<String>,
    pub actor: Option<InteractionActorObject>,
    pub turn_number: i32,
    pub prompt: Option<String>,
    pub summary: Option<String>,
    pub agent_type: String,
    pub model: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub token_usage: Option<InteractionTokenUsage>,
    pub files_modified: Vec<String>,
    pub checkpoint_id: Option<String>,
    pub tool_uses: Vec<InteractionToolUseObject>,
    pub linked_checkpoints: Vec<InteractionCommitAuthorObject>,
    pub latest_commit_author: Option<InteractionCommitAuthorObject>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct InteractionEventObject {
    pub id: ID,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub branch: Option<String>,
    pub actor: Option<InteractionActorObject>,
    pub event_type: String,
    pub event_time: String,
    pub agent_type: String,
    pub model: Option<String>,
    pub tool_use_id: Option<String>,
    pub tool_kind: Option<String>,
    pub task_description: Option<String>,
    pub subagent_id: Option<String>,
    pub payload: Option<JsonScalar>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct InteractionSessionSearchHitObject {
    pub session: InteractionSessionObject,
    pub score: i64,
    pub matched_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct InteractionTurnSearchHitObject {
    pub turn: InteractionTurnObject,
    pub session: InteractionSessionObject,
    pub score: i64,
    pub matched_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct InteractionTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub api_call_count: u64,
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

impl InteractionFilterInput {
    pub fn to_domain(&self) -> InteractionBrowseFilter {
        InteractionBrowseFilter {
            since: self.since.as_ref().map(|value| value.as_str().to_string()),
            until: self.until.as_ref().map(|value| value.as_str().to_string()),
            actor: self.actor.clone(),
            actor_id: self.actor_id.clone(),
            actor_email: self.actor_email.clone(),
            commit_author: self.commit_author.clone(),
            commit_author_email: self.commit_author_email.clone(),
            agent: self.agent.clone(),
            model: self.model.clone(),
            branch: self.branch.clone(),
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            checkpoint_id: self.checkpoint_id.clone(),
            tool_use_id: self.tool_use_id.clone(),
            tool_kind: self.tool_kind.clone(),
            has_checkpoint: self.has_checkpoint,
            path: self.path.clone(),
        }
    }
}

impl InteractionSearchInputObject {
    pub fn to_domain(&self) -> crate::host::interactions::query::InteractionSearchInput {
        crate::host::interactions::query::InteractionSearchInput {
            filter: self.filter.clone().unwrap_or_default().to_domain(),
            query: self.query.clone(),
            limit: self
                .limit
                .map(|value| usize::try_from(value.max(1)).unwrap_or(25))
                .unwrap_or(25),
        }
    }
}

impl InteractionActorObject {
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

impl InteractionCommitAuthorObject {
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

impl InteractionToolUseObject {
    fn from_domain(tool_use: &InteractionToolUse) -> Self {
        Self {
            id: ID(tool_use.tool_use_id.clone()),
            session_id: tool_use.session_id.clone(),
            turn_id: non_empty(&tool_use.turn_id),
            tool_kind: non_empty(&tool_use.tool_kind),
            task_description: non_empty(&tool_use.task_description),
            subagent_id: non_empty(&tool_use.subagent_id),
            transcript_path: non_empty(&tool_use.transcript_path),
            started_at: tool_use.started_at.clone(),
            ended_at: tool_use.ended_at.clone(),
        }
    }
}

impl InteractionSessionObject {
    pub fn from_summary(summary: &InteractionSessionSummary) -> Self {
        Self {
            id: ID(summary.session.session_id.clone()),
            branch: non_empty(&summary.session.branch),
            actor: InteractionActorObject::from_parts(
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
                .map(InteractionTokenUsage::from_metadata),
            file_paths: summary.file_paths.clone(),
            tool_uses: summary
                .tool_uses
                .iter()
                .map(InteractionToolUseObject::from_domain)
                .collect(),
            linked_checkpoints: summary
                .linked_checkpoints
                .iter()
                .map(InteractionCommitAuthorObject::from_link)
                .collect(),
            latest_commit_author: summary
                .latest_commit_author
                .as_ref()
                .map(InteractionCommitAuthorObject::from_link),
        }
    }

    pub fn cursor(&self) -> String {
        format!(
            "{}|{}",
            self.last_event_at
                .as_deref()
                .unwrap_or(self.started_at.as_str()),
            self.id.as_str()
        )
    }
}

impl InteractionTurnObject {
    pub fn from_summary(summary: &InteractionTurnSummary) -> Self {
        Self {
            id: ID(summary.turn.turn_id.clone()),
            session_id: summary.turn.session_id.clone(),
            branch: non_empty(&summary.turn.branch),
            actor: InteractionActorObject::from_parts(
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
                .map(InteractionTokenUsage::from_metadata),
            files_modified: summary.turn.files_modified.clone(),
            checkpoint_id: summary.turn.checkpoint_id.clone(),
            tool_uses: summary
                .tool_uses
                .iter()
                .map(InteractionToolUseObject::from_domain)
                .collect(),
            linked_checkpoints: summary
                .linked_checkpoints
                .iter()
                .map(InteractionCommitAuthorObject::from_link)
                .collect(),
            latest_commit_author: summary
                .latest_commit_author
                .as_ref()
                .map(InteractionCommitAuthorObject::from_link),
        }
    }

    pub fn cursor(&self) -> String {
        format!(
            "{}|{}",
            self.ended_at.as_deref().unwrap_or(self.started_at.as_str()),
            self.id.as_str()
        )
    }
}

impl InteractionEventObject {
    pub fn from_domain(event: &InteractionEvent) -> Self {
        Self {
            id: ID(event.event_id.clone()),
            session_id: event.session_id.clone(),
            turn_id: event.turn_id.clone(),
            branch: non_empty(&event.branch),
            actor: InteractionActorObject::from_parts(
                &event.actor_id,
                &event.actor_name,
                &event.actor_email,
                &event.actor_source,
            ),
            event_type: event.event_type.as_str().to_string(),
            event_time: event.event_time.clone(),
            agent_type: event.agent_type.clone(),
            model: non_empty(&event.model),
            tool_use_id: non_empty(&event.tool_use_id),
            tool_kind: non_empty(&event.tool_kind),
            task_description: non_empty(&event.task_description),
            subagent_id: non_empty(&event.subagent_id),
            payload: Some(async_graphql::types::Json(event.payload.clone())),
        }
    }

    pub fn cursor(&self) -> String {
        format!("{}|{}", self.event_time, self.id.as_str())
    }
}

impl InteractionTokenUsage {
    pub fn from_metadata(metadata: &TokenUsageMetadata) -> Self {
        Self {
            input_tokens: metadata.input_tokens,
            output_tokens: metadata.output_tokens,
            cache_creation_tokens: metadata.cache_creation_tokens,
            cache_read_tokens: metadata.cache_read_tokens,
            api_call_count: metadata.api_call_count,
        }
    }
}

impl InteractionSessionSearchHitObject {
    pub fn from_hit(hit: &InteractionSessionSearchHit) -> Self {
        Self {
            session: InteractionSessionObject::from_summary(&hit.session),
            score: hit.score,
            matched_fields: hit.matched_fields.clone(),
        }
    }
}

impl InteractionTurnSearchHitObject {
    pub fn from_hit(hit: &InteractionTurnSearchHit) -> Self {
        Self {
            turn: InteractionTurnObject::from_summary(&hit.turn),
            session: InteractionSessionObject::from_summary(&hit.session),
            score: hit.score,
            matched_fields: hit.matched_fields.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_input_converts_dates_and_actor_fields() {
        let input = InteractionFilterInput {
            since: Some(DateTimeScalar::from_rfc3339("2026-04-01T10:00:00Z").unwrap()),
            actor_email: Some("alice@example.com".into()),
            branch: Some("main".into()),
            ..Default::default()
        };
        let domain = input.to_domain();
        assert_eq!(domain.since.as_deref(), Some("2026-04-01T10:00:00Z"));
        assert_eq!(domain.actor_email.as_deref(), Some("alice@example.com"));
        assert_eq!(domain.branch.as_deref(), Some("main"));
    }
}
