use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use super::SqliteInteractionSpool;
use super::row_mapping::{append_event_filter_sql, map_event_row, map_session_row, map_turn_row};
use crate::host::interactions::store::{InteractionEventRepository, InteractionSpool};
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventFilter, InteractionMutation, InteractionSession,
    InteractionTurn,
};

impl SqliteInteractionSpool {
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
        super::ensure_repo_id(&self.repo_id, &session.repo_id, "interaction session")?;
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
             ON CONFLICT(repo_id, session_id) DO UPDATE SET
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
        super::ensure_repo_id(&self.repo_id, &turn.repo_id, "interaction turn")?;
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
                output_tokens, api_call_count, summary, prompt_count,
                transcript_offset_start, transcript_offset_end, transcript_fragment,
                files_modified, checkpoint_id, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13,
                ?14, ?15, ?16, ?17,
                ?18, ?19, ?20, ?21, ?22, ?23
             )
             ON CONFLICT(repo_id, turn_id) DO UPDATE SET
                session_id = excluded.session_id,
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
                summary = CASE
                    WHEN excluded.summary = '' THEN interaction_turns.summary
                    ELSE excluded.summary
                END,
                prompt_count = CASE
                    WHEN excluded.prompt_count = 0 THEN interaction_turns.prompt_count
                    ELSE excluded.prompt_count
                END,
                transcript_offset_start =
                    COALESCE(excluded.transcript_offset_start, interaction_turns.transcript_offset_start),
                transcript_offset_end =
                    COALESCE(excluded.transcript_offset_end, interaction_turns.transcript_offset_end),
                transcript_fragment = CASE
                    WHEN excluded.transcript_fragment = '' THEN interaction_turns.transcript_fragment
                    ELSE excluded.transcript_fragment
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
                turn.summary,
                i64::from(turn.prompt_count),
                turn.transcript_offset_start,
                turn.transcript_offset_end,
                turn.transcript_fragment,
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
        super::ensure_repo_id(&self.repo_id, &event.repo_id, "interaction event")?;
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
                    .map(|idx| format!("?{}", idx + 3))
                    .collect();
                let sql = format!(
                    "UPDATE interaction_turns
                     SET checkpoint_id = ?1, updated_at = ?2
                     WHERE repo_id = ?3 AND turn_id IN ({})",
                    placeholders.join(", ")
                );
                let mut params: Vec<&dyn rusqlite::types::ToSql> =
                    vec![&checkpoint_id, &assigned_at, &self.repo_id];
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

    fn has_pending_mutations(&self) -> Result<bool> {
        self.sqlite.with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*)
                 FROM interaction_spool_queue
                 WHERE repo_id = ?1",
                rusqlite::params![self.repo_id],
                |row| row.get(0),
            )?;
            Ok(count > 0)
        })
    }

    fn flush(&self, repository: &dyn InteractionEventRepository) -> Result<usize> {
        super::ensure_repo_id(
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
                        output_tokens, api_call_count, summary, prompt_count,
                        transcript_offset_start, transcript_offset_end, transcript_fragment,
                        files_modified, checkpoint_id, updated_at
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
                        output_tokens, api_call_count, summary, prompt_count,
                        transcript_offset_start, transcript_offset_end, transcript_fragment,
                        files_modified, checkpoint_id, updated_at
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
