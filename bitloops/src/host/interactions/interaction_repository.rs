use std::path::Path;

use anyhow::{Result, bail};

use super::store::InteractionEventRepository;
use super::types::{InteractionEvent, InteractionEventFilter, InteractionSession, InteractionTurn};
use crate::config::EventsBackendConfig;
use crate::storage::{EventStorageRole, StorageBackendKind, StorageRoleResolver};

mod clickhouse;
mod clickhouse_client;
mod duckdb;

use self::clickhouse::ClickHouseInteractionRepository;
use self::duckdb::DuckDbInteractionRepository;

pub fn create_interaction_repository(
    events_cfg: &EventsBackendConfig,
    repo_root: &Path,
    repo_id: String,
) -> Result<impl InteractionEventRepository + use<>> {
    match StorageRoleResolver::from_events_config(events_cfg)
        .event_backend_for(EventStorageRole::CanonicalEvents)
    {
        StorageBackendKind::ClickHouse => {
            let repository = ClickHouseInteractionRepository {
                repo_id,
                endpoint: events_cfg.clickhouse_endpoint(),
                user: events_cfg.clickhouse_user.clone(),
                password: events_cfg.clickhouse_password.clone(),
            };
            repository.ensure_schema()?;
            Ok(InteractionRepositoryBackend::ClickHouse(repository))
        }
        StorageBackendKind::DuckDb => {
            let repository = DuckDbInteractionRepository {
                repo_id,
                path: events_cfg.resolve_duckdb_db_path_for_repo(repo_root),
            };
            repository.ensure_schema()?;
            Ok(InteractionRepositoryBackend::DuckDb(repository))
        }
        other => bail!(
            "unsupported canonical events backend for interaction repository: {}",
            other.label()
        ),
    }
}

enum InteractionRepositoryBackend {
    DuckDb(DuckDbInteractionRepository),
    ClickHouse(ClickHouseInteractionRepository),
}

impl InteractionEventRepository for InteractionRepositoryBackend {
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

    fn list_uncheckpointed_turns(&self) -> Result<Vec<InteractionTurn>> {
        match self {
            Self::DuckDb(repository) => repository.list_uncheckpointed_turns(),
            Self::ClickHouse(repository) => repository.list_uncheckpointed_turns(),
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

fn ensure_repo_id(expected: &str, actual: &str, entity: &str) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    bail!("repo_id mismatch for {entity}: expected `{expected}`, got `{actual}`");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EventsBackendConfig;
    use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
    use crate::host::interactions::types::{
        InteractionEvent, InteractionEventType, InteractionSession, InteractionTurn,
    };

    fn sample_session(repo_id: &str) -> InteractionSession {
        InteractionSession {
            session_id: "sess-1".into(),
            repo_id: repo_id.into(),
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

    fn sample_turn(repo_id: &str) -> InteractionTurn {
        InteractionTurn {
            turn_id: "turn-1".into(),
            session_id: "sess-1".into(),
            repo_id: repo_id.into(),
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

    fn sample_event(repo_id: &str) -> InteractionEvent {
        InteractionEvent {
            event_id: "evt-1".into(),
            session_id: "sess-1".into(),
            turn_id: Some("turn-1".into()),
            repo_id: repo_id.into(),
            event_type: InteractionEventType::TurnEnd,
            event_time: "2026-04-05T10:00:02Z".into(),
            agent_type: "codex".into(),
            model: "gpt-5.4".into(),
            payload: serde_json::json!({"token_usage": {"input_tokens": 11}}),
            ..Default::default()
        }
    }

    #[test]
    fn create_interaction_repository_persists_canonical_rows_in_selected_duckdb_backend() {
        let repo_root = tempfile::tempdir().expect("temp dir");
        let duckdb_path = repo_root.path().join("events").join("events.duckdb");
        let events_cfg = EventsBackendConfig {
            duckdb_path: Some(duckdb_path.to_string_lossy().to_string()),
            clickhouse_url: None,
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: None,
        };
        let repo_id = "repo-test";
        let repository =
            create_interaction_repository(&events_cfg, repo_root.path(), repo_id.to_string())
                .expect("create DuckDB-backed interaction repository");

        repository
            .upsert_session(&sample_session(repo_id))
            .expect("upsert session");
        repository
            .upsert_turn(&sample_turn(repo_id))
            .expect("upsert turn");
        repository
            .append_event(&sample_event(repo_id))
            .expect("append event");

        let conn = ::duckdb::Connection::open(&duckdb_path).expect("open canonical events duckdb");
        let session_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM interaction_sessions WHERE repo_id = ?",
                [repo_id],
                |row| row.get(0),
            )
            .expect("count interaction_sessions rows");
        let turn_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM interaction_turns WHERE repo_id = ?",
                [repo_id],
                |row| row.get(0),
            )
            .expect("count interaction_turns rows");
        let event_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM interaction_events WHERE repo_id = ?",
                [repo_id],
                |row| row.get(0),
            )
            .expect("count interaction_events rows");

        assert_eq!(session_count, 1);
        assert_eq!(turn_count, 1);
        assert_eq!(event_count, 1);
    }

    #[test]
    fn create_interaction_repository_prefers_clickhouse_without_duckdb_fallback() {
        let repo_root = tempfile::tempdir().expect("temp dir");
        let duckdb_path = repo_root.path().join("fallback").join("events.duckdb");
        let events_cfg = EventsBackendConfig {
            duckdb_path: Some(duckdb_path.to_string_lossy().to_string()),
            clickhouse_url: Some("http://127.0.0.1:9".to_string()),
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: Some("default".to_string()),
        };

        let err = create_interaction_repository(&events_cfg, repo_root.path(), "repo-test".into())
            .err()
            .expect("unreachable ClickHouse backend must fail repository creation");
        let message = err.to_string();

        assert!(
            message.contains("ClickHouse") || message.contains("sending ClickHouse request"),
            "expected ClickHouse connection error, got: {message}"
        );
        assert!(
            !duckdb_path.exists(),
            "canonical interaction repository selection should not create DuckDB fallback storage when ClickHouse is configured"
        );
    }
}
