use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};

use super::*;
use crate::host::checkpoints::session::state::PendingCheckpointState;
use crate::host::interactions::store::{InteractionEventRepository, InteractionSpool};
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventFilter, InteractionSession, InteractionTurn,
};

type ClickHouseTestEnv = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

#[derive(Default)]
struct FakeInteractionRepository {
    repo_id: String,
    sessions: Mutex<HashMap<String, InteractionSession>>,
    turns: Mutex<HashMap<String, InteractionTurn>>,
    operations: Arc<Mutex<Vec<&'static str>>>,
    fail_list_uncheckpointed_turns: bool,
}

impl FakeInteractionRepository {
    fn new(repo_id: &str, operations: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            repo_id: repo_id.to_string(),
            operations,
            ..Default::default()
        }
    }

    fn with_session(self, session: InteractionSession) -> Self {
        self.sessions
            .lock()
            .expect("lock sessions")
            .insert(session.session_id.clone(), session);
        self
    }

    fn with_turn(self, turn: InteractionTurn) -> Self {
        self.turns
            .lock()
            .expect("lock turns")
            .insert(turn.turn_id.clone(), turn);
        self
    }

    fn checkpoint_id_for(&self, turn_id: &str) -> Option<String> {
        self.turns
            .lock()
            .expect("lock turns")
            .get(turn_id)
            .and_then(|turn| turn.checkpoint_id.clone())
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
struct FakeInteractionSpool {
    repo_id: String,
    pending_mutations: bool,
    flush_error: Option<String>,
    operations: Arc<Mutex<Vec<&'static str>>>,
    assigned_turns: Mutex<Vec<String>>,
}

impl FakeInteractionSpool {
    fn new(repo_id: &str) -> Self {
        Self {
            repo_id: repo_id.to_string(),
            ..Default::default()
        }
    }

    fn assigned_turns(&self) -> Vec<String> {
        self.assigned_turns
            .lock()
            .expect("lock assigned turns")
            .clone()
    }
}

impl InteractionSpool for FakeInteractionSpool {
    fn repo_id(&self) -> &str {
        &self.repo_id
    }

    fn record_session(&self, _session: &InteractionSession) -> Result<()> {
        Ok(())
    }

    fn record_turn(&self, _turn: &InteractionTurn) -> Result<()> {
        Ok(())
    }

    fn record_event(&self, _event: &InteractionEvent) -> Result<()> {
        Ok(())
    }

    fn assign_checkpoint_to_turns(
        &self,
        turn_ids: &[String],
        _checkpoint_id: &str,
        _assigned_at: &str,
    ) -> Result<()> {
        self.assigned_turns
            .lock()
            .expect("lock assigned turns")
            .extend(turn_ids.iter().cloned());
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
        Ok(Vec::new())
    }

    fn load_session(&self, _session_id: &str) -> Result<Option<InteractionSession>> {
        Ok(None)
    }

    fn list_turns_for_session(
        &self,
        _session_id: &str,
        _limit: usize,
    ) -> Result<Vec<InteractionTurn>> {
        Ok(Vec::new())
    }

    fn list_uncheckpointed_turns(&self) -> Result<Vec<InteractionTurn>> {
        Ok(Vec::new())
    }

    fn list_events(
        &self,
        _filter: &InteractionEventFilter,
        _limit: usize,
    ) -> Result<Vec<InteractionEvent>> {
        Ok(Vec::new())
    }
}

fn fake_interaction_session(
    repo_root: &Path,
    repo_id: &str,
    session_id: &str,
) -> InteractionSession {
    InteractionSession {
        session_id: session_id.to_string(),
        repo_id: repo_id.to_string(),
        agent_type: "codex".to_string(),
        model: "gpt-5.4".to_string(),
        first_prompt: "ship it".to_string(),
        transcript_path: repo_root
            .join("transcript.jsonl")
            .to_string_lossy()
            .to_string(),
        worktree_path: repo_root.to_string_lossy().to_string(),
        worktree_id: "main".to_string(),
        started_at: "2026-04-05T10:00:00Z".to_string(),
        last_event_at: "2026-04-05T10:00:01Z".to_string(),
        updated_at: "2026-04-05T10:00:01Z".to_string(),
        ..Default::default()
    }
}

fn fake_interaction_turn(
    repo_id: &str,
    session_id: &str,
    turn_id: &str,
    files: &[&str],
) -> InteractionTurn {
    InteractionTurn {
        turn_id: turn_id.to_string(),
        session_id: session_id.to_string(),
        repo_id: repo_id.to_string(),
        turn_number: 1,
        prompt: "make the change".to_string(),
        agent_type: "codex".to_string(),
        model: "gpt-5.4".to_string(),
        started_at: "2026-04-05T10:00:01Z".to_string(),
        ended_at: Some("2026-04-05T10:00:02Z".to_string()),
        token_usage: Some(TokenUsageMetadata {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        }),
        summary: format!("summary for {turn_id}"),
        prompt_count: 1,
        transcript_offset_start: Some(0),
        transcript_offset_end: Some(1),
        transcript_fragment: format!(
            "{{\"type\":\"user\",\"content\":\"make the change {turn_id}\"}}\n{{\"type\":\"assistant\",\"content\":\"done {turn_id}\"}}\n"
        ),
        files_modified: files.iter().map(|file| file.to_string()).collect(),
        updated_at: "2026-04-05T10:00:02Z".to_string(),
        ..Default::default()
    }
}

fn clickhouse_test_env() -> Option<ClickHouseTestEnv> {
    let url = std::env::var("BITLOOPS_TEST_CLICKHOUSE_URL").ok()?;
    let user = std::env::var("BITLOOPS_TEST_CLICKHOUSE_USER").ok();
    let password = std::env::var("BITLOOPS_TEST_CLICKHOUSE_PASSWORD").ok();
    let database = std::env::var("BITLOOPS_TEST_CLICKHOUSE_DATABASE").ok();
    Some((url, user, password, database))
}

#[test]
pub(crate) fn derive_post_commit_from_event_db_turns_with_fake_sources() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    std::fs::write(dir.path().join("transcript.jsonl"), "{}\n").unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let operations = Arc::new(Mutex::new(Vec::new()));
    let repository = FakeInteractionRepository::new(&repo_id, Arc::clone(&operations))
        .with_session(fake_interaction_session(dir.path(), &repo_id, "sess-1"))
        .with_turn(fake_interaction_turn(
            &repo_id,
            "sess-1",
            "turn-1",
            &["change.txt"],
        ));
    let mut spool = FakeInteractionSpool::new(&repo_id);
    spool.operations = Arc::clone(&operations);

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(
        dir.path(),
        &["commit", "-m", "derive checkpoint from fake event repo"],
    );
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");

    let strategy = ManualCommitStrategy::new(dir.path());
    let checkpoint_id = strategy
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .expect("derive from event db")
        .expect("checkpoint id");

    let summary = read_committed(dir.path(), &checkpoint_id)
        .expect("read derived checkpoint")
        .expect("derived checkpoint summary");
    assert_eq!(summary.files_touched, vec!["change.txt"]);
    assert_eq!(
        repository.checkpoint_id_for("turn-1").as_deref(),
        Some(checkpoint_id.as_str())
    );
    assert_eq!(spool.assigned_turns(), vec!["turn-1".to_string()]);

    let sequence = operations.lock().expect("lock operations").clone();
    assert_eq!(
        sequence[..2],
        ["spool.flush", "repo.list_uncheckpointed_turns"],
        "post_commit should flush the spool before reading the Event DB"
    );
}

