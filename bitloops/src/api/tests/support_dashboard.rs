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
