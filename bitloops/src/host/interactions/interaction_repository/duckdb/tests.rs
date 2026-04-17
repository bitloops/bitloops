use super::repository::DuckDbInteractionRepository;
use super::schema::duckdb_table_pk_columns;
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventType, InteractionSession, InteractionTurn,
};

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
        ..Default::default()
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

#[test]
fn shared_duckdb_allows_same_session_and_turn_ids_across_repos() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("events.duckdb");
    let repository_a = DuckDbInteractionRepository {
        repo_id: "repo-a".into(),
        path: path.clone(),
    };
    let repository_b = DuckDbInteractionRepository {
        repo_id: "repo-b".into(),
        path,
    };

    repository_a.ensure_schema().expect("schema repo a");
    repository_b.ensure_schema().expect("schema repo b");

    let pk_columns = duckdb_table_pk_columns(
        &duckdb::Connection::open(temp_dir.path().join("events.duckdb")).expect("open duckdb"),
        "interaction_sessions",
    )
    .expect("read session primary key columns");
    assert_eq!(
        pk_columns,
        vec!["repo_id".to_string(), "session_id".to_string()]
    );

    let mut session_a = sample_session();
    session_a.repo_id = "repo-a".into();
    session_a.first_prompt = "repo a".into();
    let mut session_b = sample_session();
    session_b.repo_id = "repo-b".into();
    session_b.first_prompt = "repo b".into();

    let mut turn_a = sample_turn();
    turn_a.repo_id = "repo-a".into();
    turn_a.prompt = "repo a turn".into();
    let mut turn_b = sample_turn();
    turn_b.repo_id = "repo-b".into();
    turn_b.prompt = "repo b turn".into();

    repository_a
        .upsert_session(&session_a)
        .expect("upsert session a");
    repository_b
        .upsert_session(&session_b)
        .expect("upsert session b");
    repository_a.upsert_turn(&turn_a).expect("upsert turn a");
    repository_b.upsert_turn(&turn_b).expect("upsert turn b");

    assert_eq!(
        repository_a
            .load_session("sess-1")
            .expect("load session a")
            .expect("session a")
            .first_prompt,
        "repo a"
    );
    assert_eq!(
        repository_b
            .load_session("sess-1")
            .expect("load session b")
            .expect("session b")
            .first_prompt,
        "repo b"
    );
    assert_eq!(
        repository_a.list_turns_for_session("sess-1", 10).unwrap()[0].prompt,
        "repo a turn"
    );
    assert_eq!(
        repository_b.list_turns_for_session("sess-1", 10).unwrap()[0].prompt,
        "repo b turn"
    );
}
