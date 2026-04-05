use anyhow::{Context, Result};

use super::store::InteractionEventStore;
use super::types::{InteractionEvent, InteractionSession, InteractionTurn};
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::storage::sqlite::SqliteConnectionPool;

/// SQLite-backed implementation of [`InteractionEventStore`].
pub struct SqliteInteractionEventStore {
    sqlite: SqliteConnectionPool,
    repo_id: String,
}

impl SqliteInteractionEventStore {
    pub fn new(sqlite: SqliteConnectionPool, repo_id: String) -> Self {
        Self { sqlite, repo_id }
    }

    pub fn repo_id(&self) -> &str {
        &self.repo_id
    }

    /// List interaction sessions, newest first.
    /// Optionally filter by agent_type and limit the number of results.
    pub fn list_sessions(
        &self,
        agent: Option<&str>,
        limit: usize,
    ) -> Result<Vec<InteractionSession>> {
        let agent = agent
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let limit = limit.max(1);

        self.sqlite.with_connection(|conn| {
            let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
                if let Some(ref agent) = agent {
                    (
                        format!(
                            "SELECT session_id, repo_id, agent_type, model, first_prompt,
                                transcript_path, worktree_path, worktree_id,
                                started_at, ended_at
                         FROM interaction_sessions
                         WHERE repo_id = ?1 AND agent_type = ?2
                         ORDER BY started_at DESC
                         LIMIT {limit}"
                        ),
                        vec![
                            Box::new(self.repo_id.clone()) as Box<dyn rusqlite::types::ToSql>,
                            Box::new(agent.clone()),
                        ],
                    )
                } else {
                    (
                        format!(
                            "SELECT session_id, repo_id, agent_type, model, first_prompt,
                                transcript_path, worktree_path, worktree_id,
                                started_at, ended_at
                         FROM interaction_sessions
                         WHERE repo_id = ?1
                         ORDER BY started_at DESC
                         LIMIT {limit}"
                        ),
                        vec![Box::new(self.repo_id.clone()) as Box<dyn rusqlite::types::ToSql>],
                    )
                };

            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok(InteractionSession {
                    session_id: row.get(0)?,
                    repo_id: row.get(1)?,
                    agent_type: row.get(2)?,
                    model: row.get(3)?,
                    first_prompt: row.get(4)?,
                    transcript_path: row.get(5)?,
                    worktree_path: row.get(6)?,
                    worktree_id: row.get(7)?,
                    started_at: row.get(8)?,
                    ended_at: row.get(9)?,
                })
            })?;
            rows.map(|r| r.context("reading interaction_sessions row"))
                .collect()
        })
    }
}