#[test]
pub(crate) fn derive_post_commit_scopes_committed_transcript_from_turn_offsets() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    std::fs::write(
        dir.path().join("transcript.jsonl"),
        "{\"role\":\"user\",\"content\":\"before\"}\n{\"role\":\"user\",\"content\":\"captured prompt\"}\n{\"role\":\"assistant\",\"content\":\"captured answer\"}\n{\"role\":\"assistant\",\"content\":\"after\"}\n",
    )
    .unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let session = fake_interaction_session(dir.path(), &repo_id, "sess-sliced");
    let mut turn = fake_interaction_turn(&repo_id, "sess-sliced", "turn-sliced", &["change.txt"]);
    turn.prompt = "captured prompt".into();
    turn.transcript_offset_start = Some(1);
    turn.transcript_offset_end = Some(3);
    turn.transcript_fragment =
        "{\"role\":\"user\",\"content\":\"captured prompt\"}\n{\"role\":\"assistant\",\"content\":\"captured answer\"}\n".into();

    let repository = FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new())))
        .with_session(session)
        .with_turn(turn);
    let spool = FakeInteractionSpool::new(&repo_id);

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "derive sliced transcript"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");

    let checkpoint_id = ManualCommitStrategy::new(dir.path())
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .expect("derive from interaction source")
        .expect("checkpoint id");

    let session_content =
        read_session_content(dir.path(), &checkpoint_id, 0).expect("read session content");
    assert!(
        session_content.transcript.contains("captured prompt"),
        "scoped transcript should contain the recorded turn prompt"
    );
    assert!(
        session_content.transcript.contains("captured answer"),
        "scoped transcript should contain the recorded turn answer"
    );
    assert!(
        !session_content.transcript.contains("before"),
        "scoped transcript should exclude content before the recorded turn offsets"
    );
    assert!(
        !session_content.transcript.contains("after"),
        "scoped transcript should exclude content after the recorded turn offsets"
    );
}

