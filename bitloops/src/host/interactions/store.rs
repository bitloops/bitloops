use anyhow::Result;

use super::types::{InteractionEvent, InteractionSession, InteractionTurn};
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;

/// Persistence abstraction for interaction events.
///
/// Implementations write to the local checkpoint SQLite database.
/// All methods are synchronous (SQLite is local and fast).
pub trait InteractionEventStore: Send + Sync {
    /// Create or update a session record.
    fn record_session(&self, session: &InteractionSession) -> Result<()>;

    /// Update a session's ended_at timestamp.
    fn end_session(&self, session_id: &str, ended_at: &str) -> Result<()>;

    /// Insert a new turn at the start of an agent turn.
    fn record_turn_start(&self, turn: &InteractionTurn) -> Result<()>;

    /// Complete a turn with token usage and file change data.
    fn record_turn_end(
        &self,
        turn_id: &str,
        ended_at: &str,
        token_usage: Option<&TokenUsageMetadata>,
        files_modified: &[String],
    ) -> Result<()>;

    /// Record a fine-grained lifecycle event.
    fn record_event(&self, event: &InteractionEvent) -> Result<()>;

    /// Load a session by ID.
    fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>>;

    /// Load all turns for a session, ordered by turn_number ascending.
    fn load_turns_for_session(&self, session_id: &str) -> Result<Vec<InteractionTurn>>;

    /// Load turns that have not yet been assigned to a checkpoint.
    fn pending_turns_for_session(&self, session_id: &str) -> Result<Vec<InteractionTurn>>;

    /// Link a set of turns to a derived checkpoint.
    fn assign_checkpoint_to_turns(&self, turn_ids: &[&str], checkpoint_id: &str) -> Result<()>;
}
