use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};

use crate::host::interactions::store::{InteractionEventRepository, InteractionSpool};
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventFilter, InteractionSession, InteractionTurn,
};

#[derive(Default)]
pub(crate) struct FakeInteractionRepository {
    pub(crate) repo_id: String,
    sessions: Mutex<HashMap<String, InteractionSession>>,
    turns: Mutex<HashMap<String, InteractionTurn>>,
    operations: Arc<Mutex<Vec<&'static str>>>,
    pub(crate) fail_list_uncheckpointed_turns: bool,
}

impl FakeInteractionRepository {
    pub(crate) fn new(repo_id: &str, operations: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            repo_id: repo_id.to_string(),
            operations,
            ..Default::default()
        }
    }

    pub(crate) fn with_session(self, session: InteractionSession) -> Self {
        self.sessions
            .lock()
            .expect("lock sessions")
            .insert(session.session_id.clone(), session);
        self
    }

    pub(crate) fn with_turn(self, turn: InteractionTurn) -> Self {
        self.turns
            .lock()
            .expect("lock turns")
            .insert(turn.turn_id.clone(), turn);
        self
    }

    pub(crate) fn checkpoint_id_for(&self, turn_id: &str) -> Option<String> {
        self.turns
            .lock()
            .expect("lock turns")
            .get(turn_id)
            .and_then(|turn| turn.checkpoint_id.clone())
    }

    pub(crate) fn files_modified_for(&self, turn_id: &str) -> Vec<String> {
        self.turns
            .lock()
            .expect("lock turns")
            .get(turn_id)
            .map(|turn| turn.files_modified.clone())
            .unwrap_or_default()
    }
}

impl InteractionEventRepository for FakeInteractionRepository {
    fn repo_id(&self) -> &str {
        &self.repo_id
    }

    fn upsert_session(&self, session: &InteractionSession) -> Result<()> {
        self.sessions
            .lock()
            .expect("lock sessions")
            .insert(session.session_id.clone(), session.clone());
        Ok(())
    }

    fn upsert_turn(&self, turn: &InteractionTurn) -> Result<()> {
        self.turns
            .lock()
            .expect("lock turns")
            .insert(turn.turn_id.clone(), turn.clone());
        Ok(())
    }

    fn append_event(&self, _event: &InteractionEvent) -> Result<()> {
        Ok(())
    }

    fn assign_checkpoint_to_turns(
        &self,
        turn_ids: &[String],
        checkpoint_id: &str,
        assigned_at: &str,
    ) -> Result<()> {
        for turn_id in turn_ids {
            if let Some(turn) = self.turns.lock().expect("lock turns").get_mut(turn_id) {
                turn.checkpoint_id = Some(checkpoint_id.to_string());
                turn.updated_at = assigned_at.to_string();
            }
        }
        Ok(())
    }

