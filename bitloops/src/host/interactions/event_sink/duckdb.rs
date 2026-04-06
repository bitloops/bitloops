use std::path::PathBuf;

use anyhow::{Context, Result};
use duckdb::OptionalExt;
use serde_json::Value;

use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::devql::esc_pg;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventFilter, InteractionEventType, InteractionSession,
    InteractionTurn,
};

pub(crate) struct DuckDbInteractionRepository {
    pub(super) repo_id: String,
    pub(super) path: PathBuf,
}

impl DuckDbInteractionRepository {
    pub(super) fn repo_id(&self) -> &str {
        &self.repo_id
    }

    pub(super) fn ensure_schema(&self) -> Result<()> {
        let conn = self.open_or_create()?;
        conn.execute_batch(SCHEMA)
            .context("ensuring DuckDB interaction schema")?;
        ensure_turn_columns(&conn)?;
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

    pub(super) fn upsert_session(&self, session: &InteractionSession) -> Result<()> {
        super::ensure_repo_id(&self.repo_id, &session.repo_id, "interaction session")?;
        let conn = self.open_or_create()?;
        conn.execute_batch(SCHEMA)?;
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

    pub(super) fn upsert_turn(&self, turn: &InteractionTurn) -> Result<()> {
        super::ensure_repo_id(&self.repo_id, &turn.repo_id, "interaction turn")?;
        let conn = self.open_or_create()?;
        conn.execute_batch(SCHEMA)?;
        let usage = turn.token_usage.clone().unwrap_or_default();
        let files_modified =
            serde_json::to_string(&turn.files_modified).context("serialising files_modified")?;
        let sql = format!(
            "INSERT OR REPLACE INTO interaction_turns (
                turn_id, session_id, repo_id, turn_number, prompt,
                agent_type, model, started_at, ended_at, has_token_usage,
                input_tokens, cache_creation_tokens, cache_read_tokens,
                output_tokens, api_call_count, summary, prompt_count,
                transcript_offset_start, transcript_offset_end, transcript_fragment,
                files_modified, checkpoint_id, updated_at
             ) VALUES (
                '{turn_id}', '{session_id}', '{repo_id}', {turn_number}, '{prompt}',
                '{agent_type}', '{model}', '{started_at}', {ended_at}, {has_token_usage},
                {input_tokens}, {cache_creation_tokens}, {cache_read_tokens},
                {output_tokens}, {api_call_count}, '{summary}', {prompt_count},
                {transcript_offset_start}, {transcript_offset_end}, '{transcript_fragment}',
                '{files_modified}', '{checkpoint_id}', '{updated_at}'
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

    pub(super) fn append_event(&self, event: &InteractionEvent) -> Result<()> {
        super::ensure_repo_id(&self.repo_id, &event.repo_id, "interaction event")?;
        let conn = self.open_or_create()?;
        conn.execute_batch(SCHEMA)?;
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

    pub(super) fn assign_checkpoint_to_turns(
        &self,
        turn_ids: &[String],
        checkpoint_id: &str,
        assigned_at: &str,
    ) -> Result<()> {
        if turn_ids.is_empty() {
            return Ok(());
        }
        let conn = self.open_or_create()?;
        conn.execute_batch(SCHEMA)?;
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

    pub(super) fn list_sessions(
        &self,
        agent: Option<&str>,
        limit: usize,
    ) -> Result<Vec<InteractionSession>> {
        let conn = self.open_or_create()?;
        conn.execute_batch(SCHEMA)?;
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
        let rows = stmt.query_map([], map_session_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("reading interaction sessions from DuckDB")
    }

    pub(super) fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>> {
        let conn = self.open_or_create()?;
        conn.execute_batch(SCHEMA)?;
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
        stmt.query_row([], map_session_row)
            .optional()
            .map_err(anyhow::Error::from)
    }

    pub(super) fn list_turns_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        let conn = self.open_or_create()?;
        conn.execute_batch(SCHEMA)?;
        let sql = format!(
            "SELECT turn_id, session_id, repo_id, turn_number, prompt,
                    agent_type, model, started_at, ended_at, has_token_usage,
                    input_tokens, cache_creation_tokens, cache_read_tokens,
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

    pub(super) fn list_uncheckpointed_turns(&self) -> Result<Vec<InteractionTurn>> {
        let conn = self.open_or_create()?;
        conn.execute_batch(SCHEMA)?;
        let sql = format!(
            "SELECT turn_id, session_id, repo_id, turn_number, prompt,
                    agent_type, model, started_at, ended_at, has_token_usage,
                    input_tokens, cache_creation_tokens, cache_read_tokens,
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

    pub(super) fn list_events(
        &self,
        filter: &InteractionEventFilter,
        limit: usize,
    ) -> Result<Vec<InteractionEvent>> {
        let conn = self.open_or_create()?;
        conn.execute_batch(SCHEMA)?;
        let mut sql = format!(
            "SELECT event_id, session_id, turn_id, repo_id, event_type,
                    event_time, agent_type, model, payload
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

const SCHEMA: &str = "\
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
    summary VARCHAR,
    prompt_count INTEGER,
    transcript_offset_start BIGINT,
    transcript_offset_end BIGINT,
    transcript_fragment VARCHAR,
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

fn ensure_turn_columns(conn: &duckdb::Connection) -> Result<()> {
    let missing = [
        (
            "summary",
            "ALTER TABLE interaction_turns ADD COLUMN summary VARCHAR DEFAULT ''",
        ),
        (
            "prompt_count",
            "ALTER TABLE interaction_turns ADD COLUMN prompt_count INTEGER DEFAULT 0",
        ),
        (
            "transcript_offset_start",
            "ALTER TABLE interaction_turns ADD COLUMN transcript_offset_start BIGINT",
        ),
        (
            "transcript_offset_end",
            "ALTER TABLE interaction_turns ADD COLUMN transcript_offset_end BIGINT",
        ),
        (
            "transcript_fragment",
            "ALTER TABLE interaction_turns ADD COLUMN transcript_fragment VARCHAR DEFAULT ''",
        ),
    ];
    for (column, alter_sql) in missing {
        let exists: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM information_schema.columns \
                     WHERE table_name = 'interaction_turns' AND column_name = '{column}'"
                ),
                [],
                |row| row.get(0),
            )
            .with_context(|| format!("checking DuckDB interaction_turns.{column} column"))?;
        if exists == 0 {
            conn.execute_batch(alter_sql)
                .with_context(|| format!("adding DuckDB interaction_turns.{column} column"))?;
        }
    }
    Ok(())
}

fn append_event_filter_sql(sql: &mut String, filter: &InteractionEventFilter) {
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

fn map_session_row(row: &duckdb::Row<'_>) -> duckdb::Result<InteractionSession> {
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

fn map_turn_row(row: &duckdb::Row<'_>) -> duckdb::Result<InteractionTurn> {
    let files_modified_raw: String = row.get(20)?;
    let files_modified =
        serde_json::from_str::<Vec<String>>(&files_modified_raw).map_err(|err| {
            duckdb::Error::FromSqlConversionFailure(20, duckdb::types::Type::Text, Box::new(err))
        })?;
    let checkpoint_id: String = row.get(21)?;
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
        summary: row.get(15)?,
        prompt_count: u32::try_from(row.get::<_, i32>(16)?).unwrap_or_default(),
        transcript_offset_start: row.get(17)?,
        transcript_offset_end: row.get(18)?,
        transcript_fragment: row.get(19)?,
        files_modified,
        checkpoint_id: (!checkpoint_id.trim().is_empty()).then_some(checkpoint_id),
        updated_at: row.get(22)?,
    })
}

fn map_event_row(row: &duckdb::Row<'_>) -> duckdb::Result<InteractionEvent> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
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
            summary: "completed main change".into(),
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

    fn make_repository(temp_dir: &tempfile::TempDir) -> DuckDbInteractionRepository {
        DuckDbInteractionRepository {
            repo_id: "repo-test".into(),
            path: temp_dir.path().join("events.duckdb"),
        }
    }

    #[test]
    fn round_trip_sessions_turns_and_events() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let repository = make_repository(&temp_dir);
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
        assert!(
            repository.list_turns_for_session("sess-1", 10).unwrap()[0]
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
    fn assigns_checkpoint_ids() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let repository = make_repository(&temp_dir);
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
}
