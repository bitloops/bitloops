use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InteractionSession {
    pub session_id: String,
    pub repo_id: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub actor_name: String,
    #[serde(default)]
    pub actor_email: String,
    #[serde(default)]
    pub actor_source: String,
    #[serde(default)]
    pub agent_type: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub first_prompt: String,
    #[serde(default)]
    pub transcript_path: String,
    #[serde(default)]
    pub worktree_path: String,
    #[serde(default)]
    pub worktree_id: String,
    #[serde(default)]
    pub started_at: String,
    pub ended_at: Option<String>,
    #[serde(default)]
    pub last_event_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InteractionTurn {
    pub turn_id: String,
    pub session_id: String,
    pub repo_id: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub actor_name: String,
    #[serde(default)]
    pub actor_email: String,
    #[serde(default)]
    pub actor_source: String,
    #[serde(default)]
    pub turn_number: u32,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub agent_type: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub started_at: String,
    pub ended_at: Option<String>,
    pub token_usage: Option<TokenUsageMetadata>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub prompt_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_offset_start: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_offset_end: Option<i64>,
    #[serde(default)]
    pub transcript_fragment: String,
    #[serde(default)]
    pub files_modified: Vec<String>,
    pub checkpoint_id: Option<String>,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct InteractionEvent {
    pub event_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub repo_id: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub actor_name: String,
    #[serde(default)]
    pub actor_email: String,
    #[serde(default)]
    pub actor_source: String,
    pub event_type: InteractionEventType,
    pub event_time: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub sequence_number: i64,
    #[serde(default)]
    pub agent_type: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub tool_kind: String,
    #[serde(default)]
    pub task_description: String,
    #[serde(default)]
    pub subagent_id: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct InteractionToolInvocation {
    pub tool_invocation_id: String,
    pub repo_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turn_id: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub input_summary: String,
    #[serde(default)]
    pub output_summary: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub command_binary: String,
    #[serde(default)]
    pub command_argv: Vec<String>,
    #[serde(default)]
    pub transcript_path: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub started_sequence_number: Option<i64>,
    pub ended_sequence_number: Option<i64>,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct InteractionSubagentRun {
    pub subagent_run_id: String,
    pub repo_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turn_id: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub subagent_id: String,
    #[serde(default)]
    pub subagent_type: String,
    #[serde(default)]
    pub task_description: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub transcript_path: String,
    #[serde(default)]
    pub child_session_id: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub started_sequence_number: Option<i64>,
    pub ended_sequence_number: Option<i64>,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct InteractionEventFilter {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub event_type: Option<InteractionEventType>,
    pub since: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InteractionMutation {
    UpsertSession {
        session: InteractionSession,
    },
    UpsertTurn {
        turn: InteractionTurn,
    },
    AppendEvent {
        event: InteractionEvent,
    },
    AssignCheckpoint {
        turn_ids: Vec<String>,
        checkpoint_id: String,
        assigned_at: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InteractionEventType {
    #[default]
    SessionStart,
    TurnStart,
    TurnEnd,
    Compaction,
    SessionEnd,
    ToolInvocationObserved,
    ToolResultObserved,
    SubagentStart,
    SubagentEnd,
}

impl InteractionEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
            Self::TurnStart => "turn_start",
            Self::TurnEnd => "turn_end",
            Self::Compaction => "compaction",
            Self::SessionEnd => "session_end",
            Self::ToolInvocationObserved => "tool_invocation_observed",
            Self::ToolResultObserved => "tool_result_observed",
            Self::SubagentStart => "subagent_run_started",
            Self::SubagentEnd => "subagent_run_finished",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "session_start" => Some(Self::SessionStart),
            "turn_start" => Some(Self::TurnStart),
            "turn_end" => Some(Self::TurnEnd),
            "compaction" => Some(Self::Compaction),
            "session_end" => Some(Self::SessionEnd),
            "tool_invocation_observed" => Some(Self::ToolInvocationObserved),
            "tool_result_observed" => Some(Self::ToolResultObserved),
            "subagent_start" | "subagent_run_started" => Some(Self::SubagentStart),
            "subagent_end" | "subagent_run_finished" => Some(Self::SubagentEnd),
            _ => None,
        }
    }
}

impl std::fmt::Display for InteractionEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_round_trip() {
        let variants = [
            InteractionEventType::SessionStart,
            InteractionEventType::TurnStart,
            InteractionEventType::TurnEnd,
            InteractionEventType::Compaction,
            InteractionEventType::SessionEnd,
            InteractionEventType::ToolInvocationObserved,
            InteractionEventType::ToolResultObserved,
            InteractionEventType::SubagentStart,
            InteractionEventType::SubagentEnd,
        ];
        for v in variants {
            let s = v.as_str();
            let parsed = InteractionEventType::parse(s).expect(s);
            assert_eq!(v, parsed);
        }
        assert_eq!(
            InteractionEventType::parse("subagent_start"),
            Some(InteractionEventType::SubagentStart)
        );
        assert_eq!(
            InteractionEventType::parse("subagent_end"),
            Some(InteractionEventType::SubagentEnd)
        );
    }

    #[test]
    fn interaction_mutation_round_trip() {
        let mutation = InteractionMutation::AssignCheckpoint {
            turn_ids: vec!["turn-1".into(), "turn-2".into()],
            checkpoint_id: "cp-1".into(),
            assigned_at: "2026-04-05T12:00:00Z".into(),
        };
        let json = serde_json::to_string(&mutation).unwrap();
        let parsed: InteractionMutation = serde_json::from_str(&json).unwrap();
        match parsed {
            InteractionMutation::AssignCheckpoint {
                turn_ids,
                checkpoint_id,
                assigned_at,
            } => {
                assert_eq!(turn_ids, vec!["turn-1", "turn-2"]);
                assert_eq!(checkpoint_id, "cp-1");
                assert_eq!(assigned_at, "2026-04-05T12:00:00Z");
            }
            other => panic!("unexpected mutation: {other:?}"),
        }
    }

    #[test]
    fn interaction_session_serde_round_trip() {
        let session = InteractionSession {
            session_id: "sess-1".into(),
            repo_id: "repo-1".into(),
            branch: "main".into(),
            actor_email: "alice@example.com".into(),
            agent_type: "claude-code".into(),
            model: "claude-opus-4-6".into(),
            first_prompt: "hello".into(),
            started_at: "2026-04-04T10:00:00Z".into(),
            last_event_at: "2026-04-04T10:01:00Z".into(),
            updated_at: "2026-04-04T10:01:00Z".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&session).unwrap();
        let parsed: InteractionSession = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.session_id, "sess-1");
        assert_eq!(parsed.branch, "main");
        assert_eq!(parsed.agent_type, "claude-code");
        assert_eq!(parsed.actor_email, "alice@example.com");
        assert!(parsed.ended_at.is_none());
    }

    #[test]
    fn interaction_turn_serde_round_trip() {
        let turn = InteractionTurn {
            turn_id: "turn-1".into(),
            session_id: "sess-1".into(),
            repo_id: "repo-1".into(),
            branch: "main".into(),
            actor_email: "alice@example.com".into(),
            turn_number: 1,
            prompt: "fix the bug".into(),
            agent_type: "claude-code".into(),
            started_at: "2026-04-04T10:01:00Z".into(),
            token_usage: Some(TokenUsageMetadata {
                input_tokens: 100,
                output_tokens: 50,
                ..Default::default()
            }),
            summary: "fixed bug".into(),
            prompt_count: 2,
            transcript_offset_start: Some(4),
            transcript_offset_end: Some(9),
            transcript_fragment: "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n".into(),
            files_modified: vec!["src/main.rs".into()],
            updated_at: "2026-04-04T10:02:00Z".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&turn).unwrap();
        let parsed: InteractionTurn = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.turn_id, "turn-1");
        assert_eq!(parsed.branch, "main");
        assert_eq!(parsed.turn_number, 1);
        assert_eq!(parsed.token_usage.unwrap().input_tokens, 100);
        assert_eq!(parsed.actor_email, "alice@example.com");
        assert_eq!(parsed.summary, "fixed bug");
        assert_eq!(parsed.prompt_count, 2);
        assert_eq!(parsed.transcript_offset_start, Some(4));
        assert_eq!(parsed.transcript_offset_end, Some(9));
        assert!(parsed.transcript_fragment.contains("\"assistant\""));
        assert_eq!(parsed.files_modified, vec!["src/main.rs"]);
        assert!(parsed.checkpoint_id.is_none());
    }
}
