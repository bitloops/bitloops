use std::path::PathBuf;

use anyhow::{Context, Result};
use duckdb::OptionalExt;

use super::row_mapping::{append_event_filter_sql, map_event_row, map_session_row, map_turn_row};
use super::schema::ensure_current_schema;
use crate::host::devql::esc_pg;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventFilter, InteractionSession, InteractionTurn,
};

pub(crate) struct DuckDbInteractionRepository {
    pub(crate) repo_id: String,
    pub(crate) path: PathBuf,
}

impl DuckDbInteractionRepository {
    pub(crate) fn repo_id(&self) -> &str {
        &self.repo_id
    }

    pub(crate) fn ensure_schema(&self) -> Result<()> {
        let conn = self.open_or_create()?;
        ensure_current_schema(&conn).context("ensuring DuckDB interaction schema")?;
        Ok(())
    }

    fn open_or_create(&self) -> Result<duckdb::Connection> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating DuckDB directory {}", parent.display()))?;
        }
        duckdb::Connection::open(&self.path)
            .with_context(|| format!("opening DuckDB at {}", self.path.display()))
    }

    pub(crate) fn upsert_session(&self, session: &InteractionSession) -> Result<()> {
        super::super::ensure_repo_id(&self.repo_id, &session.repo_id, "interaction session")?;
        let conn = self.open_or_create()?;
        ensure_current_schema(&conn)?;
        let sql = format!(
            "INSERT OR REPLACE INTO interaction_sessions (
                session_id, repo_id, branch, actor_id, actor_name, actor_email, actor_source,
                agent_type, model, first_prompt, transcript_path, worktree_path, worktree_id,
                started_at, ended_at, last_event_at, updated_at
             ) VALUES (
                '{session_id}', '{repo_id}', '{branch}', '{actor_id}', '{actor_name}',
                '{actor_email}', '{actor_source}', '{agent_type}', '{model}', '{first_prompt}',
                '{transcript_path}', '{worktree_path}', '{worktree_id}', '{started_at}',
                {ended_at}, '{last_event_at}', '{updated_at}'
             )",
            session_id = esc_pg(&session.session_id),
            repo_id = esc_pg(&self.repo_id),
            branch = esc_pg(&session.branch),
            actor_id = esc_pg(&session.actor_id),
            actor_name = esc_pg(&session.actor_name),
            actor_email = esc_pg(&session.actor_email),
            actor_source = esc_pg(&session.actor_source),
            agent_type = esc_pg(&session.agent_type),
            model = esc_pg(&session.model),
            first_prompt = esc_pg(&session.first_prompt),
            transcript_path = esc_pg(&session.transcript_path),
            worktree_path = esc_pg(&session.worktree_path),
            worktree_id = esc_pg(&session.worktree_id),
            started_at = esc_pg(&session.started_at),
            ended_at = quoted_nullable(&session.ended_at),
            last_event_at = esc_pg(&session.last_event_at),
            updated_at = esc_pg(&session.updated_at),
        );
        conn.execute_batch(&sql)
            .context("upserting interaction session in DuckDB")?;
        Ok(())
    }

    pub(crate) fn upsert_turn(&self, turn: &InteractionTurn) -> Result<()> {
        super::super::ensure_repo_id(&self.repo_id, &turn.repo_id, "interaction turn")?;
        let conn = self.open_or_create()?;
        ensure_current_schema(&conn)?;
        let usage = turn.token_usage.clone().unwrap_or_default();
        let files_modified =
            serde_json::to_string(&turn.files_modified).context("serialising files_modified")?;
        let sql = format!(
            "INSERT OR REPLACE INTO interaction_turns (
                turn_id, session_id, repo_id, branch, actor_id, actor_name, actor_email,
                actor_source, turn_number, prompt, agent_type, model, started_at, ended_at,
                has_token_usage, input_tokens, cache_creation_tokens, cache_read_tokens,
                output_tokens, api_call_count, summary, prompt_count, transcript_offset_start,
                transcript_offset_end, transcript_fragment, files_modified, checkpoint_id,
                updated_at
             ) VALUES (
                '{turn_id}', '{session_id}', '{repo_id}', '{branch}', '{actor_id}',
                '{actor_name}', '{actor_email}', '{actor_source}', {turn_number}, '{prompt}',
                '{agent_type}', '{model}', '{started_at}', {ended_at}, {has_token_usage},
                {input_tokens}, {cache_creation_tokens}, {cache_read_tokens},
                {output_tokens}, {api_call_count}, '{summary}', {prompt_count},
                {transcript_offset_start}, {transcript_offset_end}, '{transcript_fragment}',
                '{files_modified}', '{checkpoint_id}', '{updated_at}'
             )",
            turn_id = esc_pg(&turn.turn_id),
            session_id = esc_pg(&turn.session_id),
            repo_id = esc_pg(&self.repo_id),
            branch = esc_pg(&turn.branch),
            actor_id = esc_pg(&turn.actor_id),
            actor_name = esc_pg(&turn.actor_name),
            actor_email = esc_pg(&turn.actor_email),
            actor_source = esc_pg(&turn.actor_source),
            turn_number = turn.turn_number,
            prompt = esc_pg(&turn.prompt),
            agent_type = esc_pg(&turn.agent_type),
            model = esc_pg(&turn.model),
            started_at = esc_pg(&turn.started_at),
            ended_at = quoted_nullable(&turn.ended_at),
            has_token_usage = i32::from(turn.token_usage.is_some()),
            input_tokens = usage.input_tokens,
            cache_creation_tokens = usage.cache_creation_tokens,
            cache_read_tokens = usage.cache_read_tokens,
            output_tokens = usage.output_tokens,
            api_call_count = usage.api_call_count,
            summary = esc_pg(&turn.summary),
            prompt_count = turn.prompt_count,
            transcript_offset_start = quoted_nullable_i64(turn.transcript_offset_start),
            transcript_offset_end = quoted_nullable_i64(turn.transcript_offset_end),
            transcript_fragment = esc_pg(&turn.transcript_fragment),
            files_modified = esc_pg(&files_modified),
            checkpoint_id = esc_pg(turn.checkpoint_id.as_deref().unwrap_or("")),
            updated_at = esc_pg(&turn.updated_at),
        );
        conn.execute_batch(&sql)
            .context("upserting interaction turn in DuckDB")?;
        Ok(())
    }

    pub(crate) fn append_event(&self, event: &InteractionEvent) -> Result<()> {
        super::super::ensure_repo_id(&self.repo_id, &event.repo_id, "interaction event")?;
        let conn = self.open_or_create()?;
        ensure_current_schema(&conn)?;
        let payload = serde_json::to_string(&event.payload).context("serialising event payload")?;
        let sql = format!(
            "INSERT OR IGNORE INTO interaction_events (
                event_id, event_time, repo_id, session_id, turn_id, branch, actor_id,
                actor_name, actor_email, actor_source, event_type, agent_type, model,
                tool_use_id, tool_kind, task_description, subagent_id, payload
             ) VALUES (
                '{event_id}', '{event_time}', '{repo_id}', '{session_id}', '{turn_id}',
                '{branch}', '{actor_id}', '{actor_name}', '{actor_email}', '{actor_source}',
                '{event_type}', '{agent_type}', '{model}', '{tool_use_id}', '{tool_kind}',
                '{task_description}', '{subagent_id}', '{payload}'
             )",
            event_id = esc_pg(&event.event_id),
            event_time = esc_pg(&event.event_time),
            repo_id = esc_pg(&self.repo_id),
            session_id = esc_pg(&event.session_id),
            turn_id = esc_pg(event.turn_id.as_deref().unwrap_or("")),
            branch = esc_pg(&event.branch),
            actor_id = esc_pg(&event.actor_id),
            actor_name = esc_pg(&event.actor_name),
            actor_email = esc_pg(&event.actor_email),
            actor_source = esc_pg(&event.actor_source),
            event_type = esc_pg(event.event_type.as_str()),
            agent_type = esc_pg(&event.agent_type),
            model = esc_pg(&event.model),
            tool_use_id = esc_pg(&event.tool_use_id),
            tool_kind = esc_pg(&event.tool_kind),
            task_description = esc_pg(&event.task_description),
            subagent_id = esc_pg(&event.subagent_id),
            payload = esc_pg(&payload),
        );
        conn.execute_batch(&sql)
            .context("appending interaction event in DuckDB")?;
        Ok(())
    }

    pub(crate) fn assign_checkpoint_to_turns(
        &self,
        turn_ids: &[String],
        checkpoint_id: &str,
        assigned_at: &str,
    ) -> Result<()> {
        if turn_ids.is_empty() {
            return Ok(());
        }
        let conn = self.open_or_create()?;
        ensure_current_schema(&conn)?;
        let quoted_ids = turn_ids
            .iter()
            .map(|turn_id| format!("'{}'", esc_pg(turn_id)))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "UPDATE interaction_turns
             SET checkpoint_id = '{checkpoint_id}', updated_at = '{assigned_at}'
             WHERE repo_id = '{repo_id}' AND turn_id IN ({quoted_ids})",
            checkpoint_id = esc_pg(checkpoint_id),
            assigned_at = esc_pg(assigned_at),
            repo_id = esc_pg(&self.repo_id),
        );
        conn.execute_batch(&sql)
            .context("assigning checkpoint ids in DuckDB")?;
        Ok(())
    }

    pub(crate) fn list_sessions(
        &self,
        agent: Option<&str>,
        limit: usize,
    ) -> Result<Vec<InteractionSession>> {
        let conn = self.open_or_create()?;
        ensure_current_schema(&conn)?;
        let mut sql = format!(
            "SELECT session_id, repo_id, branch, actor_id, actor_name, actor_email, actor_source,
                    agent_type, model, first_prompt, transcript_path, worktree_path, worktree_id,
                    started_at, ended_at, last_event_at, updated_at
             FROM interaction_sessions
             WHERE repo_id = '{repo_id}'",
            repo_id = esc_pg(&self.repo_id),
        );
        if let Some(agent) = agent.map(str::trim).filter(|value| !value.is_empty()) {
            sql.push_str(&format!(" AND agent_type = '{}'", esc_pg(agent)));
        }
        sql.push_str(
            " ORDER BY coalesce(nullif(last_event_at, ''), started_at) DESC, session_id DESC",
        );
        sql.push_str(&format!(" LIMIT {}", limit.max(1)));

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], map_session_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("reading interaction sessions from DuckDB")
    }

    pub(crate) fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>> {
        let conn = self.open_or_create()?;
        ensure_current_schema(&conn)?;
        let sql = format!(
            "SELECT session_id, repo_id, branch, actor_id, actor_name, actor_email, actor_source,
                    agent_type, model, first_prompt, transcript_path, worktree_path, worktree_id,
                    started_at, ended_at, last_event_at, updated_at
             FROM interaction_sessions
             WHERE repo_id = '{repo_id}' AND session_id = '{session_id}'
             LIMIT 1",
            repo_id = esc_pg(&self.repo_id),
            session_id = esc_pg(session_id),
        );
        let mut stmt = conn.prepare(&sql)?;
        stmt.query_row([], map_session_row)
            .optional()
            .map_err(anyhow::Error::from)
    }

    pub(crate) fn list_turns_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        let conn = self.open_or_create()?;
        ensure_current_schema(&conn)?;
        let sql = format!(
            "SELECT turn_id, session_id, repo_id, branch, actor_id, actor_name, actor_email,
                    actor_source, turn_number, prompt, agent_type, model, started_at, ended_at,
                    has_token_usage, input_tokens, cache_creation_tokens, cache_read_tokens,
                    output_tokens, api_call_count, summary, prompt_count,
                    transcript_offset_start, transcript_offset_end, transcript_fragment,
                    files_modified, checkpoint_id, updated_at
             FROM interaction_turns
             WHERE repo_id = '{repo_id}' AND session_id = '{session_id}'
             ORDER BY turn_number ASC, started_at ASC
             LIMIT {limit}",
            repo_id = esc_pg(&self.repo_id),
            session_id = esc_pg(session_id),
            limit = limit.max(1),
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], map_turn_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("reading interaction turns from DuckDB")
    }

    pub(crate) fn list_uncheckpointed_turns(&self) -> Result<Vec<InteractionTurn>> {
        let conn = self.open_or_create()?;
        ensure_current_schema(&conn)?;
        let sql = format!(
            "SELECT turn_id, session_id, repo_id, branch, actor_id, actor_name, actor_email,
                    actor_source, turn_number, prompt, agent_type, model, started_at, ended_at,
                    has_token_usage, input_tokens, cache_creation_tokens, cache_read_tokens,
                    output_tokens, api_call_count, summary, prompt_count,
                    transcript_offset_start, transcript_offset_end, transcript_fragment,
                    files_modified, checkpoint_id, updated_at
             FROM interaction_turns
             WHERE repo_id = '{repo_id}' AND checkpoint_id = ''
             ORDER BY session_id ASC, turn_number ASC, started_at ASC",
            repo_id = esc_pg(&self.repo_id),
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], map_turn_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("reading uncheckpointed interaction turns from DuckDB")
    }

    pub(crate) fn list_events(
        &self,
        filter: &InteractionEventFilter,
        limit: usize,
    ) -> Result<Vec<InteractionEvent>> {
        let conn = self.open_or_create()?;
        ensure_current_schema(&conn)?;
        let mut sql = format!(
            "SELECT event_id, session_id, turn_id, repo_id, branch, actor_id, actor_name,
                    actor_email, actor_source, event_type, event_time, agent_type, model,
                    tool_use_id, tool_kind, task_description, subagent_id, payload
             FROM interaction_events
             WHERE repo_id = '{repo_id}'",
            repo_id = esc_pg(&self.repo_id),
        );
        append_event_filter_sql(&mut sql, filter);
        sql.push_str(" ORDER BY event_time DESC, event_id DESC");
        sql.push_str(&format!(" LIMIT {}", limit.max(1)));

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], map_event_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("reading interaction events from DuckDB")
    }
}

fn quoted_nullable(value: &Option<String>) -> String {
    match value.as_deref().filter(|candidate| !candidate.is_empty()) {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    }
}

fn quoted_nullable_i64(value: Option<i64>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => "NULL".to_string(),
    }
}
