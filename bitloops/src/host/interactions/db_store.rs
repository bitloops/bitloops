use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use super::store::{InteractionEventRepository, InteractionSpool};
use super::types::{
    InteractionEvent, InteractionEventFilter, InteractionEventType, InteractionMutation,
    InteractionSession, InteractionTurn,
};
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::storage::sqlite::SqliteConnectionPool;

const INTERACTION_SPOOL_FILE_NAME: &str = "interaction_spool.sqlite";

const INTERACTION_SPOOL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS interaction_sessions (
    session_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    agent_type TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    first_prompt TEXT NOT NULL DEFAULT '',
    transcript_path TEXT NOT NULL DEFAULT '',
    worktree_path TEXT NOT NULL DEFAULT '',
    worktree_id TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT '',
    ended_at TEXT,
    last_event_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS interaction_sessions_repo_idx
ON interaction_sessions (repo_id, last_event_at, started_at);

CREATE TABLE IF NOT EXISTS interaction_turns (
    turn_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    turn_number INTEGER NOT NULL DEFAULT 0,
    prompt TEXT NOT NULL DEFAULT '',
    agent_type TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT '',
    ended_at TEXT,
    has_token_usage INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    api_call_count INTEGER NOT NULL DEFAULT 0,
    files_modified TEXT NOT NULL DEFAULT '[]',
    checkpoint_id TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS interaction_turns_session_idx
ON interaction_turns (session_id, turn_number, started_at);

CREATE INDEX IF NOT EXISTS interaction_turns_pending_idx
ON interaction_turns (repo_id, checkpoint_id, session_id, turn_number);

CREATE TABLE IF NOT EXISTS interaction_events (
    event_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    turn_id TEXT,
    repo_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    event_time TEXT NOT NULL,
    agent_type TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    payload TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS interaction_events_repo_time_idx
ON interaction_events (repo_id, event_time, event_id);

CREATE INDEX IF NOT EXISTS interaction_events_session_idx
ON interaction_events (session_id, event_time, event_id);

CREATE TABLE IF NOT EXISTS interaction_spool_queue (
    mutation_id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id TEXT NOT NULL,
    mutation_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT NOT NULL DEFAULT '',
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS interaction_spool_queue_repo_idx
ON interaction_spool_queue (repo_id, mutation_id);
"#;

pub fn interaction_spool_db_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(".bitloops")
        .join("stores")
        .join("event")
        .join(INTERACTION_SPOOL_FILE_NAME)
}

pub struct SqliteInteractionSpool {
    sqlite: SqliteConnectionPool,
    repo_id: String,
}

impl SqliteInteractionSpool {
    pub fn new(sqlite: SqliteConnectionPool, repo_id: String) -> Result<Self> {
        sqlite
            .execute_batch(INTERACTION_SPOOL_SCHEMA)
            .context("initialising interaction spool schema")?;
        Ok(Self { sqlite, repo_id })
    }

    pub fn repo_id(&self) -> &str {
        &self.repo_id
    }

    fn enqueue_mutation(
        &self,
        conn: &rusqlite::Connection,
        mutation: &InteractionMutation,
    ) -> Result<()> {
        let payload =
            serde_json::to_string(mutation).context("serialising interaction mutation")?;
        let mutation_type = match mutation {
            InteractionMutation::UpsertSession { .. } => "upsert_session",
            InteractionMutation::UpsertTurn { .. } => "upsert_turn",
            InteractionMutation::AppendEvent { .. } => "append_event",
            InteractionMutation::AssignCheckpoint { .. } => "assign_checkpoint",
        };
        conn.execute(
            "INSERT INTO interaction_spool_queue
                (repo_id, mutation_type, payload, updated_at)
             VALUES (?1, ?2, ?3, datetime('now'))",
            rusqlite::params![self.repo_id, mutation_type, payload],
        )
        .context("enqueueing interaction mutation")?;
        Ok(())
    }

    fn upsert_local_session(
        &self,
        conn: &rusqlite::Connection,
        session: &InteractionSession,
    ) -> Result<()> {
        ensure_repo_id(&self.repo_id, &session.repo_id, "interaction session")?;
        conn.execute(
            "INSERT INTO interaction_sessions (
                session_id, repo_id, agent_type, model, first_prompt,
                transcript_path, worktree_path, worktree_id, started_at,
                ended_at, last_event_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9,
                ?10, ?11, ?12
             )
             ON CONFLICT(session_id) DO UPDATE SET
                repo_id = excluded.repo_id,
                agent_type = CASE
                    WHEN excluded.agent_type = '' THEN interaction_sessions.agent_type
                    ELSE excluded.agent_type
                END,
                model = CASE
                    WHEN excluded.model = '' THEN interaction_sessions.model
                    ELSE excluded.model
                END,
                first_prompt = CASE
                    WHEN excluded.first_prompt = '' THEN interaction_sessions.first_prompt
                    ELSE excluded.first_prompt
                END,
                transcript_path = CASE
                    WHEN excluded.transcript_path = '' THEN interaction_sessions.transcript_path
                    ELSE excluded.transcript_path
                END,
                worktree_path = CASE
                    WHEN excluded.worktree_path = '' THEN interaction_sessions.worktree_path
                    ELSE excluded.worktree_path
                END,
                worktree_id = CASE
                    WHEN excluded.worktree_id = '' THEN interaction_sessions.worktree_id
                    ELSE excluded.worktree_id
                END,
                started_at = CASE
                    WHEN excluded.started_at = '' THEN interaction_sessions.started_at
                    ELSE excluded.started_at
                END,
                ended_at = COALESCE(excluded.ended_at, interaction_sessions.ended_at),
                last_event_at = CASE
                    WHEN excluded.last_event_at = '' THEN interaction_sessions.last_event_at
                    ELSE excluded.last_event_at
                END,
                updated_at = CASE
                    WHEN excluded.updated_at = '' THEN interaction_sessions.updated_at
                    ELSE excluded.updated_at
                END",
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
                session.last_event_at,
                session.updated_at,
            ],
        )
        .context("upserting interaction session in local spool")?;
        Ok(())
    }

    fn upsert_local_turn(&self, conn: &rusqlite::Connection, turn: &InteractionTurn) -> Result<()> {
        ensure_repo_id(&self.repo_id, &turn.repo_id, "interaction turn")?;
        let usage = turn.token_usage.clone().unwrap_or_default();
        let has_token_usage = i64::from(turn.token_usage.is_some());
        let files_modified =
            serde_json::to_string(&turn.files_modified).context("serialising files_modified")?;
        let checkpoint_id = turn.checkpoint_id.clone().unwrap_or_default();
        conn.execute(
            "INSERT INTO interaction_turns (
                turn_id, session_id, repo_id, turn_number, prompt,
                agent_type, model, started_at, ended_at, has_token_usage,
                input_tokens, cache_creation_tokens, cache_read_tokens,
                output_tokens, api_call_count, files_modified, checkpoint_id, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18
             )
             ON CONFLICT(turn_id) DO UPDATE SET
                session_id = excluded.session_id,
                repo_id = excluded.repo_id,
                turn_number = CASE
                    WHEN excluded.turn_number = 0 THEN interaction_turns.turn_number
                    ELSE excluded.turn_number
                END,
                prompt = CASE
                    WHEN excluded.prompt = '' THEN interaction_turns.prompt
                    ELSE excluded.prompt
                END,
                agent_type = CASE
                    WHEN excluded.agent_type = '' THEN interaction_turns.agent_type
                    ELSE excluded.agent_type
                END,
                model = CASE
                    WHEN excluded.model = '' THEN interaction_turns.model
                    ELSE excluded.model
                END,
                started_at = CASE
                    WHEN excluded.started_at = '' THEN interaction_turns.started_at
                    ELSE excluded.started_at
                END,
                ended_at = COALESCE(excluded.ended_at, interaction_turns.ended_at),
                has_token_usage = CASE
                    WHEN excluded.has_token_usage = 1 THEN 1
                    ELSE interaction_turns.has_token_usage
                END,
                input_tokens = CASE
                    WHEN excluded.has_token_usage = 1 THEN excluded.input_tokens
                    ELSE interaction_turns.input_tokens
                END,
                cache_creation_tokens = CASE
                    WHEN excluded.has_token_usage = 1 THEN excluded.cache_creation_tokens
                    ELSE interaction_turns.cache_creation_tokens
                END,
                cache_read_tokens = CASE
                    WHEN excluded.has_token_usage = 1 THEN excluded.cache_read_tokens
                    ELSE interaction_turns.cache_read_tokens
                END,
                output_tokens = CASE
                    WHEN excluded.has_token_usage = 1 THEN excluded.output_tokens
                    ELSE interaction_turns.output_tokens
                END,
                api_call_count = CASE
                    WHEN excluded.has_token_usage = 1 THEN excluded.api_call_count
                    ELSE interaction_turns.api_call_count
                END,
                files_modified = CASE
                    WHEN excluded.files_modified = '[]' AND interaction_turns.files_modified <> '[]'
                        THEN interaction_turns.files_modified
                    ELSE excluded.files_modified
                END,
                checkpoint_id = CASE
                    WHEN excluded.checkpoint_id = '' THEN interaction_turns.checkpoint_id
                    ELSE excluded.checkpoint_id
                END,
                updated_at = CASE
                    WHEN excluded.updated_at = '' THEN interaction_turns.updated_at
                    ELSE excluded.updated_at
                END",
            rusqlite::params![
                turn.turn_id,
                turn.session_id,
                self.repo_id,
                i64::from(turn.turn_number),
                turn.prompt,
                turn.agent_type,
                turn.model,
                turn.started_at,
                turn.ended_at,
                has_token_usage,
                usage.input_tokens as i64,
                usage.cache_creation_tokens as i64,
                usage.cache_read_tokens as i64,
                usage.output_tokens as i64,
                usage.api_call_count as i64,
                files_modified,
                checkpoint_id,
                turn.updated_at,
            ],
        )
        .context("upserting interaction turn in local spool")?;
        Ok(())
    }

    fn insert_local_event(
        &self,
        conn: &rusqlite::Connection,
        event: &InteractionEvent,
    ) -> Result<()> {
        ensure_repo_id(&self.repo_id, &event.repo_id, "interaction event")?;
        let payload = serde_json::to_string(&event.payload).context("serialising event payload")?;
        conn.execute(
            "INSERT OR IGNORE INTO interaction_events (
                event_id, session_id, turn_id, repo_id, event_type,
                event_time, agent_type, model, payload
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                event.event_id,
                event.session_id,
                event.turn_id,
                self.repo_id,
                event.event_type.as_str(),
                event.event_time,
                event.agent_type,
                event.model,
                payload,
            ],
        )
        .context("inserting interaction event in local spool")?;
        Ok(())
    }
}

