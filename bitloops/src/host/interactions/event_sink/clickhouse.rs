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
        blocking_exec(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            SCHEMA,
        )
        .context("ensuring ClickHouse interaction schema")?;
        Ok(())
    }

    pub(super) fn upsert_session(&self, session: &InteractionSession) -> Result<()> {
        super::ensure_repo_id(&self.repo_id, &session.repo_id, "interaction session")?;
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
        rows.iter().map(turn_from_row).collect()
    }

    pub(super) fn list_uncheckpointed_turns(&self) -> Result<Vec<InteractionTurn>> {
        let rows = blocking_query_rows(
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
                    WHERE repo_id = '{repo_id}'
                    GROUP BY turn_id
                )
                WHERE checkpoint_id = ''
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
        rows.iter().map(turn_from_row).collect()
    }
}

const SCHEMA: &str = r#"
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

fn format_array(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| format!("'{}'", esc_ch(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
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
        };
        repository.append_event(&event).expect("append event");
        assert!(!repository.list_sessions(None, 10).unwrap().is_empty());
    }
}
