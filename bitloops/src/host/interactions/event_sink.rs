use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use duckdb::OptionalExt;
use serde_json::Value;

use super::store::InteractionEventRepository;
use super::types::{
    InteractionEvent, InteractionEventFilter, InteractionEventType, InteractionSession,
    InteractionTurn,
};
use crate::config::EventsBackendConfig;
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::devql::{esc_ch, esc_pg};

pub fn create_event_repository(
    events_cfg: &EventsBackendConfig,
    repo_root: &Path,
    repo_id: String,
) -> Result<EventDbInteractionRepository> {
    if events_cfg.has_clickhouse() {
        let repository = ClickHouseInteractionRepository {
            repo_id,
            endpoint: events_cfg.clickhouse_endpoint(),
            user: events_cfg.clickhouse_user.clone(),
            password: events_cfg.clickhouse_password.clone(),
        };
        repository.ensure_schema()?;
        return Ok(EventDbInteractionRepository::ClickHouse(repository));
    }

    let repository = DuckDbInteractionRepository {
        repo_id,
        path: events_cfg.resolve_duckdb_db_path_for_repo(repo_root),
    };
    repository.ensure_schema()?;
    Ok(EventDbInteractionRepository::DuckDb(repository))
}

pub enum EventDbInteractionRepository {
    DuckDb(DuckDbInteractionRepository),
    ClickHouse(ClickHouseInteractionRepository),
}

impl InteractionEventRepository for EventDbInteractionRepository {
    fn repo_id(&self) -> &str {
        match self {
            Self::DuckDb(repository) => repository.repo_id(),
            Self::ClickHouse(repository) => repository.repo_id(),
        }
    }

    fn upsert_session(&self, session: &InteractionSession) -> Result<()> {
        match self {
            Self::DuckDb(repository) => repository.upsert_session(session),
            Self::ClickHouse(repository) => repository.upsert_session(session),
        }
    }

    fn upsert_turn(&self, turn: &InteractionTurn) -> Result<()> {
        match self {
            Self::DuckDb(repository) => repository.upsert_turn(turn),
            Self::ClickHouse(repository) => repository.upsert_turn(turn),
        }
    }

    fn append_event(&self, event: &InteractionEvent) -> Result<()> {
        match self {
            Self::DuckDb(repository) => repository.append_event(event),
            Self::ClickHouse(repository) => repository.append_event(event),
        }
    }

    fn assign_checkpoint_to_turns(
        &self,
        turn_ids: &[String],
        checkpoint_id: &str,
        assigned_at: &str,
    ) -> Result<()> {
        match self {
            Self::DuckDb(repository) => {
                repository.assign_checkpoint_to_turns(turn_ids, checkpoint_id, assigned_at)
            }
            Self::ClickHouse(repository) => {
                repository.assign_checkpoint_to_turns(turn_ids, checkpoint_id, assigned_at)
            }
        }
    }

    fn list_sessions(&self, agent: Option<&str>, limit: usize) -> Result<Vec<InteractionSession>> {
        match self {
            Self::DuckDb(repository) => repository.list_sessions(agent, limit),
            Self::ClickHouse(repository) => repository.list_sessions(agent, limit),
        }
    }

    fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>> {
        match self {
            Self::DuckDb(repository) => repository.load_session(session_id),
            Self::ClickHouse(repository) => repository.load_session(session_id),
        }
    }

    fn list_turns_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        match self {
            Self::DuckDb(repository) => repository.list_turns_for_session(session_id, limit),
            Self::ClickHouse(repository) => repository.list_turns_for_session(session_id, limit),
        }
    }

    fn list_events(
        &self,
        filter: &InteractionEventFilter,
        limit: usize,
    ) -> Result<Vec<InteractionEvent>> {
        match self {
            Self::DuckDb(repository) => repository.list_events(filter, limit),
            Self::ClickHouse(repository) => repository.list_events(filter, limit),
        }
    }
}

pub(crate) struct DuckDbInteractionRepository {
    repo_id: String,
    path: PathBuf,
}

impl DuckDbInteractionRepository {
    fn repo_id(&self) -> &str {
        &self.repo_id
    }