#[test]
pub(crate) fn derive_post_commit_uses_event_native_transcript_when_turn_offsets_are_missing() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    let transcript_path = dir.path().join("transcript.jsonl");
    std::fs::write(
        &transcript_path,
        "{\"role\":\"user\",\"content\":\"before\"}\n{\"role\":\"user\",\"content\":\"captured prompt\"}\n{\"role\":\"assistant\",\"content\":\"captured answer\"}\n",
    )
    .unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let session = fake_interaction_session(dir.path(), &repo_id, "sess-fallback");
    let mut turn =
        fake_interaction_turn(&repo_id, "sess-fallback", "turn-fallback", &["change.txt"]);
    turn.prompt = "captured prompt".into();
    turn.transcript_offset_start = None;
    turn.transcript_offset_end = None;
    turn.transcript_fragment =
        "{\"role\":\"user\",\"content\":\"captured prompt\"}\n{\"role\":\"assistant\",\"content\":\"captured answer\"}\n".into();

    let repository = FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new())))
        .with_session(session)
        .with_turn(turn);
    let spool = FakeInteractionSpool::new(&repo_id);

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "derive transcript fallback"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");
    std::fs::remove_file(&transcript_path).expect("delete transcript file");

    let checkpoint_id = ManualCommitStrategy::new(dir.path())
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .expect("derive from interaction source")
        .expect("checkpoint id");

    let session_content =
        read_session_content(dir.path(), &checkpoint_id, 0).expect("read session content");
    assert!(
        session_content.transcript.contains("captured answer"),
        "event-native transcript should include the recorded turn content"
    );
    assert!(
        !session_content.transcript.contains("before"),
        "checkpoint derivation should no longer fall back to the transcript file"
    );
}

#[test]
pub(crate) fn derive_post_commit_errors_when_overlapping_turn_is_missing_transcript_fragment() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let session = fake_interaction_session(dir.path(), &repo_id, "sess-missing-fragment");
    let mut turn = fake_interaction_turn(
        &repo_id,
        "sess-missing-fragment",
        "turn-missing-fragment",
        &["change.txt"],
    );
    turn.transcript_fragment.clear();
    let repository = FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new())))
        .with_session(session)
        .with_turn(turn);
    let spool = FakeInteractionSpool::new(&repo_id);

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "missing transcript fragment"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");

    let err = ManualCommitStrategy::new(dir.path())
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .unwrap_err();

    assert!(
        err.to_string().contains("missing transcript_fragment"),
        "unexpected error: {err}"
    );
    assert!(
        !read_commit_checkpoint_mappings(dir.path())
            .expect("read mappings")
            .contains_key(&head),
        "failed derivation must not write a commit mapping"
    );
}

