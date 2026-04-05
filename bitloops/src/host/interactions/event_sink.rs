use std::path::Path;

use anyhow::{Result, bail};

use super::store::InteractionEventRepository;
use super::types::{InteractionEvent, InteractionEventFilter, InteractionSession, InteractionTurn};
use crate::config::EventsBackendConfig;

mod clickhouse;
mod clickhouse_client;
mod duckdb;

use self::clickhouse::ClickHouseInteractionRepository;
use self::duckdb::DuckDbInteractionRepository;

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

fn ensure_repo_id(expected: &str, actual: &str, entity: &str) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    bail!("repo_id mismatch for {entity}: expected `{expected}`, got `{actual}`");
}
