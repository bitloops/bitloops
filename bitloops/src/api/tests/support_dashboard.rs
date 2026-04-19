use super::*;

pub(super) fn test_state(
    repo_root: PathBuf,
    mode: ServeMode,
    bundle_dir: PathBuf,
) -> DashboardState {
    let db = crate::api::DashboardDbPools::default();
    DashboardState {
        config_path: repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        config_root: repo_root.clone(),
        repo_registry_path: None,
        subscription_hub: crate::graphql::SubscriptionHub::new_arc(),
        dashboard_graphql_schema: crate::api::dashboard_schema::build_dashboard_schema_template(),
        runtime_graphql_schema: crate::api::runtime_schema::build_runtime_schema_template(),
        devql_schema: crate::graphql::build_global_schema_template(),
        devql_slim_schema: crate::graphql::build_slim_schema_template(),
        repo_root,
        mode,
        db,
        bundle_dir,
        bundle_source_overrides: crate::api::DashboardBundleSourceOverrides::default(),
    }
}

pub(super) fn seed_dashboard_repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    crate::test_support::git_fixtures::write_test_daemon_config(repo_root);

    fs::write(repo_root.join("app.rs"), "fn main() {}\n").expect("write app.rs");
    git_ok(repo_root, &["add", "app.rs"]);
    git_ok(repo_root, &["commit", "-m", "Initial commit"]);

    fs::write(
        repo_root.join("app.rs"),
        "fn main() { println!(\"ok\"); }\n",
    )
    .expect("update app.rs");
    git_ok(repo_root, &["add", "app.rs"]);
    git_ok(repo_root, &["commit", "-m", "Checkpoint commit"]);
    let checkpoint_commit_sha = git_ok(repo_root, &["rev-parse", "HEAD"]);
    seed_repository_catalog_row(repo_root, SEEDED_REPO_NAME, "main");

    git_ok(
        repo_root,
        &["checkout", "--orphan", "bitloops/checkpoints/v1"],
    );
    let checkpoint_bucket = repo_root.join("aa").join("bbccddeeff");
    fs::create_dir_all(checkpoint_bucket.join("0")).expect("create checkpoint directories");

    let top_metadata = json!({
        "checkpoint_id": "aabbccddeeff",
        "strategy": "manual-commit",
        "branch": "main",
        "checkpoints_count": 2,
        "files_touched": ["app.rs"],
        "sessions": [{
            "metadata": "/aa/bbccddeeff/0/metadata.json",
            "transcript": "/aa/bbccddeeff/0/full.jsonl",
            "context": "/aa/bbccddeeff/0/context.md",
            "content_hash": "/aa/bbccddeeff/0/content_hash.txt",
            "prompt": "/aa/bbccddeeff/0/prompt.txt"
        }],
        "token_usage": {
            "input_tokens": 100,
            "output_tokens": 40,
            "cache_creation_tokens": 10,
            "cache_read_tokens": 5,
            "api_call_count": 3
        }
    });
    let session_metadata = json!({
        "checkpoint_id": "aabbccddeeff",
        "session_id": "session-1",
        "checkpoints_count": 2,
        "strategy": "manual-commit",
        "agent": "claude-code",
        "created_at": "2026-02-27T12:00:00Z",
        "cli_version": "0.0.3",
        "files_touched": ["app.rs"],
        "is_task": false,
        "tool_use_id": ""
    });
    fs::write(
        checkpoint_bucket.join("metadata.json"),
        serde_json::to_string_pretty(&top_metadata).expect("serialize top metadata"),
    )
    .expect("write top metadata");
    fs::write(
        checkpoint_bucket.join("0").join("metadata.json"),
        serde_json::to_string_pretty(&session_metadata).expect("serialize session metadata"),
    )
    .expect("write session metadata");
    let transcript_payload = "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Build dashboard API\"}]}}\n\
{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Implemented\"},{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"dashboard.rs\"}}]}}\n";
    let prompt_payload = "Build dashboard API";
    let context_payload = "Repository context";
    fs::write(
        checkpoint_bucket.join("0").join("full.jsonl"),
        transcript_payload,
    )
    .expect("write transcript");
    fs::write(
        checkpoint_bucket.join("0").join("prompt.txt"),
        prompt_payload,
    )
    .expect("write prompt");
    fs::write(
        checkpoint_bucket.join("0").join("context.md"),
        context_payload,
    )
    .expect("write context");

    git_ok(repo_root, &["add", "aa"]);
    git_ok(repo_root, &["commit", "-m", "checkpoint metadata"]);
    git_ok(repo_root, &["checkout", "main"]);

    seed_checkpoint_storage_for_dashboard(
        repo_root,
        SeedCheckpointStorage {
            commit_sha: &checkpoint_commit_sha,
            checkpoint_id: "aabbccddeeff",
            branch: "main",
            files_touched: &["app.rs"],
            checkpoints_count: 2,
            token_usage: json!({
                "input_tokens": 100,
                "output_tokens": 40,
                "cache_creation_tokens": 10,
                "cache_read_tokens": 5,
                "api_call_count": 3
            }),
            sessions: &[SeedCheckpointSession {
                session_index: 0,
                session_id: "session-1",
                agent: "claude-code",
                created_at: "2026-02-27T12:00:00Z",
                checkpoints_count: 2,
                transcript: transcript_payload,
                prompts: prompt_payload,
                context: context_payload,
            }],
            insert_mapping: true,
        },
    );
    seed_dashboard_commit_row(repo_root, &checkpoint_commit_sha);

    seed_dashboard_interactions(repo_root, "aabbccddeeff");

    dir
}

