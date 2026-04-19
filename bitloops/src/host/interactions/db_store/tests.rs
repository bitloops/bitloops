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

#[test]
fn initialising_spool_migrates_legacy_event_schema_before_creating_indexes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sqlite =
        SqliteConnectionPool::connect(dir.path().join("interaction-spool.sqlite")).expect("sqlite");
    sqlite
        .execute_batch(
            r#"
CREATE TABLE interaction_sessions (
    session_id TEXT PRIMARY KEY
);
CREATE TABLE interaction_turns (
    turn_id TEXT PRIMARY KEY
);
CREATE TABLE interaction_events (
    event_id TEXT PRIMARY KEY
);
"#,
        )
        .expect("create legacy interaction tables");

    let spool =
        SqliteInteractionSpool::new(sqlite.clone(), "repo-test".into()).expect("initialise spool");

    spool
        .with_connection(|conn| {
            let mut stmt = conn
                .prepare("PRAGMA table_info(interaction_events)")
                .expect("prepare pragma");
            let mut rows = stmt.query([]).expect("query pragma");
            let mut saw_tool_use_id = false;
            while let Some(row) = rows.next().expect("next pragma row") {
                let column_name: String = row.get(1).expect("column name");
                if column_name == "tool_use_id" {
                    saw_tool_use_id = true;
                    break;
                }
            }
            assert!(saw_tool_use_id, "expected interaction_events.tool_use_id");

            let tool_use_index_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*)
                     FROM sqlite_master
                     WHERE type = 'index' AND name = 'interaction_events_tool_use_idx'",
                    [],
                    |row| row.get(0),
                )
                .expect("tool-use index count");
            assert_eq!(tool_use_index_count, 1);

            let tool_use_table_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*)
                     FROM sqlite_master
                     WHERE type = 'table' AND name = 'interaction_tool_uses'",
                    [],
                    |row| row.get(0),
                )
                .expect("tool-use table count");
            assert_eq!(tool_use_table_count, 1);

            Ok(())
        })
        .expect("inspect migrated spool schema");
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
        ..Default::default()
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

#[test]
fn recording_tool_events_refreshes_tool_use_and_search_projections() {
    let (_dir, spool) = test_spool();
    let mut session = sample_session();
    session.branch = "main".into();
    session.actor_email = "alice@example.com".into();
    let mut turn = sample_turn();
    turn.branch = "main".into();
    turn.actor_email = "alice@example.com".into();

    spool.record_session(&session).expect("record session");
    spool.record_turn(&turn).expect("record turn");
    spool
        .record_event(&InteractionEvent {
            event_id: "event-tool-start".into(),
            session_id: session.session_id.clone(),
            turn_id: Some(turn.turn_id.clone()),
            repo_id: spool.repo_id().to_string(),
            branch: "main".into(),
            actor_email: "alice@example.com".into(),
            event_type: InteractionEventType::SubagentStart,
            event_time: "2026-04-05T10:00:01Z".into(),
            agent_type: "codex".into(),
            model: "gpt-5.4".into(),
            tool_use_id: "tool-1".into(),
            tool_kind: "edit".into(),
            task_description: "Update src/main.rs".into(),
            subagent_id: "subagent-1".into(),
            payload: serde_json::json!({"phase": "start"}),
            ..Default::default()
        })
        .expect("record tool start");
    spool
        .record_event(&InteractionEvent {
            event_id: "event-tool-end".into(),
            session_id: session.session_id.clone(),
            turn_id: Some(turn.turn_id.clone()),
            repo_id: spool.repo_id().to_string(),
            branch: "main".into(),
            actor_email: "alice@example.com".into(),
            event_type: InteractionEventType::SubagentEnd,
            event_time: "2026-04-05T10:00:02Z".into(),
            agent_type: "codex".into(),
            model: "gpt-5.4".into(),
            tool_use_id: "tool-1".into(),
            tool_kind: "edit".into(),
            task_description: "Update src/main.rs".into(),
            subagent_id: "subagent-1".into(),
            payload: serde_json::json!({"phase": "end"}),
            ..Default::default()
        })
        .expect("record tool end");
    spool
        .assign_checkpoint_to_turns(&[turn.turn_id.clone()], "cp-1", "2026-04-05T10:00:03Z")
        .expect("assign checkpoint");

    spool
        .with_connection(|conn| {
            let tool_use: (String, String, String, Option<String>, Option<String>) = conn
                .query_row(
                    "SELECT tool_kind, task_description, session_id, started_at, ended_at
                     FROM interaction_tool_uses
                     WHERE repo_id = ?1 AND tool_use_id = 'tool-1'",
                    rusqlite::params![spool.repo_id()],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .expect("tool use row");
            assert_eq!(tool_use.0, "edit");
            assert_eq!(tool_use.1, "Update src/main.rs");
            assert_eq!(tool_use.2, "session-1");
            assert_eq!(tool_use.3.as_deref(), Some("2026-04-05T10:00:01Z"));
            assert_eq!(tool_use.4.as_deref(), Some("2026-04-05T10:00:02Z"));

            let turn_doc: (String, String, String) = conn
                .query_row(
                    "SELECT prompt_text, tool_text, paths_text
                     FROM interaction_turn_search_documents
                     WHERE repo_id = ?1 AND turn_id = 'turn-1'",
                    rusqlite::params![spool.repo_id()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("turn search document");
            assert!(turn_doc.0.contains("fix it"));
            assert!(turn_doc.1.contains("Update src/main.rs"));
            assert!(turn_doc.2.contains("src/main.rs"));

            let session_doc: String = conn
                .query_row(
                    "SELECT combined_text
                     FROM interaction_session_search_documents
                     WHERE repo_id = ?1 AND session_id = 'session-1'",
                    rusqlite::params![spool.repo_id()],
                    |row| row.get(0),
                )
                .expect("session search document");
            assert!(session_doc.contains("hello"));
            assert!(session_doc.contains("Update src/main.rs"));

            let tool_term_count: i64 = conn
                .query_row(
                    "SELECT occurrences
                     FROM interaction_turn_search_terms
                     WHERE repo_id = ?1 AND turn_id = 'turn-1' AND term = 'update' AND field = 'tool'",
                    rusqlite::params![spool.repo_id()],
                    |row| row.get(0),
                )
                .expect("tool term row");
            assert_eq!(tool_term_count, 1);

            Ok(())
        })
        .expect("query projections");
}