    fn ensure_schema(&self) -> Result<()> {
        let conn = self.open_or_create()?;
        conn.execute_batch(DUCKDB_INTERACTION_SCHEMA)
            .context("ensuring DuckDB interaction schema")?;
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

    fn upsert_session(&self, session: &InteractionSession) -> Result<()> {
        ensure_repo_id(&self.repo_id, &session.repo_id, "interaction session")?;
        let conn = self.open_or_create()?;
        conn.execute_batch(DUCKDB_INTERACTION_SCHEMA)?;
        let sql = format!(
            "INSERT OR REPLACE INTO interaction_sessions (
                session_id, repo_id, agent_type, model, first_prompt,
                transcript_path, worktree_path, worktree_id, started_at,
                ended_at, last_event_at, updated_at
             ) VALUES (
                '{session_id}', '{repo_id}', '{agent_type}', '{model}', '{first_prompt}',
                '{transcript_path}', '{worktree_path}', '{worktree_id}', '{started_at}',
                {ended_at}, '{last_event_at}', '{updated_at}'
             )",
            session_id = esc_pg(&session.session_id),
            repo_id = esc_pg(&self.repo_id),
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

    fn upsert_turn(&self, turn: &InteractionTurn) -> Result<()> {
        ensure_repo_id(&self.repo_id, &turn.repo_id, "interaction turn")?;
        let conn = self.open_or_create()?;
        conn.execute_batch(DUCKDB_INTERACTION_SCHEMA)?;
        let usage = turn.token_usage.clone().unwrap_or_default();
        let files_modified =
            serde_json::to_string(&turn.files_modified).context("serialising files_modified")?;
        let sql = format!(
            "INSERT OR REPLACE INTO interaction_turns (
                turn_id, session_id, repo_id, turn_number, prompt,
                agent_type, model, started_at, ended_at, has_token_usage,
                input_tokens, cache_creation_tokens, cache_read_tokens,
                output_tokens, api_call_count, files_modified, checkpoint_id, updated_at
             ) VALUES (
                '{turn_id}', '{session_id}', '{repo_id}', {turn_number}, '{prompt}',
                '{agent_type}', '{model}', '{started_at}', {ended_at}, {has_token_usage},
                {input_tokens}, {cache_creation_tokens}, {cache_read_tokens},
                {output_tokens}, {api_call_count}, '{files_modified}', '{checkpoint_id}', '{updated_at}'
             )",
            turn_id = esc_pg(&turn.turn_id),
            session_id = esc_pg(&turn.session_id),
            repo_id = esc_pg(&self.repo_id),
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
            files_modified = esc_pg(&files_modified),
            checkpoint_id = esc_pg(turn.checkpoint_id.as_deref().unwrap_or("")),
            updated_at = esc_pg(&turn.updated_at),
        );
        conn.execute_batch(&sql)
            .context("upserting interaction turn in DuckDB")?;
        Ok(())
    }

    fn append_event(&self, event: &InteractionEvent) -> Result<()> {
        ensure_repo_id(&self.repo_id, &event.repo_id, "interaction event")?;
        let conn = self.open_or_create()?;
        conn.execute_batch(DUCKDB_INTERACTION_SCHEMA)?;
        let payload = serde_json::to_string(&event.payload).context("serialising event payload")?;
        let sql = format!(
            "INSERT OR IGNORE INTO interaction_events (
                event_id, event_time, repo_id, session_id, turn_id,
                event_type, agent_type, model, payload
             ) VALUES (
                '{event_id}', '{event_time}', '{repo_id}', '{session_id}', '{turn_id}',
                '{event_type}', '{agent_type}', '{model}', '{payload}'
             )",
            event_id = esc_pg(&event.event_id),
            event_time = esc_pg(&event.event_time),
            repo_id = esc_pg(&self.repo_id),
            session_id = esc_pg(&event.session_id),
            turn_id = esc_pg(event.turn_id.as_deref().unwrap_or("")),
            event_type = esc_pg(event.event_type.as_str()),
            agent_type = esc_pg(&event.agent_type),
            model = esc_pg(&event.model),
            payload = esc_pg(&payload),
        );
        conn.execute_batch(&sql)
            .context("appending interaction event in DuckDB")?;
        Ok(())
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
        let conn = self.open_or_create()?;
        conn.execute_batch(DUCKDB_INTERACTION_SCHEMA)?;
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

    fn list_sessions(&self, agent: Option<&str>, limit: usize) -> Result<Vec<InteractionSession>> {
        let conn = self.open_or_create()?;
        conn.execute_batch(DUCKDB_INTERACTION_SCHEMA)?;
        let mut sql = format!(
            "SELECT session_id, repo_id, agent_type, model, first_prompt,
                    transcript_path, worktree_path, worktree_id, started_at,
                    ended_at, last_event_at, updated_at
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
        let rows = stmt.query_map([], map_duckdb_session_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("reading interaction sessions from DuckDB")
    }

    fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>> {
        let conn = self.open_or_create()?;
        conn.execute_batch(DUCKDB_INTERACTION_SCHEMA)?;
        let sql = format!(
            "SELECT session_id, repo_id, agent_type, model, first_prompt,
                    transcript_path, worktree_path, worktree_id, started_at,
                    ended_at, last_event_at, updated_at
             FROM interaction_sessions
             WHERE repo_id = '{repo_id}' AND session_id = '{session_id}'
             LIMIT 1",
            repo_id = esc_pg(&self.repo_id),
            session_id = esc_pg(session_id),
        );
        let mut stmt = conn.prepare(&sql)?;
        stmt.query_row([], map_duckdb_session_row)
            .optional()
            .map_err(anyhow::Error::from)
    }

    fn list_turns_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        let conn = self.open_or_create()?;
        conn.execute_batch(DUCKDB_INTERACTION_SCHEMA)?;
        let sql = format!(
            "SELECT turn_id, session_id, repo_id, turn_number, prompt,
                    agent_type, model, started_at, ended_at, has_token_usage,
                    input_tokens, cache_creation_tokens, cache_read_tokens,
                    output_tokens, api_call_count, files_modified, checkpoint_id, updated_at
             FROM interaction_turns
             WHERE repo_id = '{repo_id}' AND session_id = '{session_id}'
             ORDER BY turn_number ASC, started_at ASC
             LIMIT {limit}",
            repo_id = esc_pg(&self.repo_id),
            session_id = esc_pg(session_id),
            limit = limit.max(1),
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], map_duckdb_turn_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("reading interaction turns from DuckDB")
    }

    fn list_events(
        &self,
        filter: &InteractionEventFilter,
        limit: usize,
    ) -> Result<Vec<InteractionEvent>> {
        let conn = self.open_or_create()?;
        conn.execute_batch(DUCKDB_INTERACTION_SCHEMA)?;
        let mut sql = format!(
            "SELECT event_id, session_id, turn_id, repo_id, event_type,
                    event_time, agent_type, model, payload
             FROM interaction_events
             WHERE repo_id = '{repo_id}'",
            repo_id = esc_pg(&self.repo_id),
        );
        append_duckdb_event_filter_sql(&mut sql, filter);
        sql.push_str(" ORDER BY event_time DESC, event_id DESC");
        sql.push_str(&format!(" LIMIT {}", limit.max(1)));

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], map_duckdb_event_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("reading interaction events from DuckDB")
    }
}

const DUCKDB_INTERACTION_SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS interaction_sessions (
    session_id VARCHAR PRIMARY KEY,
    repo_id VARCHAR,
    agent_type VARCHAR,
    model VARCHAR,
    first_prompt VARCHAR,
    transcript_path VARCHAR,
    worktree_path VARCHAR,
    worktree_id VARCHAR,
    started_at VARCHAR,
    ended_at VARCHAR,
    last_event_at VARCHAR,
    updated_at VARCHAR
);
CREATE INDEX IF NOT EXISTS interaction_sessions_repo_idx
    ON interaction_sessions (repo_id, last_event_at, started_at);

CREATE TABLE IF NOT EXISTS interaction_turns (
    turn_id VARCHAR PRIMARY KEY,
    session_id VARCHAR,
    repo_id VARCHAR,
    turn_number INTEGER,
    prompt VARCHAR,
    agent_type VARCHAR,
    model VARCHAR,
    started_at VARCHAR,
    ended_at VARCHAR,
    has_token_usage INTEGER,
    input_tokens BIGINT,
    cache_creation_tokens BIGINT,
    cache_read_tokens BIGINT,
    output_tokens BIGINT,
    api_call_count BIGINT,
    files_modified VARCHAR,
    checkpoint_id VARCHAR,
    updated_at VARCHAR
);
CREATE INDEX IF NOT EXISTS interaction_turns_session_idx
    ON interaction_turns (session_id, turn_number, started_at);
CREATE INDEX IF NOT EXISTS interaction_turns_pending_idx
    ON interaction_turns (repo_id, checkpoint_id, session_id, turn_number);

CREATE TABLE IF NOT EXISTS interaction_events (
    event_id VARCHAR PRIMARY KEY,
    event_time VARCHAR,
    repo_id VARCHAR,
    session_id VARCHAR,
    turn_id VARCHAR,
    event_type VARCHAR,
    agent_type VARCHAR,
    model VARCHAR,
    payload VARCHAR
);
CREATE INDEX IF NOT EXISTS interaction_events_repo_time_idx
    ON interaction_events (repo_id, event_time);
CREATE INDEX IF NOT EXISTS interaction_events_session_idx
    ON interaction_events (session_id, event_time);
CREATE INDEX IF NOT EXISTS interaction_events_type_idx
    ON interaction_events (repo_id, event_type, event_time);
";

pub(crate) struct ClickHouseInteractionRepository {
    repo_id: String,
    endpoint: String,
    user: Option<String>,
    password: Option<String>,
}

impl ClickHouseInteractionRepository {
    fn repo_id(&self) -> &str {
        &self.repo_id
    }