pub(super) fn seed_dashboard_repo_with_duckdb_events() -> TempDir {
    let repo = seed_dashboard_repo();
    let head_commit = git_ok(repo.path(), &["rev-parse", "HEAD"]);
    let previous_commit = git_ok(repo.path(), &["rev-parse", "HEAD^"]);

    seed_checkpoint_storage_for_dashboard(
        repo.path(),
        SeedCheckpointStorage {
            commit_sha: &previous_commit,
            checkpoint_id: "checkpoint-dashboard-previous",
            branch: "main",
            files_touched: &["app.rs"],
            checkpoints_count: 1,
            token_usage: json!({
                "input_tokens": 20,
                "output_tokens": 10,
                "cache_creation_tokens": 0,
                "cache_read_tokens": 0,
                "api_call_count": 1
            }),
            sessions: &[SeedCheckpointSession {
                session_index: 0,
                session_id: "session-dashboard-previous",
                agent: "codex",
                created_at: "2026-03-26T09:15:00Z",
                checkpoints_count: 1,
                transcript: "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Checkpoint previous\"}]}}\n",
                prompts: "Checkpoint previous",
                context: "Previous checkpoint context",
            }],
            insert_mapping: true,
        },
    );

    seed_duckdb_events(
        repo.path(),
        &[
            SeedGraphqlEvent {
                event_id: "evt-dashboard-head",
                event_time: "2026-03-26T09:30:00Z",
                checkpoint_id: "checkpoint-dashboard-head",
                session_id: "session-dashboard-head",
                commit_sha: &head_commit,
                branch: "main",
                event_type: "checkpoint_committed",
                agent: "codex",
                strategy: "manual-commit",
                files_touched: &["app.rs"],
                payload: json!({"source": "dashboard-head"}),
            },
            SeedGraphqlEvent {
                event_id: "evt-dashboard-previous",
                event_time: "2026-03-26T09:15:00Z",
                checkpoint_id: "checkpoint-dashboard-previous",
                session_id: "session-dashboard-previous",
                commit_sha: &previous_commit,
                branch: "main",
                event_type: "checkpoint_committed",
                agent: "codex",
                strategy: "manual-commit",
                files_touched: &["app.rs"],
                payload: json!({"source": "dashboard-previous"}),
            },
        ],
    );

    repo
}

