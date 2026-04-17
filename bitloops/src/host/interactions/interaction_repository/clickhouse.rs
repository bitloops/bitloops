use anyhow::{Context, Result};

use super::clickhouse_client::{
    blocking_exec, blocking_query_rows, event_from_row, session_from_row, turn_from_row,
};
use crate::host::devql::esc_ch;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventFilter, InteractionSession, InteractionTurn,
};

pub(crate) struct ClickHouseInteractionRepository {
    pub(super) repo_id: String,
    pub(super) endpoint: String,
    pub(super) user: Option<String>,
    pub(super) password: Option<String>,
}

impl ClickHouseInteractionRepository {
    pub(super) fn repo_id(&self) -> &str {
        &self.repo_id
    }

    pub(super) fn ensure_schema(&self) -> Result<()> {
        for (name, sql) in SCHEMA_STATEMENTS {
            blocking_exec(
                &self.endpoint,
                self.user.as_deref(),
                self.password.as_deref(),
                sql,
            )
            .with_context(|| format!("ensuring ClickHouse interaction schema: {name}"))?;
        }
        for sql in SCHEMA_MIGRATIONS {
            blocking_exec(
                &self.endpoint,
                self.user.as_deref(),
                self.password.as_deref(),
                sql,
            )
            .with_context(|| format!("applying ClickHouse interaction schema migration: {sql}"))?;
        }
        Ok(())
    }