    fn ensure_schema(&self) -> Result<()> {
        blocking_clickhouse_exec(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            CLICKHOUSE_INTERACTION_SCHEMA,
        )
        .context("ensuring ClickHouse interaction schema")?;
        Ok(())
    }

    fn upsert_session(&self, session: &InteractionSession) -> Result<()> {
        ensure_repo_id(&self.repo_id, &session.repo_id, "interaction session")?;
        let sql = format!(
            "INSERT INTO interaction_sessions (
                session_id, repo_id, agent_type, model, first_prompt,
                transcript_path, worktree_path, worktree_id, started_at,
                ended_at, last_event_at, updated_at
             ) VALUES (
                '{session_id}', '{repo_id}', '{agent_type}', '{model}', '{first_prompt}',
                '{transcript_path}', '{worktree_path}', '{worktree_id}', '{started_at}',
                '{ended_at}', '{last_event_at}',
                coalesce(parseDateTime64BestEffortOrNull('{updated_at}'), now64(3))
             )",
            session_id = esc_ch(&session.session_id),
            repo_id = esc_ch(&self.repo_id),
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
        blocking_clickhouse_exec(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &sql,
        )
        .context("upserting interaction session in ClickHouse")?;
        Ok(())
    }

    fn upsert_turn(&self, turn: &InteractionTurn) -> Result<()> {
        ensure_repo_id(&self.repo_id, &turn.repo_id, "interaction turn")?;
        let usage = turn.token_usage.clone().unwrap_or_default();
        let files_modified = format_clickhouse_array(&turn.files_modified);
        let sql = format!(
            "INSERT INTO interaction_turns (
                turn_id, session_id, repo_id, turn_number, prompt,
                agent_type, model, started_at, ended_at, has_token_usage,
                input_tokens, cache_creation_tokens, cache_read_tokens,
                output_tokens, api_call_count, files_modified, checkpoint_id, updated_at
             ) VALUES (
                '{turn_id}', '{session_id}', '{repo_id}', {turn_number}, '{prompt}',
                '{agent_type}', '{model}', '{started_at}', '{ended_at}', {has_token_usage},
                {input_tokens}, {cache_creation_tokens}, {cache_read_tokens},
                {output_tokens}, {api_call_count}, {files_modified}, '{checkpoint_id}',
                coalesce(parseDateTime64BestEffortOrNull('{updated_at}'), now64(3))
             )",
            turn_id = esc_ch(&turn.turn_id),
            session_id = esc_ch(&turn.session_id),
            repo_id = esc_ch(&self.repo_id),
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
            files_modified = files_modified,
            checkpoint_id = esc_ch(turn.checkpoint_id.as_deref().unwrap_or("")),
            updated_at = esc_ch(&turn.updated_at),
        );
        blocking_clickhouse_exec(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &sql,
        )
        .context("upserting interaction turn in ClickHouse")?;
        Ok(())
    }