fn seed_dashboard_interactions(repo_root: &Path, checkpoint_id: &str) {
    use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
    use crate::host::interactions::db_store::{SqliteInteractionSpool, interaction_spool_db_path};
    use crate::host::interactions::store::InteractionSpool;
    use crate::host::interactions::types::{
        InteractionEvent, InteractionEventType, InteractionSession, InteractionTurn,
    };
    use crate::storage::sqlite::SqliteConnectionPool;

    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .expect("resolve repo identity for seeded interactions")
        .repo_id;
    let spool_path = interaction_spool_db_path(repo_root).expect("resolve interaction spool path");
    let sqlite = SqliteConnectionPool::connect(spool_path).expect("connect interaction spool");
    let spool = SqliteInteractionSpool::new(sqlite, repo_id).expect("initialise interaction spool");

    let transcript_path = repo_root.join("interaction-transcript.jsonl");
    fs::write(
        &transcript_path,
        "{\"type\":\"user\",\"content\":\"Build dashboard API\"}\n{\"type\":\"assistant\",\"content\":\"Implemented dashboard API\"}\n",
    )
    .expect("write interaction transcript");

    let session = InteractionSession {
        session_id: "session-1".to_string(),
        repo_id: spool.repo_id().to_string(),
        branch: "main".to_string(),
        actor_id: "actor-1".to_string(),
        actor_name: "Alice".to_string(),
        actor_email: "alice@example.com".to_string(),
        actor_source: "bitloops-session".to_string(),
        agent_type: "claude-code".to_string(),
        model: "gpt-5.4".to_string(),
        first_prompt: "Build dashboard API".to_string(),
        transcript_path: transcript_path.to_string_lossy().to_string(),
        worktree_path: repo_root.to_string_lossy().to_string(),
        worktree_id: "main".to_string(),
        started_at: "2026-02-27T12:00:00Z".to_string(),
        ended_at: Some("2026-02-27T12:05:00Z".to_string()),
        last_event_at: "2026-02-27T12:05:00Z".to_string(),
        updated_at: "2026-02-27T12:05:00Z".to_string(),
    };
    spool
        .record_session(&session)
        .expect("record interaction session");

    let turn = InteractionTurn {
        turn_id: "turn-1".to_string(),
        session_id: session.session_id.clone(),
        repo_id: spool.repo_id().to_string(),
        branch: "main".to_string(),
        actor_id: "actor-1".to_string(),
        actor_name: "Alice".to_string(),
        actor_email: "alice@example.com".to_string(),
        actor_source: "bitloops-session".to_string(),
        turn_number: 1,
        prompt: "Build dashboard API".to_string(),
        agent_type: "claude-code".to_string(),
        model: "gpt-5.4".to_string(),
        started_at: "2026-02-27T12:00:10Z".to_string(),
        ended_at: Some("2026-02-27T12:02:00Z".to_string()),
        token_usage: Some(TokenUsageMetadata {
            input_tokens: 100,
            output_tokens: 40,
            cache_creation_tokens: 10,
            cache_read_tokens: 5,
            api_call_count: 3,
            subagent_tokens: None,
        }),
        summary: "Implemented dashboard API".to_string(),
        prompt_count: 1,
        transcript_offset_start: Some(0),
        transcript_offset_end: Some(2),
        transcript_fragment:
            "{\"type\":\"user\",\"content\":\"Build dashboard API\"}\n{\"type\":\"assistant\",\"content\":\"Implemented dashboard API\"}\n"
                .to_string(),
        files_modified: vec!["app.rs".to_string()],
        checkpoint_id: None,
        updated_at: "2026-02-27T12:02:00Z".to_string(),
    };
    spool.record_turn(&turn).expect("record interaction turn");

    let events = [
        InteractionEvent {
            event_id: "event-session-start".to_string(),
            session_id: session.session_id.clone(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "bitloops-session".to_string(),
            event_type: InteractionEventType::SessionStart,
            event_time: "2026-02-27T12:00:00Z".to_string(),
            agent_type: "claude-code".to_string(),
            model: "gpt-5.4".to_string(),
            payload: json!({"first_prompt": "Build dashboard API"}),
            ..Default::default()
        },
        InteractionEvent {
            event_id: "event-turn-start".to_string(),
            session_id: session.session_id.clone(),
            turn_id: Some(turn.turn_id.clone()),
            repo_id: spool.repo_id().to_string(),
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "bitloops-session".to_string(),
            event_type: InteractionEventType::TurnStart,
            event_time: "2026-02-27T12:00:10Z".to_string(),
            agent_type: "claude-code".to_string(),
            model: "gpt-5.4".to_string(),
            payload: json!({"prompt": "Build dashboard API"}),
            ..Default::default()
        },
        InteractionEvent {
            event_id: "event-subagent-start".to_string(),
            session_id: session.session_id.clone(),
            turn_id: Some(turn.turn_id.clone()),
            repo_id: spool.repo_id().to_string(),
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "bitloops-session".to_string(),
            event_type: InteractionEventType::SubagentStart,
            event_time: "2026-02-27T12:00:30Z".to_string(),
            agent_type: "claude-code".to_string(),
            model: "gpt-5.4".to_string(),
            tool_use_id: "tool-use-1".to_string(),
            tool_kind: "edit".to_string(),
            task_description: "Update dashboard GraphQL types".to_string(),
            subagent_id: "subagent-1".to_string(),
            payload: json!({"tool_kind": "edit"}),
        },
        InteractionEvent {
            event_id: "event-subagent-end".to_string(),
            session_id: session.session_id.clone(),
            turn_id: Some(turn.turn_id.clone()),
            repo_id: spool.repo_id().to_string(),
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "bitloops-session".to_string(),
            event_type: InteractionEventType::SubagentEnd,
            event_time: "2026-02-27T12:01:00Z".to_string(),
            agent_type: "claude-code".to_string(),
            model: "gpt-5.4".to_string(),
            tool_use_id: "tool-use-1".to_string(),
            tool_kind: "edit".to_string(),
            task_description: "Update dashboard GraphQL types".to_string(),
            subagent_id: "subagent-1".to_string(),
            payload: json!({"status": "completed"}),
        },
        InteractionEvent {
            event_id: "event-turn-end".to_string(),
            session_id: session.session_id.clone(),
            turn_id: Some(turn.turn_id.clone()),
            repo_id: spool.repo_id().to_string(),
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "bitloops-session".to_string(),
            event_type: InteractionEventType::TurnEnd,
            event_time: "2026-02-27T12:02:00Z".to_string(),
            agent_type: "claude-code".to_string(),
            model: "gpt-5.4".to_string(),
            payload: json!({"summary": "Implemented dashboard API"}),
            ..Default::default()
        },
        InteractionEvent {
            event_id: "event-session-end".to_string(),
            session_id: session.session_id.clone(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "bitloops-session".to_string(),
            event_type: InteractionEventType::SessionEnd,
            event_time: "2026-02-27T12:05:00Z".to_string(),
            agent_type: "claude-code".to_string(),
            model: "gpt-5.4".to_string(),
            payload: json!({"status": "completed"}),
            ..Default::default()
        },
    ];
    for event in events {
        spool
            .record_event(&event)
            .expect("record interaction event");
    }

    spool
        .assign_checkpoint_to_turns(
            std::slice::from_ref(&turn.turn_id),
            checkpoint_id,
            "2026-02-27T12:02:30Z",
        )
        .expect("assign checkpoint to interaction turn");
}

fn seed_dashboard_commit_row(repo_root: &Path, commit_sha: &str) {
    let sqlite_path = checkpoint_sqlite_path(repo_root);
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(sqlite_path).expect("connect sqlite");
    sqlite
        .initialise_devql_schema()
        .expect("initialise devql schema for commit row");
    let repo_id = crate::host::devql::resolve_repo_id(repo_root).expect("resolve repo id");
    sqlite
        .with_connection(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO commits (
                    commit_sha, repo_id, author_name, author_email, commit_message, committed_at
                 ) VALUES (?1, ?2, 'Alice', 'alice@example.com', 'Checkpoint commit', '2026-02-27T12:05:00Z')",
                rusqlite::params![commit_sha, repo_id.as_str()],
            )?;
            Ok(())
        })
        .expect("insert commit row");
}