    fn list_sessions(
        &self,
        _agent: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<InteractionSession>> {
        Ok(self
            .sessions
            .lock()
            .expect("lock sessions")
            .values()
            .cloned()
            .collect())
    }

    fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>> {
        self.operations
            .lock()
            .expect("lock operations")
            .push("repo.load_session");
        Ok(self
            .sessions
            .lock()
            .expect("lock sessions")
            .get(session_id)
            .cloned())
    }

    fn list_turns_for_session(
        &self,
        session_id: &str,
        _limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        Ok(self
            .turns
            .lock()
            .expect("lock turns")
            .values()
            .filter(|turn| turn.session_id == session_id)
            .cloned()
            .collect())
    }

    fn list_uncheckpointed_turns(&self) -> Result<Vec<InteractionTurn>> {
        self.operations
            .lock()
            .expect("lock operations")
            .push("repo.list_uncheckpointed_turns");
        if self.fail_list_uncheckpointed_turns {
            return Err(anyhow!("forced list_uncheckpointed_turns failure"));
        }
        Ok(self
            .turns
            .lock()
            .expect("lock turns")
            .values()
            .filter(|turn| turn.checkpoint_id.as_deref().unwrap_or("").is_empty())
            .cloned()
            .collect())
    }

    fn list_events(
        &self,
        _filter: &InteractionEventFilter,
        _limit: usize,
    ) -> Result<Vec<InteractionEvent>> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
pub(crate) struct FakeInteractionSpool {
    pub(crate) repo_id: String,
    pub(crate) pending_mutations: bool,
    pub(crate) flush_error: Option<String>,
    pub(crate) operations: Arc<Mutex<Vec<&'static str>>>,
    sessions: Mutex<HashMap<String, InteractionSession>>,
    turns: Mutex<HashMap<String, InteractionTurn>>,
}

impl FakeInteractionSpool {
    pub(crate) fn new(repo_id: &str) -> Self {
        Self {
            repo_id: repo_id.to_string(),
            ..Default::default()
        }
    }

    pub(crate) fn with_pending_mutations(mut self, pending_mutations: bool) -> Self {
        self.pending_mutations = pending_mutations;
        self
    }

    pub(crate) fn with_flush_error(mut self, message: impl Into<String>) -> Self {
        self.flush_error = Some(message.into());
        self
    }

    pub(crate) fn with_session(self, session: InteractionSession) -> Self {
        self.sessions
            .lock()
            .expect("lock spool sessions")
            .insert(session.session_id.clone(), session);
        self
    }

    pub(crate) fn with_turn(self, turn: InteractionTurn) -> Self {
        self.turns
            .lock()
            .expect("lock spool turns")
            .insert(turn.turn_id.clone(), turn);
        self
    }

    pub(crate) fn checkpoint_id_for(&self, turn_id: &str) -> Option<String> {
        self.turns
            .lock()
            .expect("lock spool turns")
            .get(turn_id)
            .and_then(|turn| turn.checkpoint_id.clone())
    }
}

impl InteractionSpool for FakeInteractionSpool {
    fn repo_id(&self) -> &str {
        &self.repo_id
    }

    fn record_session(&self, _session: &InteractionSession) -> Result<()> {
        self.sessions
            .lock()
            .expect("lock spool sessions")
            .insert(_session.session_id.clone(), _session.clone());
        Ok(())
    }

    fn record_turn(&self, turn: &InteractionTurn) -> Result<()> {
        self.turns
            .lock()
            .expect("lock spool turns")
            .insert(turn.turn_id.clone(), turn.clone());
        Ok(())
    }

    fn record_event(&self, _event: &InteractionEvent) -> Result<()> {
        Ok(())
    }

    fn assign_checkpoint_to_turns(
        &self,
        turn_ids: &[String],
        checkpoint_id: &str,
        assigned_at: &str,
    ) -> Result<()> {
        for turn_id in turn_ids {
            if let Some(turn) = self.turns.lock().expect("lock spool turns").get_mut(turn_id) {
                turn.checkpoint_id = Some(checkpoint_id.to_string());
                turn.updated_at = assigned_at.to_string();
            }
        }
        Ok(())
    }

    fn has_pending_mutations(&self) -> Result<bool> {
        Ok(self.pending_mutations)
    }

    fn flush(&self, _repository: &dyn InteractionEventRepository) -> Result<usize> {
        self.operations
            .lock()
            .expect("lock operations")
            .push("spool.flush");
        if let Some(message) = &self.flush_error {
            return Err(anyhow!(message.clone()));
        }
        Ok(0)
    }

    fn list_sessions(
        &self,
        _agent: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<InteractionSession>> {
        Ok(self
            .sessions
            .lock()
            .expect("lock spool sessions")
            .values()
            .cloned()
            .collect())
    }

    fn load_session(&self, session_id: &str) -> Result<Option<InteractionSession>> {
        self.operations
            .lock()
            .expect("lock spool operations")
            .push("spool.load_session");
        Ok(self
            .sessions
            .lock()
            .expect("lock spool sessions")
            .get(session_id)
            .cloned())
    }

    fn list_turns_for_session(
        &self,
        session_id: &str,
        _limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        Ok(self
            .turns
            .lock()
            .expect("lock spool turns")
            .values()
            .filter(|turn| turn.session_id == session_id)
            .cloned()
            .collect())
    }

    fn list_uncheckpointed_turns(&self) -> Result<Vec<InteractionTurn>> {
        self.operations
            .lock()
            .expect("lock spool operations")
            .push("spool.list_uncheckpointed_turns");
        Ok(self
            .turns
            .lock()
            .expect("lock spool turns")
            .values()
            .filter(|turn| turn.checkpoint_id.as_deref().unwrap_or("").is_empty())
            .cloned()
            .collect())
    }

    fn list_events(
        &self,
        _filter: &InteractionEventFilter,
        _limit: usize,
    ) -> Result<Vec<InteractionEvent>> {
        Ok(Vec::new())
    }
}