    pub(super) fn upsert_session(&self, session: &InteractionSession) -> Result<()> {
        super::ensure_repo_id(&self.repo_id, &session.repo_id, "interaction session")?;
        let sql = format!(
            "INSERT INTO interaction_sessions (
                session_id, repo_id, branch, actor_id, actor_name, actor_email, actor_source,
                agent_type, model, first_prompt,
                transcript_path, worktree_path, worktree_id, started_at,
                ended_at, last_event_at, updated_at
             ) VALUES (
                '{session_id}', '{repo_id}', '{branch}', '{actor_id}', '{actor_name}',
                '{actor_email}', '{actor_source}', '{agent_type}', '{model}', '{first_prompt}',
                '{transcript_path}', '{worktree_path}', '{worktree_id}', '{started_at}',
                '{ended_at}', '{last_event_at}',
                coalesce(parseDateTime64BestEffortOrNull('{updated_at}'), now64(3))
             )",
            session_id = esc_ch(&session.session_id),
            repo_id = esc_ch(&self.repo_id),
            branch = esc_ch(&session.branch),
            actor_id = esc_ch(&session.actor_id),
            actor_name = esc_ch(&session.actor_name),
            actor_email = esc_ch(&session.actor_email),
            actor_source = esc_ch(&session.actor_source),
            agent_type = esc_ch(&session.agent_type),
            model = esc_ch(&session.model),
            first_prompt = esc_ch(&session.first_prompt),
            transcript_path = esc_ch(&session.transcript_path),
            worktree_path = esc_ch(&session.worktree_path),
            worktree_id = esc_ch(&session.worktree_id),
            started_at = esc_ch(&session.started_at),
            ended_at = esc_ch(session.ended_at.as_deref().unwrap_or("")),
            last_event_at = esc_ch(&session.last_event_at),
            updated_at = esc_ch(&session.updated_at),
        );
        blocking_exec(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &sql,
        )
        .context("upserting interaction session in ClickHouse")?;
        Ok(())
    }

    pub(super) fn upsert_turn(&self, turn: &InteractionTurn) -> Result<()> {
        super::ensure_repo_id(&self.repo_id, &turn.repo_id, "interaction turn")?;
        let usage = turn.token_usage.clone().unwrap_or_default();
        let files_modified = format_array(&turn.files_modified);
        let sql = format!(
            "INSERT INTO interaction_turns (
                turn_id, session_id, repo_id, branch, actor_id, actor_name, actor_email,
                actor_source, turn_number, prompt,
                agent_type, model, started_at, ended_at, has_token_usage,
                input_tokens, cache_creation_tokens, cache_read_tokens,
                output_tokens, api_call_count, summary, prompt_count,
                transcript_offset_start, transcript_offset_end, transcript_fragment,
                files_modified, checkpoint_id, updated_at
             ) VALUES (
                '{turn_id}', '{session_id}', '{repo_id}', '{branch}', '{actor_id}',
                '{actor_name}', '{actor_email}', '{actor_source}', {turn_number}, '{prompt}',
                '{agent_type}', '{model}', '{started_at}', '{ended_at}', {has_token_usage},
                {input_tokens}, {cache_creation_tokens}, {cache_read_tokens},
                {output_tokens}, {api_call_count}, '{summary}', {prompt_count},
                {transcript_offset_start}, {transcript_offset_end}, '{transcript_fragment}',
                {files_modified}, '{checkpoint_id}',
                coalesce(parseDateTime64BestEffortOrNull('{updated_at}'), now64(3))
             )",
            turn_id = esc_ch(&turn.turn_id),
            session_id = esc_ch(&turn.session_id),
            repo_id = esc_ch(&self.repo_id),
            branch = esc_ch(&turn.branch),
            actor_id = esc_ch(&turn.actor_id),
            actor_name = esc_ch(&turn.actor_name),
            actor_email = esc_ch(&turn.actor_email),
            actor_source = esc_ch(&turn.actor_source),
            turn_number = turn.turn_number,
            prompt = esc_ch(&turn.prompt),
            agent_type = esc_ch(&turn.agent_type),
            model = esc_ch(&turn.model),
            started_at = esc_ch(&turn.started_at),
            ended_at = esc_ch(turn.ended_at.as_deref().unwrap_or("")),
            has_token_usage = i32::from(turn.token_usage.is_some()),
            input_tokens = usage.input_tokens,
            cache_creation_tokens = usage.cache_creation_tokens,
            cache_read_tokens = usage.cache_read_tokens,
            output_tokens = usage.output_tokens,
            api_call_count = usage.api_call_count,
            summary = esc_ch(&turn.summary),
            prompt_count = turn.prompt_count,
            transcript_offset_start = nullable_i64(turn.transcript_offset_start),
            transcript_offset_end = nullable_i64(turn.transcript_offset_end),
            transcript_fragment = esc_ch(&turn.transcript_fragment),
            files_modified = files_modified,
            checkpoint_id = esc_ch(turn.checkpoint_id.as_deref().unwrap_or("")),
            updated_at = esc_ch(&turn.updated_at),
        );
        blocking_exec(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &sql,
        )
        .context("upserting interaction turn in ClickHouse")?;
        Ok(())
    }

    pub(super) fn append_event(&self, event: &InteractionEvent) -> Result<()> {
        super::ensure_repo_id(&self.repo_id, &event.repo_id, "interaction event")?;
        let payload = serde_json::to_string(&event.payload).context("serialising event payload")?;
        let sql = format!(
            "INSERT INTO interaction_events (
                event_id, event_time, repo_id, session_id, turn_id, branch,
                actor_id, actor_name, actor_email, actor_source,
                event_type, agent_type, model, tool_use_id, tool_kind,
                task_description, subagent_id, payload
             ) VALUES (
                '{event_id}', coalesce(parseDateTime64BestEffortOrNull('{event_time}'), now64(3)),
                '{repo_id}', '{session_id}', '{turn_id}', '{branch}',
                '{actor_id}', '{actor_name}', '{actor_email}', '{actor_source}',
                '{event_type}', '{agent_type}', '{model}', '{tool_use_id}', '{tool_kind}',
                '{task_description}', '{subagent_id}', '{payload}'
             )",
            event_id = esc_ch(&event.event_id),
            event_time = esc_ch(&event.event_time),
            repo_id = esc_ch(&self.repo_id),
            session_id = esc_ch(&event.session_id),
            turn_id = esc_ch(event.turn_id.as_deref().unwrap_or("")),
            branch = esc_ch(&event.branch),
            actor_id = esc_ch(&event.actor_id),
            actor_name = esc_ch(&event.actor_name),
            actor_email = esc_ch(&event.actor_email),
            actor_source = esc_ch(&event.actor_source),
            event_type = esc_ch(event.event_type.as_str()),
            agent_type = esc_ch(&event.agent_type),
            model = esc_ch(&event.model),
            tool_use_id = esc_ch(&event.tool_use_id),
            tool_kind = esc_ch(&event.tool_kind),
            task_description = esc_ch(&event.task_description),
            subagent_id = esc_ch(&event.subagent_id),
            payload = esc_ch(&payload),
        );
        blocking_exec(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &sql,
        )
        .context("appending interaction event in ClickHouse")?;
        Ok(())
    }

    pub(super) fn assign_checkpoint_to_turns(
        &self,
        turn_ids: &[String],
        checkpoint_id: &str,
        assigned_at: &str,
    ) -> Result<()> {
        if turn_ids.is_empty() {
            return Ok(());
        }
        let turns = self.load_turns_by_ids(turn_ids)?;
        for mut turn in turns {
            turn.checkpoint_id = Some(checkpoint_id.to_string());
            turn.updated_at = assigned_at.to_string();
            self.upsert_turn(&turn)?;
        }
        Ok(())
    }

    pub(super) fn list_sessions(
        &self,
        agent: Option<&str>,
        limit: usize,
    ) -> Result<Vec<InteractionSession>> {
        let mut conditions = vec![format!("sessions.repo_id = '{}'", esc_ch(&self.repo_id))];
        if let Some(agent) = agent.map(str::trim).filter(|value| !value.is_empty()) {
            conditions.push(format!("sessions.agent_type = '{}'", esc_ch(agent)));
        }
        let sql = format!(
            "SELECT * FROM (
                SELECT
                    sessions.session_id AS session_id,
                    argMax(sessions.repo_id, sessions.updated_at) AS repo_id,
                    argMax(sessions.branch, sessions.updated_at) AS branch,
                    argMax(sessions.actor_id, sessions.updated_at) AS actor_id,
                    argMax(sessions.actor_name, sessions.updated_at) AS actor_name,
                    argMax(sessions.actor_email, sessions.updated_at) AS actor_email,
                    argMax(sessions.actor_source, sessions.updated_at) AS actor_source,
                    argMax(sessions.agent_type, sessions.updated_at) AS agent_type,
                    argMax(sessions.model, sessions.updated_at) AS model,
                    argMax(sessions.first_prompt, sessions.updated_at) AS first_prompt,
                    argMax(sessions.transcript_path, sessions.updated_at) AS transcript_path,
                    argMax(sessions.worktree_path, sessions.updated_at) AS worktree_path,
                    argMax(sessions.worktree_id, sessions.updated_at) AS worktree_id,
                    argMax(sessions.started_at, sessions.updated_at) AS started_at,
                    argMax(sessions.ended_at, sessions.updated_at) AS ended_at,
                    argMax(sessions.last_event_at, sessions.updated_at) AS last_event_at,
                    toString(max(sessions.updated_at)) AS updated_at
                FROM interaction_sessions AS sessions
                WHERE {}
                GROUP BY sessions.repo_id, sessions.session_id
            )
            ORDER BY parseDateTime64BestEffortOrZero(if(last_event_at = '', started_at, last_event_at)) DESC,
                     session_id DESC
            LIMIT {}",
            conditions.join(" AND "),
            limit.max(1),
        );
        let rows = blocking_query_rows(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &sql,
        )?;
        rows.iter().map(session_from_row).collect()
    }

    pub(super) fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>> {
        let rows = blocking_query_rows(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &format!(
                "SELECT * FROM (
                    SELECT
                        sessions.session_id AS session_id,
                        argMax(sessions.repo_id, sessions.updated_at) AS repo_id,
                        argMax(sessions.branch, sessions.updated_at) AS branch,
                        argMax(sessions.actor_id, sessions.updated_at) AS actor_id,
                        argMax(sessions.actor_name, sessions.updated_at) AS actor_name,
                        argMax(sessions.actor_email, sessions.updated_at) AS actor_email,
                        argMax(sessions.actor_source, sessions.updated_at) AS actor_source,
                        argMax(sessions.agent_type, sessions.updated_at) AS agent_type,
                        argMax(sessions.model, sessions.updated_at) AS model,
                        argMax(sessions.first_prompt, sessions.updated_at) AS first_prompt,
                        argMax(sessions.transcript_path, sessions.updated_at) AS transcript_path,
                        argMax(sessions.worktree_path, sessions.updated_at) AS worktree_path,
                        argMax(sessions.worktree_id, sessions.updated_at) AS worktree_id,
                        argMax(sessions.started_at, sessions.updated_at) AS started_at,
                        argMax(sessions.ended_at, sessions.updated_at) AS ended_at,
                        argMax(sessions.last_event_at, sessions.updated_at) AS last_event_at,
                        toString(max(sessions.updated_at)) AS updated_at
                    FROM interaction_sessions AS sessions
                    WHERE sessions.repo_id = '{repo_id}' AND sessions.session_id = '{session_id}'
                    GROUP BY sessions.repo_id, sessions.session_id
                )
                LIMIT 1",
                repo_id = esc_ch(&self.repo_id),
                session_id = esc_ch(session_id),
            ),
        )?;
        rows.first().map(session_from_row).transpose()
    }

    pub(super) fn list_turns_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        let rows = blocking_query_rows(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &format!(
                "SELECT
                        turns.turn_id AS turn_id,
                        argMax(turns.session_id, turns.updated_at) AS session_id,
                        argMax(turns.repo_id, turns.updated_at) AS repo_id,
                        argMax(turns.branch, turns.updated_at) AS branch,
                        argMax(turns.actor_id, turns.updated_at) AS actor_id,
                        argMax(turns.actor_name, turns.updated_at) AS actor_name,
                        argMax(turns.actor_email, turns.updated_at) AS actor_email,
                        argMax(turns.actor_source, turns.updated_at) AS actor_source,
                        argMax(turns.turn_number, turns.updated_at) AS turn_number,
                        argMax(turns.prompt, turns.updated_at) AS prompt,
                        argMax(turns.agent_type, turns.updated_at) AS agent_type,
                        argMax(turns.model, turns.updated_at) AS model,
                        argMax(turns.started_at, turns.updated_at) AS started_at,
                        argMax(turns.ended_at, turns.updated_at) AS ended_at,
                        argMax(turns.has_token_usage, turns.updated_at) AS has_token_usage,
                        argMax(turns.input_tokens, turns.updated_at) AS input_tokens,
                        argMax(turns.cache_creation_tokens, turns.updated_at) AS cache_creation_tokens,
                        argMax(turns.cache_read_tokens, turns.updated_at) AS cache_read_tokens,
                        argMax(turns.output_tokens, turns.updated_at) AS output_tokens,
                        argMax(turns.api_call_count, turns.updated_at) AS api_call_count,
                        argMax(turns.summary, turns.updated_at) AS summary,
                        argMax(turns.prompt_count, turns.updated_at) AS prompt_count,
                        argMax(turns.transcript_offset_start, turns.updated_at) AS transcript_offset_start,
                        argMax(turns.transcript_offset_end, turns.updated_at) AS transcript_offset_end,
                        argMax(turns.transcript_fragment, turns.updated_at) AS transcript_fragment,
                        argMax(turns.files_modified, turns.updated_at) AS files_modified,
                        argMax(turns.checkpoint_id, turns.updated_at) AS checkpoint_id,
                        toString(max(turns.updated_at)) AS updated_at
                    FROM interaction_turns AS turns
                    WHERE turns.repo_id = '{repo_id}' AND turns.session_id = '{session_id}'
                    GROUP BY turns.repo_id, turns.turn_id
                ORDER BY turn_number ASC, started_at ASC
                LIMIT {limit}",
                repo_id = esc_ch(&self.repo_id),
                session_id = esc_ch(session_id),
                limit = limit.max(1),
            ),
        )?;
        rows.iter().map(turn_from_row).collect()
    }

    pub(super) fn list_uncheckpointed_turns(&self) -> Result<Vec<InteractionTurn>> {
        let rows = blocking_query_rows(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &format!(
                "SELECT
                    turns.turn_id AS turn_id,
                    argMax(turns.session_id, turns.updated_at) AS session_id,
                    argMax(turns.repo_id, turns.updated_at) AS repo_id,
                    argMax(turns.branch, turns.updated_at) AS branch,
                    argMax(turns.actor_id, turns.updated_at) AS actor_id,
                    argMax(turns.actor_name, turns.updated_at) AS actor_name,
                    argMax(turns.actor_email, turns.updated_at) AS actor_email,
                    argMax(turns.actor_source, turns.updated_at) AS actor_source,
                    argMax(turns.turn_number, turns.updated_at) AS turn_number,
                    argMax(turns.prompt, turns.updated_at) AS prompt,
                    argMax(turns.agent_type, turns.updated_at) AS agent_type,
                    argMax(turns.model, turns.updated_at) AS model,
                    argMax(turns.started_at, turns.updated_at) AS started_at,
                    argMax(turns.ended_at, turns.updated_at) AS ended_at,
                    argMax(turns.has_token_usage, turns.updated_at) AS has_token_usage,
                    argMax(turns.input_tokens, turns.updated_at) AS input_tokens,
                    argMax(turns.cache_creation_tokens, turns.updated_at) AS cache_creation_tokens,
                    argMax(turns.cache_read_tokens, turns.updated_at) AS cache_read_tokens,
                    argMax(turns.output_tokens, turns.updated_at) AS output_tokens,
                    argMax(turns.api_call_count, turns.updated_at) AS api_call_count,
                    argMax(turns.summary, turns.updated_at) AS summary,
                    argMax(turns.prompt_count, turns.updated_at) AS prompt_count,
                    argMax(turns.transcript_offset_start, turns.updated_at) AS transcript_offset_start,
                    argMax(turns.transcript_offset_end, turns.updated_at) AS transcript_offset_end,
                    argMax(turns.transcript_fragment, turns.updated_at) AS transcript_fragment,
                    argMax(turns.files_modified, turns.updated_at) AS files_modified,
                    argMax(turns.checkpoint_id, turns.updated_at) AS checkpoint_id,
                    toString(max(turns.updated_at)) AS updated_at
                 FROM interaction_turns AS turns
                 WHERE turns.repo_id = '{repo_id}'
                 GROUP BY turns.repo_id, turns.turn_id
                 HAVING argMax(turns.checkpoint_id, turns.updated_at) = ''
                ORDER BY session_id ASC, turn_number ASC, started_at ASC",
                repo_id = esc_ch(&self.repo_id),
            ),
        )?;
        rows.iter().map(turn_from_row).collect()
    }

    pub(super) fn list_events(
        &self,
        filter: &InteractionEventFilter,
        limit: usize,
    ) -> Result<Vec<InteractionEvent>> {
        let mut conditions = vec![format!("repo_id = '{}'", esc_ch(&self.repo_id))];
        if let Some(session_id) = filter
            .session_id
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            conditions.push(format!("session_id = '{}'", esc_ch(session_id)));
        }
        if let Some(turn_id) = filter.turn_id.as_deref().filter(|value| !value.is_empty()) {
            conditions.push(format!("turn_id = '{}'", esc_ch(turn_id)));
        }
        if let Some(event_type) = filter.event_type {
            conditions.push(format!("event_type = '{}'", esc_ch(event_type.as_str())));
        }
        if let Some(since) = filter.since.as_deref().filter(|value| !value.is_empty()) {
            conditions.push(format!(
                "event_time >= parseDateTime64BestEffortOrZero('{}')",
                esc_ch(since)
            ));
        }
        let rows = blocking_query_rows(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &format!(
                "SELECT event_id, session_id, turn_id, repo_id, branch,
                        actor_id, actor_name, actor_email, actor_source, event_type,
                        toString(event_time) AS event_time, agent_type, model, tool_use_id,
                        tool_kind, task_description, subagent_id, payload
                 FROM interaction_events
                 WHERE {}
                 ORDER BY event_time DESC, event_id DESC
                 LIMIT {}",
                conditions.join(" AND "),
                limit.max(1),
            ),
        )?;
        rows.iter().map(event_from_row).collect()
    }

    fn load_turns_by_ids(&self, turn_ids: &[String]) -> Result<Vec<InteractionTurn>> {
        let ids = turn_ids
            .iter()
            .map(|turn_id| format!("'{}'", esc_ch(turn_id)))
            .collect::<Vec<_>>()
            .join(", ");
        let rows = blocking_query_rows(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &format!(
                "SELECT * FROM (
                    SELECT
                        turns.turn_id AS turn_id,
                        argMax(turns.session_id, turns.updated_at) AS session_id,
                        argMax(turns.repo_id, turns.updated_at) AS repo_id,
                        argMax(turns.branch, turns.updated_at) AS branch,
                        argMax(turns.actor_id, turns.updated_at) AS actor_id,
                        argMax(turns.actor_name, turns.updated_at) AS actor_name,
                        argMax(turns.actor_email, turns.updated_at) AS actor_email,
                        argMax(turns.actor_source, turns.updated_at) AS actor_source,
                        argMax(turns.turn_number, turns.updated_at) AS turn_number,
                        argMax(turns.prompt, turns.updated_at) AS prompt,
                        argMax(turns.agent_type, turns.updated_at) AS agent_type,
                        argMax(turns.model, turns.updated_at) AS model,
                        argMax(turns.started_at, turns.updated_at) AS started_at,
                        argMax(turns.ended_at, turns.updated_at) AS ended_at,
                        argMax(turns.has_token_usage, turns.updated_at) AS has_token_usage,
                        argMax(turns.input_tokens, turns.updated_at) AS input_tokens,
                        argMax(turns.cache_creation_tokens, turns.updated_at) AS cache_creation_tokens,
                        argMax(turns.cache_read_tokens, turns.updated_at) AS cache_read_tokens,
                        argMax(turns.output_tokens, turns.updated_at) AS output_tokens,
                        argMax(turns.api_call_count, turns.updated_at) AS api_call_count,
                        argMax(turns.summary, turns.updated_at) AS summary,
                        argMax(turns.prompt_count, turns.updated_at) AS prompt_count,
                        argMax(turns.transcript_offset_start, turns.updated_at) AS transcript_offset_start,
                        argMax(turns.transcript_offset_end, turns.updated_at) AS transcript_offset_end,
                        argMax(turns.transcript_fragment, turns.updated_at) AS transcript_fragment,
                        argMax(turns.files_modified, turns.updated_at) AS files_modified,
                        argMax(turns.checkpoint_id, turns.updated_at) AS checkpoint_id,
                        toString(max(turns.updated_at)) AS updated_at
                    FROM interaction_turns AS turns
                    WHERE turns.repo_id = '{repo_id}' AND turns.turn_id IN ({ids})
                    GROUP BY turns.repo_id, turns.turn_id
                )",
                repo_id = esc_ch(&self.repo_id),
                ids = ids,
            ),
        )?;
        rows.iter().map(turn_from_row).collect()
    }
}