impl InteractionSpool for SqliteInteractionSpool {
    fn repo_id(&self) -> &str {
        &self.repo_id
    }

    fn record_session(&self, session: &InteractionSession) -> Result<()> {
        let mutation = InteractionMutation::UpsertSession {
            session: session.clone(),
        };
        self.sqlite.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")
                .context("starting interaction session spool transaction")?;
            let result = (|| -> Result<()> {
                self.upsert_local_session(conn, session)?;
                self.enqueue_mutation(conn, &mutation)?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    conn.execute_batch("COMMIT;")
                        .context("committing interaction session spool transaction")?;
                    Ok(())
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(err)
                }
            }
        })
    }

    fn record_turn(&self, turn: &InteractionTurn) -> Result<()> {
        let mutation = InteractionMutation::UpsertTurn { turn: turn.clone() };
        self.sqlite.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")
                .context("starting interaction turn spool transaction")?;
            let result = (|| -> Result<()> {
                self.upsert_local_turn(conn, turn)?;
                self.enqueue_mutation(conn, &mutation)?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    conn.execute_batch("COMMIT;")
                        .context("committing interaction turn spool transaction")?;
                    Ok(())
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(err)
                }
            }
        })
    }

    fn record_event(&self, event: &InteractionEvent) -> Result<()> {
        let mutation = InteractionMutation::AppendEvent {
            event: event.clone(),
        };
        self.sqlite.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")
                .context("starting interaction event spool transaction")?;
            let result = (|| -> Result<()> {
                self.insert_local_event(conn, event)?;
                self.enqueue_mutation(conn, &mutation)?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    conn.execute_batch("COMMIT;")
                        .context("committing interaction event spool transaction")?;
                    Ok(())
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(err)
                }
            }
        })
    }

    fn assign_checkpoint_to_turns(
        &self,
        turn_ids: &[String],
        checkpoint_id: &str,
        assigned_at: &str,
    ) -> Result<()> {
        if turn_ids.is_empty() {
            return Ok(());
        }
        let mutation = InteractionMutation::AssignCheckpoint {
            turn_ids: turn_ids.to_vec(),
            checkpoint_id: checkpoint_id.to_string(),
            assigned_at: assigned_at.to_string(),
        };
        self.sqlite.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")
                .context("starting checkpoint assignment spool transaction")?;
            let result = (|| -> Result<()> {
                let placeholders: Vec<String> = (1..=turn_ids.len())
                    .map(|idx| format!("?{}", idx + 2))
                    .collect();
                let sql = format!(
                    "UPDATE interaction_turns
                     SET checkpoint_id = ?1, updated_at = ?2
                     WHERE turn_id IN ({})",
                    placeholders.join(", ")
                );
                let mut params: Vec<&dyn rusqlite::types::ToSql> =
                    vec![&checkpoint_id, &assigned_at];
                for turn_id in turn_ids {
                    params.push(turn_id);
                }
                conn.execute(&sql, params.as_slice())
                    .context("updating local turn checkpoint ids")?;
                self.enqueue_mutation(conn, &mutation)?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    conn.execute_batch("COMMIT;")
                        .context("committing checkpoint assignment spool transaction")?;
                    Ok(())
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(err)
                }
            }
        })
    }

    fn flush(&self, repository: &dyn InteractionEventRepository) -> Result<usize> {
        ensure_repo_id(
            &self.repo_id,
            repository.repo_id(),
            "interaction repository",
        )?;
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT mutation_id, payload
                 FROM interaction_spool_queue
                 WHERE repo_id = ?1
                 ORDER BY mutation_id ASC",
            )?;
            let queue_rows = stmt
                .query_map(rusqlite::params![self.repo_id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()
                .context("reading interaction spool queue")?;

            let mut flushed = 0usize;
            for (mutation_id, payload) in queue_rows {
                let mutation: InteractionMutation =
                    serde_json::from_str(&payload).context("deserialising interaction mutation")?;
                let apply_result = match &mutation {
                    InteractionMutation::UpsertSession { session } => {
                        repository.upsert_session(session)
                    }
                    InteractionMutation::UpsertTurn { turn } => repository.upsert_turn(turn),
                    InteractionMutation::AppendEvent { event } => repository.append_event(event),
                    InteractionMutation::AssignCheckpoint {
                        turn_ids,
                        checkpoint_id,
                        assigned_at,
                    } => {
                        repository.assign_checkpoint_to_turns(turn_ids, checkpoint_id, assigned_at)
                    }
                };

                match apply_result {
                    Ok(()) => {
                        conn.execute(
                            "DELETE FROM interaction_spool_queue WHERE mutation_id = ?1",
                            rusqlite::params![mutation_id],
                        )
                        .context("deleting flushed interaction mutation")?;
                        flushed += 1;
                    }
                    Err(err) => {
                        conn.execute(
                            "UPDATE interaction_spool_queue
                             SET attempts = attempts + 1,
                                 last_error = ?2,
                                 updated_at = datetime('now')
                             WHERE mutation_id = ?1",
                            rusqlite::params![mutation_id, format!("{err:#}")],
                        )
                        .context("recording interaction spool flush failure")?;
                        return Err(err).with_context(|| {
                            format!("flushing interaction mutation {mutation_id}")
                        });
                    }
                }
            }
            Ok(flushed)
        })
    }

    fn list_sessions(&self, agent: Option<&str>, limit: usize) -> Result<Vec<InteractionSession>> {
        let limit = limit.max(1);
        self.sqlite.with_connection(|conn| {
            let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
                if let Some(agent) = agent.map(str::trim).filter(|value| !value.is_empty()) {
                    (
                        format!(
                            "SELECT session_id, repo_id, agent_type, model, first_prompt,
                                    transcript_path, worktree_path, worktree_id, started_at,
                                    ended_at, last_event_at, updated_at
                             FROM interaction_sessions
                             WHERE repo_id = ?1 AND agent_type = ?2
                             ORDER BY COALESCE(NULLIF(last_event_at, ''), started_at) DESC, session_id DESC
                             LIMIT {limit}"
                        ),
                        vec![Box::new(self.repo_id.clone()), Box::new(agent.to_string())],
                    )
                } else {
                    (
                        format!(
                            "SELECT session_id, repo_id, agent_type, model, first_prompt,
                                    transcript_path, worktree_path, worktree_id, started_at,
                                    ended_at, last_event_at, updated_at
                             FROM interaction_sessions
                             WHERE repo_id = ?1
                             ORDER BY COALESCE(NULLIF(last_event_at, ''), started_at) DESC, session_id DESC
                             LIMIT {limit}"
                        ),
                        vec![Box::new(self.repo_id.clone())],
                    )
                };

            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|value| value.as_ref()).collect();
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(param_refs.as_slice(), map_session_row)?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("reading interaction sessions from spool")
        })
    }

    fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>> {
        self.sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT session_id, repo_id, agent_type, model, first_prompt,
                        transcript_path, worktree_path, worktree_id, started_at,
                        ended_at, last_event_at, updated_at
                 FROM interaction_sessions
                 WHERE session_id = ?1 AND repo_id = ?2
                 LIMIT 1",
                rusqlite::params![session_id, self.repo_id],
                map_session_row,
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
    }

    fn list_turns_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        let limit = limit.max(1);
        self.sqlite.with_connection(|conn| {
            let sql = format!(
                "SELECT turn_id, session_id, repo_id, turn_number, prompt,
                        agent_type, model, started_at, ended_at, has_token_usage,
                        input_tokens, cache_creation_tokens, cache_read_tokens,
                        output_tokens, api_call_count, files_modified, checkpoint_id, updated_at
                 FROM interaction_turns
                 WHERE session_id = ?1 AND repo_id = ?2
                 ORDER BY turn_number ASC, started_at ASC
                 LIMIT {limit}"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params![session_id, self.repo_id], map_turn_row)?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("reading interaction turns from spool")
        })
    }

    fn list_uncheckpointed_turns(&self) -> Result<Vec<InteractionTurn>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT turn_id, session_id, repo_id, turn_number, prompt,
                        agent_type, model, started_at, ended_at, has_token_usage,
                        input_tokens, cache_creation_tokens, cache_read_tokens,
                        output_tokens, api_call_count, files_modified, checkpoint_id, updated_at
                 FROM interaction_turns
                 WHERE repo_id = ?1 AND checkpoint_id = ''
                 ORDER BY session_id ASC, turn_number ASC, started_at ASC",
            )?;
            let rows = stmt.query_map(rusqlite::params![self.repo_id], map_turn_row)?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("reading uncheckpointed turns from spool")
        })
    }

    fn list_events(
        &self,
        filter: &InteractionEventFilter,
        limit: usize,
    ) -> Result<Vec<InteractionEvent>> {
        let limit = limit.max(1);
        self.sqlite.with_connection(|conn| {
            let mut sql = String::from(
                "SELECT event_id, session_id, turn_id, repo_id, event_type,
                        event_time, agent_type, model, payload
                 FROM interaction_events
                 WHERE repo_id = ?1",
            );
            let mut values: Vec<Box<dyn rusqlite::types::ToSql>> =
                vec![Box::new(self.repo_id.clone())];
            append_event_filter_sql(&mut sql, &mut values, filter);
            sql.push_str(" ORDER BY event_time DESC, event_id DESC");
            sql.push_str(&format!(" LIMIT {limit}"));

            let params: Vec<&dyn rusqlite::types::ToSql> =
                values.iter().map(|value| value.as_ref()).collect();
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params.as_slice(), map_event_row)?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("reading interaction events from spool")
        })
    }
}

