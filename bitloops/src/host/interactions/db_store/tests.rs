use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;

use super::*;
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::interactions::store::{InteractionEventRepository, InteractionSpool};
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventFilter, InteractionEventType, InteractionSession,
    InteractionTurn,
};
use crate::storage::sqlite::SqliteConnectionPool;

struct MockRepository {
    repo_id: String,
    sessions: Mutex<HashMap<String, InteractionSession>>,
    turns: Mutex<HashMap<String, InteractionTurn>>,
    events: Mutex<Vec<InteractionEvent>>,
}

impl MockRepository {
    fn new(repo_id: &str) -> Self {
        Self {
            repo_id: repo_id.to_string(),
            sessions: Mutex::new(HashMap::new()),
            turns: Mutex::new(HashMap::new()),
            events: Mutex::new(Vec::new()),
        }
    }
}

impl InteractionEventRepository for MockRepository {
    fn repo_id(&self) -> &str {
        &self.repo_id
    }

    fn upsert_session(&self, session: &InteractionSession) -> Result<()> {
        self.sessions
            .lock()
            .unwrap()
            .insert(session.session_id.clone(), session.clone());
        Ok(())
    }

    fn upsert_turn(&self, turn: &InteractionTurn) -> Result<()> {
        self.turns
            .lock()
            .unwrap()
            .insert(turn.turn_id.clone(), turn.clone());
        Ok(())
    }

    fn append_event(&self, event: &InteractionEvent) -> Result<()> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }

    fn assign_checkpoint_to_turns(
        &self,
        turn_ids: &[String],
        checkpoint_id: &str,
        assigned_at: &str,
    ) -> Result<()> {
        let mut turns = self.turns.lock().unwrap();
        for turn_id in turn_ids {
            if let Some(turn) = turns.get_mut(turn_id) {
                turn.checkpoint_id = Some(checkpoint_id.to_string());
                turn.updated_at = assigned_at.to_string();
            }
        }
        Ok(())
    }

    fn list_sessions(
        &self,
        _agent: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<InteractionSession>> {
        Ok(self.sessions.lock().unwrap().values().cloned().collect())
    }

    fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>> {
        Ok(self.sessions.lock().unwrap().get(session_id).cloned())
    }

    fn list_turns_for_session(
        &self,
        session_id: &str,
        _limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        Ok(self
            .turns
            .lock()
            .unwrap()
            .values()
            .filter(|turn| turn.session_id == session_id)
            .cloned()
            .collect())
    }

    fn list_uncheckpointed_turns(&self) -> Result<Vec<InteractionTurn>> {
        Ok(self
            .turns
            .lock()
            .unwrap()
            .values()
            .filter(|turn| turn.checkpoint_id.as_deref().unwrap_or("").is_empty())
            .cloned()
            .collect())
    }

    fn list_events(
        &self,
        _filter: &InteractionEventFilter,
        _limit: usize,
    ) -> Result<Vec<InteractionEvent>> {
        Ok(self.events.lock().unwrap().clone())
    }
}

fn test_spool() -> (tempfile::TempDir, SqliteInteractionSpool) {
    let dir = tempfile::tempdir().expect("tempdir");
    let sqlite =
        SqliteConnectionPool::connect(dir.path().join("interaction-spool.sqlite")).expect("sqlite");
    (
        dir,
        SqliteInteractionSpool::new(sqlite, "repo-test".into()).expect("spool"),
    )
}

fn sample_session() -> InteractionSession {
    InteractionSession {
        session_id: "session-1".into(),
        repo_id: "repo-test".into(),
        agent_type: "codex".into(),
        model: "gpt-5.4".into(),
        first_prompt: "hello".into(),
        transcript_path: "/tmp/transcript.jsonl".into(),
        worktree_path: "/tmp/repo".into(),
        worktree_id: "main".into(),
        started_at: "2026-04-05T10:00:00Z".into(),
        last_event_at: "2026-04-05T10:00:00Z".into(),
        updated_at: "2026-04-05T10:00:00Z".into(),
        ..Default::default()
    }
}