impl InteractionEventStore for SqliteInteractionEventStore {
    fn record_session(&self, session: &InteractionSession) -> Result<()> {
        if session.repo_id != self.repo_id {
            anyhow::bail!(
                "repo_id mismatch in record_session: expected '{}', got '{}'",
                self.repo_id,
                session.repo_id
            );
        }
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO interaction_sessions
                    (session_id, repo_id, agent_type, model, first_prompt,
                     transcript_path, worktree_path, worktree_id, started_at,
                     ended_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                         datetime('now'), datetime('now'))
                 ON CONFLICT (session_id) DO UPDATE SET
                    agent_type = excluded.agent_type,
                    model = excluded.model,
                    first_prompt = CASE WHEN interaction_sessions.first_prompt = ''
                                        THEN excluded.first_prompt
                                        ELSE interaction_sessions.first_prompt END,
                    transcript_path = excluded.transcript_path,
                    worktree_path = excluded.worktree_path,
                    worktree_id = excluded.worktree_id,
                    updated_at = datetime('now')",
                rusqlite::params![
                    session.session_id,
                    self.repo_id,
                    session.agent_type,
                    session.model,
                    session.first_prompt,
                    session.transcript_path,
                    session.worktree_path,
                    session.worktree_id,
                    session.started_at,
                    session.ended_at,
                ],
            )
            .context("upserting interaction_sessions row")?;
            Ok(())
        })
    }

    fn end_session(&self, session_id: &str, ended_at: &str) -> Result<()> {
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "UPDATE interaction_sessions
                 SET ended_at = ?2, updated_at = datetime('now')
                 WHERE session_id = ?1",
                rusqlite::params![session_id, ended_at],
            )
            .context("updating interaction_sessions ended_at")?;
            Ok(())
        })
    }

    fn record_turn_start(&self, turn: &InteractionTurn) -> Result<()> {
        if turn.repo_id != self.repo_id {
            anyhow::bail!(
                "repo_id mismatch in record_turn_start: expected '{}', got '{}'",
                self.repo_id,
                turn.repo_id
            );
        }
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO interaction_turns
                    (turn_id, session_id, repo_id, turn_number, prompt,
                     agent_type, model, started_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
                 ON CONFLICT (turn_id) DO UPDATE SET
                    prompt = excluded.prompt,
                    agent_type = excluded.agent_type,
                    model = excluded.model",
                rusqlite::params![
                    turn.turn_id,
                    turn.session_id,
                    self.repo_id,
                    turn.turn_number,
                    turn.prompt,
                    turn.agent_type,
                    turn.model,
                    turn.started_at,
                ],
            )
            .context("inserting interaction_turns row")?;
            Ok(())
        })
    }

    fn record_turn_end(
        &self,
        turn_id: &str,
        ended_at: &str,
        token_usage: Option<&TokenUsageMetadata>,
        files_modified: &[String],
    ) -> Result<()> {
        let token_json = token_usage
            .map(serde_json::to_string)
            .transpose()
            .context("serializing token_usage for interaction_turns")?;
        let files_json =
            serde_json::to_string(files_modified).context("serializing files_modified")?;

        self.sqlite.with_connection(|conn| {
            conn.execute(
                "UPDATE interaction_turns
                 SET ended_at = ?2, token_usage = ?3, files_modified = ?4
                 WHERE turn_id = ?1",
                rusqlite::params![turn_id, ended_at, token_json, files_json],
            )
            .context("updating interaction_turns at turn end")?;
            Ok(())
        })
    }

    fn record_event(&self, event: &InteractionEvent) -> Result<()> {
        if event.repo_id != self.repo_id {
            anyhow::bail!(
                "repo_id mismatch in record_event: expected '{}', got '{}'",
                self.repo_id,
                event.repo_id
            );
        }
        let payload_str =
            serde_json::to_string(&event.payload).context("serializing event payload")?;
        self.sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO interaction_events
                    (event_id, session_id, turn_id, repo_id, event_type,
                     event_time, agent_type, model, payload, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))
                 ON CONFLICT (event_id) DO NOTHING",
                rusqlite::params![
                    event.event_id,
                    event.session_id,
                    event.turn_id,
                    self.repo_id,
                    event.event_type.as_str(),
                    event.event_time,
                    event.agent_type,
                    event.model,
                    payload_str,
                ],
            )
            .context("inserting interaction_events row")?;
            Ok(())
        })
    }

    fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT session_id, repo_id, agent_type, model, first_prompt,
                        transcript_path, worktree_path, worktree_id,
                        started_at, ended_at
                 FROM interaction_sessions
                 WHERE session_id = ?1",
            )?;
            let mut rows = stmt.query(rusqlite::params![session_id])?;
            match rows.next()? {
                Some(row) => Ok(Some(InteractionSession {
                    session_id: row.get(0)?,
                    repo_id: row.get(1)?,
                    agent_type: row.get(2)?,
                    model: row.get(3)?,
                    first_prompt: row.get(4)?,
                    transcript_path: row.get(5)?,
                    worktree_path: row.get(6)?,
                    worktree_id: row.get(7)?,
                    started_at: row.get(8)?,
                    ended_at: row.get(9)?,
                })),
                None => Ok(None),
            }
        })
    }

    fn load_turns_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        let limit = limit.max(1);
        self.sqlite.with_connection(|conn| {
            let sql = format!(
                "SELECT turn_id, session_id, repo_id, turn_number, prompt,
                        agent_type, model, started_at, ended_at,
                        token_usage, files_modified, checkpoint_id
                 FROM interaction_turns
                 WHERE session_id = ?1
                 ORDER BY turn_number ASC
                 LIMIT {limit}"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params![session_id], |row| {
                Ok(TurnRow {
                    turn_id: row.get(0)?,
                    session_id: row.get(1)?,
                    repo_id: row.get(2)?,
                    turn_number: row.get(3)?,
                    prompt: row.get(4)?,
                    agent_type: row.get(5)?,
                    model: row.get(6)?,
                    started_at: row.get(7)?,
                    ended_at: row.get(8)?,
                    token_usage_json: row.get(9)?,
                    files_modified_json: row.get(10)?,
                    checkpoint_id: row.get(11)?,
                })
            })?;
            rows.map(|r| {
                let r = r.context("reading interaction_turns row")?;
                turn_row_to_interaction_turn(r)
            })
            .collect()
        })
    }

    fn pending_turns_for_session(&self, session_id: &str) -> Result<Vec<InteractionTurn>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT turn_id, session_id, repo_id, turn_number, prompt,
                        agent_type, model, started_at, ended_at,
                        token_usage, files_modified, checkpoint_id
                 FROM interaction_turns
                 WHERE session_id = ?1 AND checkpoint_id IS NULL
                 ORDER BY turn_number ASC",
            )?;
            let rows = stmt.query_map(rusqlite::params![session_id], |row| {
                Ok(TurnRow {
                    turn_id: row.get(0)?,
                    session_id: row.get(1)?,
                    repo_id: row.get(2)?,
                    turn_number: row.get(3)?,
                    prompt: row.get(4)?,
                    agent_type: row.get(5)?,
                    model: row.get(6)?,
                    started_at: row.get(7)?,
                    ended_at: row.get(8)?,
                    token_usage_json: row.get(9)?,
                    files_modified_json: row.get(10)?,
                    checkpoint_id: row.get(11)?,
                })
            })?;
            rows.map(|r| {
                let r = r.context("reading interaction_turns row")?;
                turn_row_to_interaction_turn(r)
            })
            .collect()
        })
    }

    fn assign_checkpoint_to_turns(&self, turn_ids: &[&str], checkpoint_id: &str) -> Result<()> {
        if turn_ids.is_empty() {
            return Ok(());
        }
        self.sqlite.with_connection(|conn| {
            let placeholders: Vec<String> = (1..=turn_ids.len()).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "UPDATE interaction_turns SET checkpoint_id = ?{} WHERE turn_id IN ({})",
                turn_ids.len() + 1,
                placeholders.join(", ")
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = turn_ids
                .iter()
                .map(|id| Box::new(id.to_string()) as Box<dyn rusqlite::types::ToSql>)
                .collect();
            params.push(Box::new(checkpoint_id.to_string()));
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            stmt.execute(param_refs.as_slice())
                .context("assigning checkpoint_id to interaction_turns")?;
            Ok(())
        })
    }
}

