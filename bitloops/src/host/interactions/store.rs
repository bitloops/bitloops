use anyhow::Result;

use super::types::{InteractionEvent, InteractionEventFilter, InteractionSession, InteractionTurn};

/// Canonical interaction repository backed by the Event DB.
pub trait InteractionEventRepository: Send + Sync {
    fn repo_id(&self) -> &str;

    fn upsert_session(&self, session: &InteractionSession) -> Result<()>;

    fn upsert_turn(&self, turn: &InteractionTurn) -> Result<()>;

    fn append_event(&self, event: &InteractionEvent) -> Result<()>;

    fn assign_checkpoint_to_turns(
        &self,
        turn_ids: &[String],
        checkpoint_id: &str,
        assigned_at: &str,
    ) -> Result<()>;

    fn list_sessions(&self, agent: Option<&str>, limit: usize) -> Result<Vec<InteractionSession>>;

    fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>>;

    fn list_turns_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionTurn>>;

    fn list_events(
        &self,
        filter: &InteractionEventFilter,
        limit: usize,
    ) -> Result<Vec<InteractionEvent>>;
}

/// Durable local spool for interaction mutations.
pub trait InteractionSpool: Send + Sync {
    fn repo_id(&self) -> &str;

    fn record_session(&self, session: &InteractionSession) -> Result<()>;

    fn record_turn(&self, turn: &InteractionTurn) -> Result<()>;

    fn record_event(&self, event: &InteractionEvent) -> Result<()>;

    fn assign_checkpoint_to_turns(
        &self,
        turn_ids: &[String],
        checkpoint_id: &str,
        assigned_at: &str,
    ) -> Result<()>;

    fn flush(&self, repository: &dyn InteractionEventRepository) -> Result<usize>;

    fn list_sessions(&self, agent: Option<&str>, limit: usize) -> Result<Vec<InteractionSession>>;

    fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>>;

    fn list_turns_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionTurn>>;

    fn list_uncheckpointed_turns(&self) -> Result<Vec<InteractionTurn>>;

    fn list_events(
        &self,
        filter: &InteractionEventFilter,
        limit: usize,
    ) -> Result<Vec<InteractionEvent>>;
}