fn ensure_repo_id(expected: &str, actual: &str, entity: &str) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    anyhow::bail!("repo_id mismatch for {entity}: expected '{expected}', got '{actual}'");
}

fn append_event_filter_sql(
    sql: &mut String,
    values: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    filter: &InteractionEventFilter,
) {
    if let Some(session_id) = filter
        .session_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        sql.push_str(&format!(" AND session_id = ?{}", values.len() + 1));
        values.push(Box::new(session_id.to_string()));
    }
    if let Some(turn_id) = filter.turn_id.as_deref().filter(|value| !value.is_empty()) {
        sql.push_str(&format!(" AND turn_id = ?{}", values.len() + 1));
        values.push(Box::new(turn_id.to_string()));
    }
    if let Some(event_type) = filter.event_type {
        sql.push_str(&format!(" AND event_type = ?{}", values.len() + 1));
        values.push(Box::new(event_type.as_str().to_string()));
    }
    if let Some(since) = filter.since.as_deref().filter(|value| !value.is_empty()) {
        sql.push_str(&format!(" AND event_time >= ?{}", values.len() + 1));
        values.push(Box::new(since.to_string()));
    }
}

fn map_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InteractionSession> {
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
        last_event_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn map_turn_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InteractionTurn> {
    let has_token_usage: i64 = row.get(9)?;
    let files_modified_json: String = row.get(15)?;
    let files_modified =
        serde_json::from_str::<Vec<String>>(&files_modified_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                15,
                rusqlite::types::Type::Text,
                Box::new(err),
            )
        })?;
    let checkpoint_id: String = row.get(16)?;
    Ok(InteractionTurn {
        turn_id: row.get(0)?,
        session_id: row.get(1)?,
        repo_id: row.get(2)?,
        turn_number: u32::try_from(row.get::<_, i64>(3)?).unwrap_or_default(),
        prompt: row.get(4)?,
        agent_type: row.get(5)?,
        model: row.get(6)?,
        started_at: row.get(7)?,
        ended_at: row.get(8)?,
        token_usage: (has_token_usage == 1).then(|| TokenUsageMetadata {
            input_tokens: row.get::<_, i64>(10).unwrap_or_default().max(0) as u64,
            cache_creation_tokens: row.get::<_, i64>(11).unwrap_or_default().max(0) as u64,
            cache_read_tokens: row.get::<_, i64>(12).unwrap_or_default().max(0) as u64,
            output_tokens: row.get::<_, i64>(13).unwrap_or_default().max(0) as u64,
            api_call_count: row.get::<_, i64>(14).unwrap_or_default().max(0) as u64,
            subagent_tokens: None,
        }),
        files_modified,
        checkpoint_id: (!checkpoint_id.trim().is_empty()).then_some(checkpoint_id),
        updated_at: row.get(17)?,
    })
}

fn map_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InteractionEvent> {
    let payload_raw: String = row.get(8)?;
    let payload = serde_json::from_str::<serde_json::Value>(&payload_raw).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(err))
    })?;
    let event_type_raw: String = row.get(4)?;
    let event_type = InteractionEventType::parse(&event_type_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown interaction event type `{event_type_raw}`"),
            )),
        )
    })?;
    Ok(InteractionEvent {
        event_id: row.get(0)?,
        session_id: row.get(1)?,
        turn_id: row.get(2)?,
        repo_id: row.get(3)?,
        event_type,
        event_time: row.get(5)?,
        agent_type: row.get(6)?,
        model: row.get(7)?,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

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
        let sqlite = SqliteConnectionPool::connect(dir.path().join("interaction-spool.sqlite"))
            .expect("sqlite");
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
}