fn sample_turn() -> InteractionTurn {
    InteractionTurn {
        turn_id: "turn-1".into(),
        session_id: "session-1".into(),
        repo_id: "repo-test".into(),
        turn_number: 1,
        prompt: "fix it".into(),
        agent_type: "codex".into(),
        model: "gpt-5.4".into(),
        started_at: "2026-04-05T10:00:01Z".into(),
        ended_at: Some("2026-04-05T10:00:02Z".into()),
        token_usage: Some(TokenUsageMetadata {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        }),
        summary: "completed change".into(),
        prompt_count: 2,
        transcript_offset_start: Some(1),
        transcript_offset_end: Some(3),
        transcript_fragment: "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n".into(),
        files_modified: vec!["src/main.rs".into()],
        updated_at: "2026-04-05T10:00:02Z".into(),
        ..Default::default()
    }
}

fn sample_event() -> InteractionEvent {
    InteractionEvent {
        event_id: "event-1".into(),
        session_id: "session-1".into(),
        turn_id: Some("turn-1".into()),
        repo_id: "repo-test".into(),
        event_type: InteractionEventType::TurnEnd,
        event_time: "2026-04-05T10:00:02Z".into(),
        agent_type: "codex".into(),
        model: "gpt-5.4".into(),
        payload: serde_json::json!({"files_modified": ["src/main.rs"]}),
    }
}

#[test]
fn record_and_flush_interactions() {
    let (_dir, spool) = test_spool();
    let repository = MockRepository::new("repo-test");

    spool
        .record_session(&sample_session())
        .expect("record session");
    spool.record_turn(&sample_turn()).expect("record turn");
    spool.record_event(&sample_event()).expect("record event");

    let flushed = spool.flush(&repository).expect("flush");
    assert_eq!(flushed, 3);
    assert!(repository.load_session("session-1").unwrap().is_some());
    assert_eq!(
        repository
            .list_turns_for_session("session-1", 10)
            .unwrap()
            .len(),
        1
    );
    assert!(
        repository.list_turns_for_session("session-1", 10).unwrap()[0]
            .transcript_fragment
            .contains("\"assistant\"")
    );
    assert_eq!(
        repository
            .list_events(&Default::default(), 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn assign_checkpoint_updates_local_and_remote_turns() {
    let (_dir, spool) = test_spool();
    let repository = MockRepository::new("repo-test");
    spool.record_turn(&sample_turn()).expect("record turn");
    spool.flush(&repository).expect("flush turn");

    spool
        .assign_checkpoint_to_turns(&["turn-1".to_string()], "cp-1", "2026-04-05T10:10:00Z")
        .expect("assign checkpoint");
    spool.flush(&repository).expect("flush assignment");

    let local_turn = spool
        .list_turns_for_session("session-1", 10)
        .expect("local turns")
        .pop()
        .expect("one local turn");
    assert_eq!(local_turn.checkpoint_id.as_deref(), Some("cp-1"));

    let remote_turn = repository
        .list_turns_for_session("session-1", 10)
        .expect("remote turns")
        .pop()
        .expect("one remote turn");
    assert_eq!(remote_turn.checkpoint_id.as_deref(), Some("cp-1"));
}

#[test]
fn list_uncheckpointed_turns_excludes_assigned_turns() {
    let (_dir, spool) = test_spool();
    let turn = sample_turn();
    spool.record_turn(&turn).expect("record turn");
    assert_eq!(spool.list_uncheckpointed_turns().unwrap().len(), 1);
    spool
        .assign_checkpoint_to_turns(&["turn-1".to_string()], "cp-1", "2026-04-05T10:10:00Z")
        .expect("assign checkpoint");
    assert!(spool.list_uncheckpointed_turns().unwrap().is_empty());
}