const INTERACTION_SESSIONS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS interaction_sessions (
    session_id String,
    repo_id String,
    branch String,
    actor_id String,
    actor_name String,
    actor_email String,
    actor_source String,
    agent_type String,
    model String,
    first_prompt String,
    transcript_path String,
    worktree_path String,
    worktree_id String,
    started_at String,
    ended_at String,
    last_event_at String,
    updated_at DateTime64(3, 'UTC')
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (repo_id, session_id)
"#;

const INTERACTION_TURNS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS interaction_turns (
    turn_id String,
    session_id String,
    repo_id String,
    branch String,
    actor_id String,
    actor_name String,
    actor_email String,
    actor_source String,
    turn_number UInt32,
    prompt String,
    agent_type String,
    model String,
    started_at String,
    ended_at String,
    has_token_usage UInt8,
    input_tokens UInt64,
    cache_creation_tokens UInt64,
    cache_read_tokens UInt64,
    output_tokens UInt64,
    api_call_count UInt64,
    summary String,
    prompt_count UInt32,
    transcript_offset_start Nullable(Int64),
    transcript_offset_end Nullable(Int64),
    transcript_fragment String,
    files_modified Array(String),
    checkpoint_id String,
    updated_at DateTime64(3, 'UTC')
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (repo_id, session_id, turn_id)
"#;

const INTERACTION_EVENTS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS interaction_events (
    event_id String,
    event_time DateTime64(3, 'UTC'),
    repo_id String,
    session_id String,
    turn_id String,
    branch String,
    actor_id String,
    actor_name String,
    actor_email String,
    actor_source String,
    event_type String,
    agent_type String,
    model String,
    tool_use_id String,
    tool_kind String,
    task_description String,
    subagent_id String,
    payload String
)
ENGINE = ReplacingMergeTree(event_time)
ORDER BY (repo_id, event_time, event_id);
"#;

const SCHEMA_STATEMENTS: &[(&str, &str)] = &[
    ("interaction_sessions", INTERACTION_SESSIONS_SCHEMA),
    ("interaction_turns", INTERACTION_TURNS_SCHEMA),
    ("interaction_events", INTERACTION_EVENTS_SCHEMA),
];

const SCHEMA_MIGRATIONS: &[&str] = &[
    "ALTER TABLE interaction_sessions ADD COLUMN IF NOT EXISTS branch String AFTER repo_id",
    "ALTER TABLE interaction_sessions ADD COLUMN IF NOT EXISTS actor_id String AFTER branch",
    "ALTER TABLE interaction_sessions ADD COLUMN IF NOT EXISTS actor_name String AFTER actor_id",
    "ALTER TABLE interaction_sessions ADD COLUMN IF NOT EXISTS actor_email String AFTER actor_name",
    "ALTER TABLE interaction_sessions ADD COLUMN IF NOT EXISTS actor_source String AFTER actor_email",
    "ALTER TABLE interaction_turns ADD COLUMN IF NOT EXISTS branch String AFTER repo_id",
    "ALTER TABLE interaction_turns ADD COLUMN IF NOT EXISTS actor_id String AFTER branch",
    "ALTER TABLE interaction_turns ADD COLUMN IF NOT EXISTS actor_name String AFTER actor_id",
    "ALTER TABLE interaction_turns ADD COLUMN IF NOT EXISTS actor_email String AFTER actor_name",
    "ALTER TABLE interaction_turns ADD COLUMN IF NOT EXISTS actor_source String AFTER actor_email",
    "ALTER TABLE interaction_turns ADD COLUMN IF NOT EXISTS summary String AFTER api_call_count",
    "ALTER TABLE interaction_turns ADD COLUMN IF NOT EXISTS prompt_count UInt32 AFTER summary",
    "ALTER TABLE interaction_turns ADD COLUMN IF NOT EXISTS transcript_offset_start Nullable(Int64) AFTER prompt_count",
    "ALTER TABLE interaction_turns ADD COLUMN IF NOT EXISTS transcript_offset_end Nullable(Int64) AFTER transcript_offset_start",
    "ALTER TABLE interaction_turns ADD COLUMN IF NOT EXISTS transcript_fragment String AFTER transcript_offset_end",
    "ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS branch String AFTER turn_id",
    "ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS actor_id String AFTER branch",
    "ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS actor_name String AFTER actor_id",
    "ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS actor_email String AFTER actor_name",
    "ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS actor_source String AFTER actor_email",
    "ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS tool_use_id String AFTER model",
    "ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS tool_kind String AFTER tool_use_id",
    "ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS task_description String AFTER tool_kind",
    "ALTER TABLE interaction_events ADD COLUMN IF NOT EXISTS subagent_id String AFTER task_description",
];

fn format_array(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| format!("'{}'", esc_ch(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
}

fn nullable_i64(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
    use crate::host::interactions::types::{
        InteractionEvent, InteractionEventType, InteractionSession, InteractionTurn,
    };

    #[test]
    #[ignore = "requires a running ClickHouse instance configured via BITLOOPS_TEST_CLICKHOUSE_URL"]
    fn round_trip_is_env_gated() {
        let Some(url) = std::env::var("BITLOOPS_TEST_CLICKHOUSE_URL").ok() else {
            return;
        };
        let repository = ClickHouseInteractionRepository {
            repo_id: "repo-test".into(),
            endpoint: format!("{}/?database=default", url.trim_end_matches('/')),
            user: std::env::var("BITLOOPS_TEST_CLICKHOUSE_USER").ok(),
            password: std::env::var("BITLOOPS_TEST_CLICKHOUSE_PASSWORD").ok(),
        };
        repository.ensure_schema().expect("schema");

        let session = InteractionSession {
            session_id: "sess-1".into(),
            repo_id: "repo-test".into(),
            agent_type: "codex".into(),
            model: "gpt-5.4".into(),
            first_prompt: "hello".into(),
            transcript_path: "/tmp/transcript.jsonl".into(),
            worktree_path: "/tmp/repo".into(),
            worktree_id: "main".into(),
            started_at: "2026-04-05T10:00:00Z".into(),
            last_event_at: "2026-04-05T10:00:01Z".into(),
            updated_at: "2026-04-05T10:00:01Z".into(),
            ..Default::default()
        };
        repository.upsert_session(&session).expect("upsert session");

        let turn = InteractionTurn {
            turn_id: "turn-1".into(),
            session_id: "sess-1".into(),
            repo_id: "repo-test".into(),
            turn_number: 1,
            prompt: "ship it".into(),
            agent_type: "codex".into(),
            model: "gpt-5.4".into(),
            started_at: "2026-04-05T10:00:01Z".into(),
            ended_at: Some("2026-04-05T10:00:02Z".into()),
            token_usage: Some(TokenUsageMetadata {
                input_tokens: 11,
                output_tokens: 7,
                ..Default::default()
            }),
            summary: "completed main change".into(),
            prompt_count: 2,
            transcript_offset_start: Some(1),
            transcript_offset_end: Some(3),
            transcript_fragment: "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n".into(),
            files_modified: vec!["src/main.rs".into()],
            updated_at: "2026-04-05T10:00:02Z".into(),
            ..Default::default()
        };
        repository.upsert_turn(&turn).expect("upsert turn");

        let event = InteractionEvent {
            event_id: "evt-1".into(),
            session_id: "sess-1".into(),
            turn_id: Some("turn-1".into()),
            repo_id: "repo-test".into(),
            event_type: InteractionEventType::TurnEnd,
            event_time: "2026-04-05T10:00:02Z".into(),
            agent_type: "codex".into(),
            model: "gpt-5.4".into(),
            payload: serde_json::json!({"token_usage": {"input_tokens": 11}}),
            ..Default::default()
        };
        repository.append_event(&event).expect("append event");
        assert!(!repository.list_sessions(None, 10).unwrap().is_empty());
    }
}