    fn append_event(&self, event: &InteractionEvent) -> Result<()> {
        ensure_repo_id(&self.repo_id, &event.repo_id, "interaction event")?;
        let payload = serde_json::to_string(&event.payload).context("serialising event payload")?;
        let sql = format!(
            "INSERT INTO interaction_events (
                event_id, event_time, repo_id, session_id, turn_id,
                event_type, agent_type, model, payload
             ) VALUES (
                '{event_id}', coalesce(parseDateTime64BestEffortOrNull('{event_time}'), now64(3)),
                '{repo_id}', '{session_id}', '{turn_id}',
                '{event_type}', '{agent_type}', '{model}', '{payload}'
             )",
            event_id = esc_ch(&event.event_id),
            event_time = esc_ch(&event.event_time),
            repo_id = esc_ch(&self.repo_id),
            session_id = esc_ch(&event.session_id),
            turn_id = esc_ch(event.turn_id.as_deref().unwrap_or("")),
            event_type = esc_ch(event.event_type.as_str()),
            agent_type = esc_ch(&event.agent_type),
            model = esc_ch(&event.model),
            payload = esc_ch(&payload),
        );
        blocking_clickhouse_exec(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &sql,
        )
        .context("appending interaction event in ClickHouse")?;
        Ok(())
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
        let turns = self.load_turns_by_ids(turn_ids)?;
        for mut turn in turns {
            turn.checkpoint_id = Some(checkpoint_id.to_string());
            turn.updated_at = assigned_at.to_string();
            self.upsert_turn(&turn)?;
        }
        Ok(())
    }

    fn list_sessions(&self, agent: Option<&str>, limit: usize) -> Result<Vec<InteractionSession>> {
        let mut conditions = vec![format!("repo_id = '{}'", esc_ch(&self.repo_id))];
        if let Some(agent) = agent.map(str::trim).filter(|value| !value.is_empty()) {
            conditions.push(format!("agent_type = '{}'", esc_ch(agent)));
        }
        let sql = format!(
            "SELECT * FROM (
                SELECT
                    session_id,
                    argMax(repo_id, updated_at) AS repo_id,
                    argMax(agent_type, updated_at) AS agent_type,
                    argMax(model, updated_at) AS model,
                    argMax(first_prompt, updated_at) AS first_prompt,
                    argMax(transcript_path, updated_at) AS transcript_path,
                    argMax(worktree_path, updated_at) AS worktree_path,
                    argMax(worktree_id, updated_at) AS worktree_id,
                    argMax(started_at, updated_at) AS started_at,
                    argMax(ended_at, updated_at) AS ended_at,
                    argMax(last_event_at, updated_at) AS last_event_at,
                    toString(max(updated_at)) AS updated_at
                FROM interaction_sessions
                WHERE {}
                GROUP BY session_id
            )
            ORDER BY parseDateTime64BestEffortOrZero(if(last_event_at = '', started_at, last_event_at)) DESC,
                     session_id DESC
            LIMIT {}",
            conditions.join(" AND "),
            limit.max(1),
        );
        let rows = blocking_clickhouse_query_rows(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &sql,
        )?;
        rows.iter().map(clickhouse_session_from_row).collect()
    }

    fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>> {
        let rows = blocking_clickhouse_query_rows(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &format!(
                "SELECT * FROM (
                    SELECT
                        session_id,
                        argMax(repo_id, updated_at) AS repo_id,
                        argMax(agent_type, updated_at) AS agent_type,
                        argMax(model, updated_at) AS model,
                        argMax(first_prompt, updated_at) AS first_prompt,
                        argMax(transcript_path, updated_at) AS transcript_path,
                        argMax(worktree_path, updated_at) AS worktree_path,
                        argMax(worktree_id, updated_at) AS worktree_id,
                        argMax(started_at, updated_at) AS started_at,
                        argMax(ended_at, updated_at) AS ended_at,
                        argMax(last_event_at, updated_at) AS last_event_at,
                        toString(max(updated_at)) AS updated_at
                    FROM interaction_sessions
                    WHERE repo_id = '{repo_id}' AND session_id = '{session_id}'
                    GROUP BY session_id
                )
                LIMIT 1",
                repo_id = esc_ch(&self.repo_id),
                session_id = esc_ch(session_id),
            ),
        )?;
        rows.first().map(clickhouse_session_from_row).transpose()
    }

    fn list_turns_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        let rows = blocking_clickhouse_query_rows(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &format!(
                "SELECT * FROM (
                    SELECT
                        turn_id,
                        argMax(session_id, updated_at) AS session_id,
                        argMax(repo_id, updated_at) AS repo_id,
                        argMax(turn_number, updated_at) AS turn_number,
                        argMax(prompt, updated_at) AS prompt,
                        argMax(agent_type, updated_at) AS agent_type,
                        argMax(model, updated_at) AS model,
                        argMax(started_at, updated_at) AS started_at,
                        argMax(ended_at, updated_at) AS ended_at,
                        argMax(has_token_usage, updated_at) AS has_token_usage,
                        argMax(input_tokens, updated_at) AS input_tokens,
                        argMax(cache_creation_tokens, updated_at) AS cache_creation_tokens,
                        argMax(cache_read_tokens, updated_at) AS cache_read_tokens,
                        argMax(output_tokens, updated_at) AS output_tokens,
                        argMax(api_call_count, updated_at) AS api_call_count,
                        argMax(files_modified, updated_at) AS files_modified,
                        argMax(checkpoint_id, updated_at) AS checkpoint_id,
                        toString(max(updated_at)) AS updated_at
                    FROM interaction_turns
                    WHERE repo_id = '{repo_id}' AND session_id = '{session_id}'
                    GROUP BY turn_id
                )
                ORDER BY turn_number ASC, started_at ASC
                LIMIT {limit}",
                repo_id = esc_ch(&self.repo_id),
                session_id = esc_ch(session_id),
                limit = limit.max(1),
            ),
        )?;
        rows.iter().map(clickhouse_turn_from_row).collect()
    }

    fn list_events(
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
        let rows = blocking_clickhouse_query_rows(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &format!(
                "SELECT event_id, session_id, turn_id, repo_id, event_type,
                        toString(event_time) AS event_time, agent_type, model, payload
                 FROM interaction_events
                 WHERE {}
                 ORDER BY event_time DESC, event_id DESC
                 LIMIT {}",
                conditions.join(" AND "),
                limit.max(1),
            ),
        )?;
        rows.iter().map(clickhouse_event_from_row).collect()
    }

    fn load_turns_by_ids(&self, turn_ids: &[String]) -> Result<Vec<InteractionTurn>> {
        let ids = turn_ids
            .iter()
            .map(|turn_id| format!("'{}'", esc_ch(turn_id)))
            .collect::<Vec<_>>()
            .join(", ");
        let rows = blocking_clickhouse_query_rows(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            &format!(
                "SELECT * FROM (
                    SELECT
                        turn_id,
                        argMax(session_id, updated_at) AS session_id,
                        argMax(repo_id, updated_at) AS repo_id,
                        argMax(turn_number, updated_at) AS turn_number,
                        argMax(prompt, updated_at) AS prompt,
                        argMax(agent_type, updated_at) AS agent_type,
                        argMax(model, updated_at) AS model,
                        argMax(started_at, updated_at) AS started_at,
                        argMax(ended_at, updated_at) AS ended_at,
                        argMax(has_token_usage, updated_at) AS has_token_usage,
                        argMax(input_tokens, updated_at) AS input_tokens,
                        argMax(cache_creation_tokens, updated_at) AS cache_creation_tokens,
                        argMax(cache_read_tokens, updated_at) AS cache_read_tokens,
                        argMax(output_tokens, updated_at) AS output_tokens,
                        argMax(api_call_count, updated_at) AS api_call_count,
                        argMax(files_modified, updated_at) AS files_modified,
                        argMax(checkpoint_id, updated_at) AS checkpoint_id,
                        toString(max(updated_at)) AS updated_at
                    FROM interaction_turns
                    WHERE repo_id = '{repo_id}' AND turn_id IN ({ids})
                    GROUP BY turn_id
                )",
                repo_id = esc_ch(&self.repo_id),
                ids = ids,
            ),
        )?;
        rows.iter().map(clickhouse_turn_from_row).collect()
    }
}