#[test]
pub(crate) fn derive_post_commit_ignores_session_state_without_interaction_turns() {
    let dir = tempfile::tempdir().unwrap();
    let head_before = setup_git_repo(&dir);
    init_devql_schema(dir.path());
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "sess-state-only".to_string(),
            phase: SessionPhase::Idle,
            base_commit: head_before,
            pending: PendingCheckpointState {
                step_count: 2,
                files_touched: vec!["state-only.txt".to_string()],
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();

    std::fs::write(
        dir.path().join("state-only.txt"),
        "pending but uncaptured\n",
    )
    .unwrap();
    git_ok(dir.path(), &["add", "state-only.txt"]);
    git_ok(dir.path(), &["commit", "-m", "state only pending work"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let repository = FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new())));
    let spool = FakeInteractionSpool::new(&repo_id);

    let strategy = ManualCommitStrategy::new(dir.path());
    let checkpoint_id = strategy
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .expect("derive without interaction turns");

    assert!(checkpoint_id.is_none());
    assert!(
        !read_commit_checkpoint_mappings(dir.path())
            .expect("mappings")
            .contains_key(&head),
        "session-state-only pending work must not derive a checkpoint"
    );
}

#[test]
pub(crate) fn derive_post_commit_returns_error_when_spool_flush_fails_with_pending_work() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let repository = FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new())));
    let spool = FakeInteractionSpool {
        repo_id,
        pending_mutations: true,
        flush_error: Some("forced flush failure".to_string()),
        ..FakeInteractionSpool::default()
    };

    let strategy = ManualCommitStrategy::new(dir.path());
    let err = strategy
        .derive_post_commit_from_interaction_sources(
            "deadbeef",
            &HashSet::new(),
            false,
            &repository,
            Some(&spool),
        )
        .expect_err("flush failure with pending work should error");
    assert!(
        format!("{err:#}").contains("flushing interaction spool before post_commit derivation")
    );
}

#[test]
pub(crate) fn derive_post_commit_returns_error_when_overlapping_turn_session_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    std::fs::write(dir.path().join("transcript.jsonl"), "{}\n").unwrap();

    std::fs::write(dir.path().join("change.txt"), "hello\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "missing interaction session"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let repository =
        FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new()))).with_turn(
            fake_interaction_turn(&repo_id, "missing-session", "turn-1", &["change.txt"]),
        );
    let spool = FakeInteractionSpool::new(&repo_id);

    let strategy = ManualCommitStrategy::new(dir.path());
    let err = strategy
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .expect_err("missing interaction session should error");
    assert!(format!("{err:#}").contains("missing interaction session"));
}

#[test]
#[ignore = "ad hoc DuckDB integration coverage"]
pub(crate) fn post_commit_derives_checkpoint_from_interaction_turns_duckdb_integration() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    seed_interaction_turn(dir.path(), "sess-1", "turn-1", &["change.txt"]);

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(
        dir.path(),
        &["commit", "-m", "derive checkpoint from interaction"],
    );
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    std::fs::remove_file(dir.path().join("transcript.jsonl")).expect("delete transcript file");

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().expect("post_commit should succeed");

    let checkpoint_id = read_commit_checkpoint_mappings(dir.path())
        .expect("mappings")
        .get(&head)
        .cloned()
        .expect("checkpoint mapping for derived commit");
    let summary = read_committed(dir.path(), &checkpoint_id)
        .expect("read derived checkpoint")
        .expect("derived checkpoint summary");
    assert_eq!(summary.files_touched, vec!["change.txt"]);

    let turns = open_test_spool(dir.path())
        .list_turns_for_session("sess-1", 10)
        .expect("list turns after derivation");
    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].checkpoint_id.as_deref(),
        Some(checkpoint_id.as_str())
    );
}

#[test]
#[ignore = "ad hoc DuckDB integration coverage"]
pub(crate) fn post_commit_skips_non_overlapping_interaction_turns_duckdb_integration() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    seed_interaction_turn(dir.path(), "sess-2", "turn-2", &["design-notes.md"]);

    std::fs::write(dir.path().join("change.txt"), "real commit\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "non-overlapping interaction"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().expect("post_commit should succeed");

    let mappings = read_commit_checkpoint_mappings(dir.path()).expect("mappings");
    assert!(
        !mappings.contains_key(&head),
        "non-overlapping interaction turn should not derive a checkpoint"
    );

    let turns = open_test_spool(dir.path())
        .list_turns_for_session("sess-2", 10)
        .expect("list turns after skipped derivation");
    assert_eq!(turns.len(), 1);
    assert!(turns[0].checkpoint_id.is_none());
}