struct TurnRow {
    turn_id: String,
    session_id: String,
    repo_id: String,
    turn_number: i64,
    prompt: String,
    agent_type: String,
    model: String,
    started_at: String,
    ended_at: Option<String>,
    token_usage_json: Option<String>,
    files_modified_json: Option<String>,
    checkpoint_id: Option<String>,
}

fn turn_row_to_interaction_turn(r: TurnRow) -> Result<InteractionTurn> {
    let token_usage = r
        .token_usage_json
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(serde_json::from_str)
        .transpose()
        .context("deserializing token_usage from interaction_turns")?;
    let files_modified: Vec<String> = r
        .files_modified_json
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(serde_json::from_str)
        .transpose()
        .context("deserializing files_modified from interaction_turns")?
        .unwrap_or_default();
    let turn_number = u32::try_from(r.turn_number)
        .context("turn_number out of u32 range in interaction_turns")?;
    Ok(InteractionTurn {
        turn_id: r.turn_id,
        session_id: r.session_id,
        repo_id: r.repo_id,
        turn_number,
        prompt: r.prompt,
        agent_type: r.agent_type,
        model: r.model,
        started_at: r.started_at,
        ended_at: r.ended_at,
        token_usage,
        files_modified,
        checkpoint_id: r.checkpoint_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> (tempfile::TempDir, SqliteInteractionEventStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("test.db");
        let pool = SqliteConnectionPool::connect(db_path).expect("sqlite connect");
        pool.execute_batch(crate::host::devql::checkpoint_schema_sql_sqlite())
            .expect("init checkpoint schema");
        (
            dir,
            SqliteInteractionEventStore::new(pool, "test-repo".into()),
        )
    }

    #[test]
    fn record_and_load_session() {
        let (_dir, store) = test_store();
        let session = InteractionSession {
            session_id: "s1".into(),
            repo_id: "test-repo".into(),
            agent_type: "claude-code".into(),
            model: "claude-opus-4-6".into(),
            first_prompt: "hello".into(),
            started_at: "2026-04-04T10:00:00Z".into(),
            ..Default::default()
        };
        store.record_session(&session).expect("record session");
        let loaded = store.load_session("s1").expect("load").expect("found");
        assert_eq!(loaded.session_id, "s1");
        assert_eq!(loaded.agent_type, "claude-code");
        assert_eq!(loaded.first_prompt, "hello");
        assert!(loaded.ended_at.is_none());
    }

    #[test]
    fn record_session_upsert_preserves_first_prompt() {
        let (_dir, store) = test_store();
        let session = InteractionSession {
            session_id: "s1".into(),
            repo_id: "test-repo".into(),
            agent_type: "claude-code".into(),
            first_prompt: "original".into(),
            started_at: "2026-04-04T10:00:00Z".into(),
            ..Default::default()
        };
        store.record_session(&session).expect("first insert");

        let updated = InteractionSession {
            first_prompt: "new prompt".into(),
            model: "updated-model".into(),
            ..session.clone()
        };
        store.record_session(&updated).expect("upsert");

        let loaded = store.load_session("s1").expect("load").expect("found");
        assert_eq!(loaded.first_prompt, "original");
        assert_eq!(loaded.model, "updated-model");
    }

    #[test]
    fn end_session_sets_ended_at() {
        let (_dir, store) = test_store();
        store
            .record_session(&InteractionSession {
                session_id: "s1".into(),
                repo_id: "test-repo".into(),
                started_at: "2026-04-04T10:00:00Z".into(),
                ..Default::default()
            })
            .unwrap();
        store.end_session("s1", "2026-04-04T11:00:00Z").unwrap();
        let loaded = store.load_session("s1").unwrap().unwrap();
        assert_eq!(loaded.ended_at.as_deref(), Some("2026-04-04T11:00:00Z"));
    }

    #[test]
    fn record_turn_start_and_end_round_trip() {
        let (_dir, store) = test_store();
        store
            .record_session(&InteractionSession {
                session_id: "s1".into(),
                repo_id: "test-repo".into(),
                started_at: "2026-04-04T10:00:00Z".into(),
                ..Default::default()
            })
            .unwrap();

        let turn = InteractionTurn {
            turn_id: "t1".into(),
            session_id: "s1".into(),
            repo_id: "test-repo".into(),
            turn_number: 1,
            prompt: "fix bug".into(),
            agent_type: "claude-code".into(),
            started_at: "2026-04-04T10:01:00Z".into(),
            ..Default::default()
        };
        store.record_turn_start(&turn).unwrap();

        let usage = TokenUsageMetadata {
            input_tokens: 200,
            output_tokens: 100,
            cache_creation_tokens: 10,
            cache_read_tokens: 5,
            api_call_count: 3,
            subagent_tokens: None,
        };
        store
            .record_turn_end(
                "t1",
                "2026-04-04T10:02:00Z",
                Some(&usage),
                &["src/main.rs".to_string()],
            )
            .unwrap();

        let turns = store.load_turns_for_session("s1", 1000).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_id, "t1");
        assert_eq!(turns[0].ended_at.as_deref(), Some("2026-04-04T10:02:00Z"));
        let tu = turns[0].token_usage.as_ref().unwrap();
        assert_eq!(tu.input_tokens, 200);
        assert_eq!(tu.output_tokens, 100);
        assert_eq!(turns[0].files_modified, vec!["src/main.rs"]);
    }

    #[test]
    fn pending_turns_excludes_assigned() {
        let (_dir, store) = test_store();
        store
            .record_session(&InteractionSession {
                session_id: "s1".into(),
                repo_id: "test-repo".into(),
                started_at: "2026-04-04T10:00:00Z".into(),
                ..Default::default()
            })
            .unwrap();

        for i in 1..=3 {
            store
                .record_turn_start(&InteractionTurn {
                    turn_id: format!("t{i}"),
                    session_id: "s1".into(),
                    repo_id: "test-repo".into(),
                    turn_number: i,
                    started_at: format!("2026-04-04T10:0{i}:00Z"),
                    ..Default::default()
                })
                .unwrap();
        }

        store.assign_checkpoint_to_turns(&["t1"], "cp-1").unwrap();

        let pending = store.pending_turns_for_session("s1").unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].turn_id, "t2");
        assert_eq!(pending[1].turn_id, "t3");

        let all = store.load_turns_for_session("s1", 1000).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].checkpoint_id.as_deref(), Some("cp-1"));
        assert!(all[1].checkpoint_id.is_none());
    }

    #[test]
    fn record_event_and_idempotent() {
        let (_dir, store) = test_store();
        let event = InteractionEvent {
            event_id: "e1".into(),
            session_id: "s1".into(),
            turn_id: Some("t1".into()),
            repo_id: "test-repo".into(),
            event_type: super::super::types::InteractionEventType::TurnEnd,
            event_time: "2026-04-04T10:02:00Z".into(),
            agent_type: "claude-code".into(),
            model: "claude-opus-4-6".into(),
            payload: serde_json::json!({"input_tokens": 100}),
        };
        store.record_event(&event).expect("first insert");
        store.record_event(&event).expect("duplicate is no-op");
    }

    #[test]
    fn turns_ordered_by_turn_number() {
        let (_dir, store) = test_store();
        store
            .record_session(&InteractionSession {
                session_id: "s1".into(),
                repo_id: "test-repo".into(),
                started_at: "2026-04-04T10:00:00Z".into(),
                ..Default::default()
            })
            .unwrap();

        // Insert out of order
        for i in [3, 1, 2] {
            store
                .record_turn_start(&InteractionTurn {
                    turn_id: format!("t{i}"),
                    session_id: "s1".into(),
                    repo_id: "test-repo".into(),
                    turn_number: i,
                    started_at: format!("2026-04-04T10:0{i}:00Z"),
                    ..Default::default()
                })
                .unwrap();
        }

        let turns = store.load_turns_for_session("s1", 1000).unwrap();
        let numbers: Vec<u32> = turns.iter().map(|t| t.turn_number).collect();
        assert_eq!(numbers, vec![1, 2, 3]);
    }

    #[test]
    fn list_sessions_returns_newest_first() {
        let (_dir, store) = test_store();
        for (id, time) in [
            ("s1", "2026-04-04T08:00:00Z"),
            ("s2", "2026-04-04T10:00:00Z"),
        ] {
            store
                .record_session(&InteractionSession {
                    session_id: id.into(),
                    repo_id: "test-repo".into(),
                    agent_type: "claude-code".into(),
                    started_at: time.into(),
                    ..Default::default()
                })
                .unwrap();
        }
        let sessions = store.list_sessions(None, 100).unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, "s2", "newest first");
        assert_eq!(sessions[1].session_id, "s1");
    }

    #[test]
    fn list_sessions_filters_by_agent() {
        let (_dir, store) = test_store();
        store
            .record_session(&InteractionSession {
                session_id: "s1".into(),
                repo_id: "test-repo".into(),
                agent_type: "claude-code".into(),
                started_at: "2026-04-04T10:00:00Z".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .record_session(&InteractionSession {
                session_id: "s2".into(),
                repo_id: "test-repo".into(),
                agent_type: "cursor".into(),
                started_at: "2026-04-04T11:00:00Z".into(),
                ..Default::default()
            })
            .unwrap();
        let sessions = store.list_sessions(Some("cursor"), 100).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "s2");
    }

    #[test]
    fn list_sessions_respects_limit() {
        let (_dir, store) = test_store();
        for i in 1..=5 {
            store
                .record_session(&InteractionSession {
                    session_id: format!("s{i}"),
                    repo_id: "test-repo".into(),
                    agent_type: "claude-code".into(),
                    started_at: format!("2026-04-04T{:02}:00:00Z", 10 + i),
                    ..Default::default()
                })
                .unwrap();
        }
        let sessions = store.list_sessions(None, 2).unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn list_sessions_empty_when_no_sessions() {
        let (_dir, store) = test_store();
        let sessions = store.list_sessions(None, 100).unwrap();
        assert!(sessions.is_empty());
    }
}