const CLICKHOUSE_INTERACTION_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS interaction_sessions (
    session_id String,
    repo_id String,
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
ORDER BY (repo_id, session_id);

CREATE TABLE IF NOT EXISTS interaction_turns (
    turn_id String,
    session_id String,
    repo_id String,
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
    files_modified Array(String),
    checkpoint_id String,
    updated_at DateTime64(3, 'UTC')
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (repo_id, session_id, turn_id);

CREATE TABLE IF NOT EXISTS interaction_events (
    event_id String,
    event_time DateTime64(3, 'UTC'),
    repo_id String,
    session_id String,
    turn_id String,
    event_type String,
    agent_type String,
    model String,
    payload String
)
ENGINE = ReplacingMergeTree(event_time)
ORDER BY (repo_id, event_time, event_id);
"#;

fn quoted_nullable(value: &Option<String>) -> String {
    match value.as_deref().filter(|candidate| !candidate.is_empty()) {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    }
}

fn append_duckdb_event_filter_sql(sql: &mut String, filter: &InteractionEventFilter) {
    if let Some(session_id) = filter
        .session_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        sql.push_str(&format!(" AND session_id = '{}'", esc_pg(session_id)));
    }
    if let Some(turn_id) = filter.turn_id.as_deref().filter(|value| !value.is_empty()) {
        sql.push_str(&format!(" AND turn_id = '{}'", esc_pg(turn_id)));
    }
    if let Some(event_type) = filter.event_type {
        sql.push_str(&format!(
            " AND event_type = '{}'",
            esc_pg(event_type.as_str())
        ));
    }
    if let Some(since) = filter.since.as_deref().filter(|value| !value.is_empty()) {
        sql.push_str(&format!(" AND event_time >= '{}'", esc_pg(since)));
    }
}

fn ensure_repo_id(expected: &str, actual: &str, entity: &str) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    bail!("repo_id mismatch for {entity}: expected `{expected}`, got `{actual}`");
}

