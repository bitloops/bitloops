use async_graphql::{ID, SimpleObject};

use crate::graphql::types::JsonScalar;
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::interactions::types::{InteractionEvent, InteractionSession, InteractionTurn};

#[derive(Debug, Clone, SimpleObject)]
pub struct InteractionSessionObject {
    pub id: ID,
    pub agent_type: String,
    pub model: Option<String>,
    pub first_prompt: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct InteractionTurnObject {
    pub id: ID,
    pub session_id: String,
    pub turn_number: i32,
    pub prompt: Option<String>,
    pub agent_type: String,
    pub model: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub token_usage: Option<InteractionTokenUsage>,
    pub files_modified: Vec<String>,
    pub checkpoint_id: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct InteractionEventObject {
    pub id: ID,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub event_type: String,
    pub event_time: String,
    pub agent_type: String,
    pub model: Option<String>,
    pub payload: Option<JsonScalar>,
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

impl InteractionSessionObject {
    pub fn from_domain(session: &InteractionSession) -> Self {
        Self {
            id: ID(session.session_id.clone()),
            agent_type: session.agent_type.clone(),
            model: non_empty(&session.model),
            first_prompt: non_empty(&session.first_prompt),
            started_at: session.started_at.clone(),
            ended_at: session.ended_at.clone(),
        }
    }
}

impl InteractionTurnObject {
    pub fn from_domain(turn: &InteractionTurn) -> Self {
        Self {
            id: ID(turn.turn_id.clone()),
            session_id: turn.session_id.clone(),
            turn_number: i32::try_from(turn.turn_number).unwrap_or(i32::MAX),
            prompt: non_empty(&turn.prompt),
            agent_type: turn.agent_type.clone(),
            model: non_empty(&turn.model),
            started_at: turn.started_at.clone(),
            ended_at: turn.ended_at.clone(),
            token_usage: turn
                .token_usage
                .as_ref()
                .map(InteractionTokenUsage::from_metadata),
            files_modified: turn.files_modified.clone(),
            checkpoint_id: turn.checkpoint_id.clone(),
        }
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

impl InteractionEventObject {
    pub fn from_domain(event: &InteractionEvent) -> Self {
        Self {
            id: ID(event.event_id.clone()),
            session_id: event.session_id.clone(),
            turn_id: event.turn_id.clone(),
            event_type: event.event_type.as_str().to_string(),
            event_time: event.event_time.clone(),
            agent_type: event.agent_type.clone(),
            model: non_empty(&event.model),
            payload: Some(async_graphql::types::Json(event.payload.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_from_domain_basic() {
        let session = InteractionSession {
            session_id: "sess-1".into(),
            repo_id: "repo-1".into(),
            agent_type: "claude-code".into(),
            model: "claude-opus-4-6".into(),
            first_prompt: "hello world".into(),
            started_at: "2026-04-04T10:00:00Z".into(),
            ended_at: Some("2026-04-04T11:00:00Z".into()),
            ..Default::default()
        };
        let obj = InteractionSessionObject::from_domain(&session);
        assert_eq!(obj.id.as_str(), "sess-1");
        assert_eq!(obj.agent_type, "claude-code");
        assert_eq!(obj.model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(obj.first_prompt.as_deref(), Some("hello world"));
        assert_eq!(obj.started_at, "2026-04-04T10:00:00Z");
        assert_eq!(obj.ended_at.as_deref(), Some("2026-04-04T11:00:00Z"));
    }

    #[test]
    fn session_from_domain_empty_optional_fields() {
        let session = InteractionSession {
            session_id: "sess-2".into(),
            repo_id: "repo-1".into(),
            agent_type: "cursor".into(),
            model: "".into(),
            first_prompt: "  ".into(),
            started_at: "2026-04-04T10:00:00Z".into(),
            ended_at: None,
            ..Default::default()
        };
        let obj = InteractionSessionObject::from_domain(&session);
        assert_eq!(obj.id.as_str(), "sess-2");
        assert_eq!(obj.agent_type, "cursor");
        assert!(obj.model.is_none(), "empty model should be None");
        assert!(
            obj.first_prompt.is_none(),
            "whitespace-only prompt should be None"
        );
        assert!(obj.ended_at.is_none());
    }

    #[test]
    fn turn_from_domain_basic() {
        let turn = InteractionTurn {
            turn_id: "turn-1".into(),
            session_id: "sess-1".into(),
            repo_id: "repo-1".into(),
            turn_number: 3,
            prompt: "fix the bug".into(),
            agent_type: "claude-code".into(),
            model: "claude-opus-4-6".into(),
            started_at: "2026-04-04T10:01:00Z".into(),
            ended_at: Some("2026-04-04T10:02:00Z".into()),
            token_usage: Some(TokenUsageMetadata {
                input_tokens: 200,
                output_tokens: 100,
                cache_creation_tokens: 10,
                cache_read_tokens: 5,
                api_call_count: 3,
                subagent_tokens: None,
            }),
            summary: "fixed bug".into(),
            prompt_count: 2,
            transcript_offset_start: Some(4),
            transcript_offset_end: Some(9),
            files_modified: vec!["src/main.rs".into(), "src/lib.rs".into()],
            checkpoint_id: Some("cp-1".into()),
            updated_at: "2026-04-04T10:02:00Z".into(),
        };
        let obj = InteractionTurnObject::from_domain(&turn);
        assert_eq!(obj.id.as_str(), "turn-1");
        assert_eq!(obj.session_id, "sess-1");
        assert_eq!(obj.turn_number, 3);
        assert_eq!(obj.prompt.as_deref(), Some("fix the bug"));
        assert_eq!(obj.agent_type, "claude-code");
        assert_eq!(obj.model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(obj.started_at, "2026-04-04T10:01:00Z");
        assert_eq!(obj.ended_at.as_deref(), Some("2026-04-04T10:02:00Z"));
        assert_eq!(obj.files_modified, vec!["src/main.rs", "src/lib.rs"]);
        assert_eq!(obj.checkpoint_id.as_deref(), Some("cp-1"));

        let token_usage = obj.token_usage.unwrap();
        assert_eq!(token_usage.input_tokens, 200);
        assert_eq!(token_usage.output_tokens, 100);
        assert_eq!(token_usage.cache_creation_tokens, 10);
        assert_eq!(token_usage.cache_read_tokens, 5);
        assert_eq!(token_usage.api_call_count, 3);
    }

    #[test]
    fn turn_from_domain_no_token_usage() {
        let turn = InteractionTurn {
            turn_id: "turn-2".into(),
            session_id: "sess-1".into(),
            repo_id: "repo-1".into(),
            turn_number: 1,
            prompt: "".into(),
            agent_type: "claude-code".into(),
            model: "".into(),
            started_at: "2026-04-04T10:01:00Z".into(),
            ended_at: None,
            token_usage: None,
            summary: String::new(),
            prompt_count: 0,
            transcript_offset_start: None,
            transcript_offset_end: None,
            files_modified: Vec::new(),
            checkpoint_id: None,
            updated_at: "2026-04-04T10:01:00Z".into(),
        };
        let obj = InteractionTurnObject::from_domain(&turn);
        assert_eq!(obj.id.as_str(), "turn-2");
        assert!(obj.prompt.is_none());
        assert!(obj.model.is_none());
        assert!(obj.ended_at.is_none());
        assert!(obj.token_usage.is_none());
        assert!(obj.files_modified.is_empty());
        assert!(obj.checkpoint_id.is_none());
    }

    #[test]
    fn token_usage_from_metadata() {
        let metadata = TokenUsageMetadata {
            input_tokens: 500,
            output_tokens: 250,
            cache_creation_tokens: 20,
            cache_read_tokens: 15,
            api_call_count: 7,
            subagent_tokens: None,
        };
        let usage = InteractionTokenUsage::from_metadata(&metadata);
        assert_eq!(usage.input_tokens, 500);
        assert_eq!(usage.output_tokens, 250);
        assert_eq!(usage.cache_creation_tokens, 20);
        assert_eq!(usage.cache_read_tokens, 15);
        assert_eq!(usage.api_call_count, 7);
    }
}