#[test]
#[ignore = "ad hoc ClickHouse integration coverage"]
pub(crate) fn post_commit_derives_checkpoint_from_interaction_turns_clickhouse_integration() {
    let Some((url, user, password, database)) = clickhouse_test_env() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema_with_clickhouse(
        dir.path(),
        &url,
        user.as_deref(),
        password.as_deref(),
        database.as_deref(),
    );
    seed_interaction_turn(dir.path(), "sess-ch-1", "turn-ch-1", &["change.txt"]);

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(
        dir.path(),
        &["commit", "-m", "derive checkpoint from interaction"],
    );
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    std::fs::remove_file(dir.path().join("transcript.jsonl")).expect("delete transcript file");

    ManualCommitStrategy::new(dir.path())
        .post_commit()
        .expect("post_commit should succeed");

    let checkpoint_id = read_commit_checkpoint_mappings(dir.path())
        .expect("mappings")
        .get(&head)
        .cloned()
        .expect("checkpoint mapping for derived commit");
    let summary = read_committed(dir.path(), &checkpoint_id)
        .expect("read derived checkpoint")
        .expect("derived checkpoint summary");
    assert_eq!(summary.files_touched, vec!["change.txt"]);

    let turns = open_test_spool(dir.path())
        .list_turns_for_session("sess-ch-1", 10)
        .expect("list turns after derivation");
    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].checkpoint_id.as_deref(),
        Some(checkpoint_id.as_str())
    );
}

#[test]
#[ignore = "ad hoc DuckDB integration coverage"]
pub(crate) fn post_commit_errors_when_duckdb_interaction_turn_is_missing_transcript_fragment() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    seed_interaction_turn_with_fragment(
        dir.path(),
        "sess-missing-fragment-db",
        "turn-missing-fragment-db",
        &["change.txt"],
        "",
    );

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "missing transcript fragment"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let err = ManualCommitStrategy::new(dir.path())
        .post_commit()
        .expect_err("post_commit should fail when transcript fragments are missing");
    assert!(err.to_string().contains("missing transcript_fragment"));
    assert!(
        !read_commit_checkpoint_mappings(dir.path())
            .expect("mappings")
            .contains_key(&head),
        "failed derivation must not write a commit mapping"
    );
}

#[test]
#[ignore = "ad hoc ClickHouse integration coverage"]
pub(crate) fn post_commit_skips_non_overlapping_interaction_turns_clickhouse_integration() {
    let Some((url, user, password, database)) = clickhouse_test_env() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema_with_clickhouse(
        dir.path(),
        &url,
        user.as_deref(),
        password.as_deref(),
        database.as_deref(),
    );
    seed_interaction_turn(dir.path(), "sess-ch-2", "turn-ch-2", &["design-notes.md"]);

    std::fs::write(dir.path().join("change.txt"), "real commit\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "non-overlapping interaction"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    ManualCommitStrategy::new(dir.path())
        .post_commit()
        .expect("post_commit should succeed");

    let mappings = read_commit_checkpoint_mappings(dir.path()).expect("mappings");
    assert!(
        !mappings.contains_key(&head),
        "non-overlapping interaction turn should not derive a checkpoint"
    );

    let turns = open_test_spool(dir.path())
        .list_turns_for_session("sess-ch-2", 10)
        .expect("list turns after skipped derivation");
    assert_eq!(turns.len(), 1);
    assert!(turns[0].checkpoint_id.is_none());
}

#[test]
#[ignore = "ad hoc ClickHouse integration coverage"]
pub(crate) fn post_commit_errors_when_clickhouse_interaction_turn_is_missing_transcript_fragment() {
    let Some((url, user, password, database)) = clickhouse_test_env() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema_with_clickhouse(
        dir.path(),
        &url,
        user.as_deref(),
        password.as_deref(),
        database.as_deref(),
    );
    seed_interaction_turn_with_fragment(
        dir.path(),
        "sess-ch-missing-fragment",
        "turn-ch-missing-fragment",
        &["change.txt"],
        "",
    );

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "missing transcript fragment"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let err = ManualCommitStrategy::new(dir.path())
        .post_commit()
        .expect_err("post_commit should fail when transcript fragments are missing");
    assert!(err.to_string().contains("missing transcript_fragment"));
    assert!(
        !read_commit_checkpoint_mappings(dir.path())
            .expect("mappings")
            .contains_key(&head),
        "failed derivation must not write a commit mapping"
    );
}