fn map_duckdb_session_row(row: &duckdb::Row<'_>) -> duckdb::Result<InteractionSession> {
    let ended_at: Option<String> = row.get(9)?;
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
        ended_at: ended_at.filter(|value| !value.trim().is_empty()),
        last_event_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn map_duckdb_turn_row(row: &duckdb::Row<'_>) -> duckdb::Result<InteractionTurn> {
    let files_modified_raw: String = row.get(15)?;
    let files_modified =
        serde_json::from_str::<Vec<String>>(&files_modified_raw).map_err(|err| {
            duckdb::Error::FromSqlConversionFailure(15, duckdb::types::Type::Text, Box::new(err))
        })?;
    let checkpoint_id: String = row.get(16)?;
    let has_token_usage: i32 = row.get(9)?;
    Ok(InteractionTurn {
        turn_id: row.get(0)?,
        session_id: row.get(1)?,
        repo_id: row.get(2)?,
        turn_number: u32::try_from(row.get::<_, i32>(3)?).unwrap_or_default(),
        prompt: row.get(4)?,
        agent_type: row.get(5)?,
        model: row.get(6)?,
        started_at: row.get(7)?,
        ended_at: row
            .get::<_, Option<String>>(8)?
            .filter(|value| !value.trim().is_empty()),
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

fn map_duckdb_event_row(row: &duckdb::Row<'_>) -> duckdb::Result<InteractionEvent> {
    let event_type_raw: String = row.get(4)?;
    let payload_raw: String = row.get(8)?;
    let payload = serde_json::from_str::<Value>(&payload_raw).map_err(|err| {
        duckdb::Error::FromSqlConversionFailure(8, duckdb::types::Type::Text, Box::new(err))
    })?;
    let event_type = InteractionEventType::parse(&event_type_raw).ok_or_else(|| {
        duckdb::Error::FromSqlConversionFailure(
            4,
            duckdb::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown interaction event type `{event_type_raw}`"),
            )),
        )
    })?;
    let turn_id: String = row.get(2)?;
    Ok(InteractionEvent {
        event_id: row.get(0)?,
        session_id: row.get(1)?,
        turn_id: (!turn_id.trim().is_empty()).then_some(turn_id),
        repo_id: row.get(3)?,
        event_type,
        event_time: row.get(5)?,
        agent_type: row.get(6)?,
        model: row.get(7)?,
        payload,
    })
}

fn format_clickhouse_array(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| format!("'{}'", esc_ch(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
}

fn blocking_clickhouse_query_rows(
    endpoint: &str,
    user: Option<&str>,
    password: Option<&str>,
    sql: &str,
) -> Result<Vec<Value>> {
    let mut query = sql.trim().to_string();
    if !query.to_ascii_uppercase().contains("FORMAT JSON") {
        query.push_str(" FORMAT JSON");
    }
    let raw = blocking_clickhouse_exec(endpoint, user, password, &query)?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: Value =
        serde_json::from_str(&raw).with_context(|| format!("parsing ClickHouse JSON: {raw}"))?;
    Ok(parsed
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

fn blocking_clickhouse_exec(
    endpoint: &str,
    user: Option<&str>,
    password: Option<&str>,
    sql: &str,
) -> Result<String> {
    let client = blocking_clickhouse_client()?;
    let mut request = client.post(endpoint).body(sql.to_string());
    if let Some(username) = user {
        request = request.basic_auth(username, Some(password.unwrap_or("")));
    }
    let response = request
        .send()
        .context("sending interaction request to ClickHouse")?;
    let status = response.status();
    let body = response
        .text()
        .context("reading ClickHouse response body")?;
    if !status.is_success() {
        let detail = body.trim();
        if detail.is_empty() {
            bail!("ClickHouse request failed with status {status}");
        }
        bail!("ClickHouse request failed with status {status}: {detail}");
    }
    Ok(body)
}

fn blocking_clickhouse_client() -> Result<&'static reqwest::blocking::Client> {
    static CLIENT: OnceLock<Result<reqwest::blocking::Client, String>> = OnceLock::new();
    let result = CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|err| format!("{err:#}"))
    });
    match result {
        Ok(client) => Ok(client),
        Err(err) => Err(anyhow!("building blocking ClickHouse client: {err}")),
    }
}

fn clickhouse_session_from_row(row: &Value) -> Result<InteractionSession> {
    Ok(InteractionSession {
        session_id: required_string(row, "session_id")?,
        repo_id: required_string(row, "repo_id")?,
        agent_type: optional_string(row, "agent_type"),
        model: optional_string(row, "model"),
        first_prompt: optional_string(row, "first_prompt"),
        transcript_path: optional_string(row, "transcript_path"),
        worktree_path: optional_string(row, "worktree_path"),
        worktree_id: optional_string(row, "worktree_id"),
        started_at: optional_string(row, "started_at"),
        ended_at: empty_to_none(optional_string(row, "ended_at")),
        last_event_at: optional_string(row, "last_event_at"),
        updated_at: optional_string(row, "updated_at"),
    })
}

fn clickhouse_turn_from_row(row: &Value) -> Result<InteractionTurn> {
    let files_modified = match row.get("files_modified") {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(other) => bail!("unexpected files_modified payload: {other}"),
        None => Vec::new(),
    };
    let has_token_usage = row
        .get("has_token_usage")
        .and_then(Value::as_u64)
        .unwrap_or_default()
        == 1;
    Ok(InteractionTurn {
        turn_id: required_string(row, "turn_id")?,
        session_id: required_string(row, "session_id")?,
        repo_id: required_string(row, "repo_id")?,
        turn_number: row
            .get("turn_number")
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32,
        prompt: optional_string(row, "prompt"),
        agent_type: optional_string(row, "agent_type"),
        model: optional_string(row, "model"),
        started_at: optional_string(row, "started_at"),
        ended_at: empty_to_none(optional_string(row, "ended_at")),
        token_usage: has_token_usage.then(|| TokenUsageMetadata {
            input_tokens: row
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            cache_creation_tokens: row
                .get("cache_creation_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            cache_read_tokens: row
                .get("cache_read_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            output_tokens: row
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            api_call_count: row
                .get("api_call_count")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            subagent_tokens: None,
        }),
        files_modified,
        checkpoint_id: empty_to_none(optional_string(row, "checkpoint_id")),
        updated_at: optional_string(row, "updated_at"),
    })
}

fn clickhouse_event_from_row(row: &Value) -> Result<InteractionEvent> {
    let event_type_raw = required_string(row, "event_type")?;
    let payload = row
        .get("payload")
        .and_then(Value::as_str)
        .map(serde_json::from_str::<Value>)
        .transpose()
        .context("parsing interaction event payload")?
        .unwrap_or_else(|| Value::Object(Default::default()));
    Ok(InteractionEvent {
        event_id: required_string(row, "event_id")?,
        session_id: required_string(row, "session_id")?,
        turn_id: empty_to_none(optional_string(row, "turn_id")),
        repo_id: required_string(row, "repo_id")?,
        event_type: InteractionEventType::parse(&event_type_raw)
            .ok_or_else(|| anyhow!("unknown interaction event type `{event_type_raw}`"))?,
        event_time: required_string(row, "event_time")?,
        agent_type: optional_string(row, "agent_type"),
        model: optional_string(row, "model"),
        payload,
    })
}

fn required_string(row: &Value, field: &str) -> Result<String> {
    row.get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("missing `{field}` in ClickHouse interaction row"))
}

fn optional_string(row: &Value, field: &str) -> String {
    row.get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn empty_to_none(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::interactions::types::InteractionEventType;

    fn sample_session() -> InteractionSession {
        InteractionSession {
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
        }
    }

    fn sample_turn() -> InteractionTurn {
        InteractionTurn {
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
            files_modified: vec!["src/main.rs".into()],
            updated_at: "2026-04-05T10:00:02Z".into(),
            ..Default::default()
        }
    }

    fn sample_event() -> InteractionEvent {
        InteractionEvent {
            event_id: "evt-1".into(),
            session_id: "sess-1".into(),
            turn_id: Some("turn-1".into()),
            repo_id: "repo-test".into(),
            event_type: InteractionEventType::TurnEnd,
            event_time: "2026-04-05T10:00:02Z".into(),
            agent_type: "codex".into(),
            model: "gpt-5.4".into(),
            payload: serde_json::json!({"token_usage": {"input_tokens": 11}}),
        }
    }

    fn duckdb_repository(temp_dir: &tempfile::TempDir) -> DuckDbInteractionRepository {
        DuckDbInteractionRepository {
            repo_id: "repo-test".into(),
            path: temp_dir.path().join("events.duckdb"),
        }
    }

    #[test]
    fn duckdb_repository_round_trip_sessions_turns_and_events() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let repository = duckdb_repository(&temp_dir);
        repository.ensure_schema().expect("schema");

        repository
            .upsert_session(&sample_session())
            .expect("upsert session");
        repository.upsert_turn(&sample_turn()).expect("upsert turn");
        repository
            .append_event(&sample_event())
            .expect("append event");

        assert_eq!(repository.list_sessions(None, 10).unwrap().len(), 1);
        assert_eq!(
            repository
                .list_turns_for_session("sess-1", 10)
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
    fn duckdb_repository_assigns_checkpoint_ids() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let repository = duckdb_repository(&temp_dir);
        repository.ensure_schema().expect("schema");
        repository.upsert_turn(&sample_turn()).expect("upsert turn");

        repository
            .assign_checkpoint_to_turns(&["turn-1".to_string()], "cp-1", "2026-04-05T11:00:00Z")
            .expect("assign checkpoint");

        let turn = repository
            .list_turns_for_session("sess-1", 10)
            .unwrap()
            .pop()
            .expect("one turn");
        assert_eq!(turn.checkpoint_id.as_deref(), Some("cp-1"));
    }

    #[test]
    #[ignore = "requires a running ClickHouse instance configured via BITLOOPS_TEST_CLICKHOUSE_URL"]
    fn clickhouse_repository_round_trip_is_env_gated() {
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
        repository
            .upsert_session(&sample_session())
            .expect("upsert session");
        repository.upsert_turn(&sample_turn()).expect("upsert turn");
        repository
            .append_event(&sample_event())
            .expect("append event");
        assert!(!repository.list_sessions(None, 10).unwrap().is_empty());
    }
}
