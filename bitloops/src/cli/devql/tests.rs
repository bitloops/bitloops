use super::graphql::{
    RuntimeWatcherReconcileGraphqlRecord, reconcile_repo_watcher_via_runtime_graphql,
    with_graphql_executor_hook, with_schema_sdl_fetch_hook,
};
use super::*;
use crate::cli::{Cli, Commands};
use crate::test_support::git_fixtures::{git_ok, init_test_repo};
use crate::test_support::process_state::enter_process_state;
use clap::Parser;
use rusqlite::Connection;
use serde_json::json;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tempfile::TempDir;

fn test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

fn ingest_enqueue_command(require_daemon: bool) -> DevqlCommand {
    DevqlCommand::Tasks(DevqlTasksArgs {
        command: DevqlTasksCommand::Enqueue(DevqlTaskEnqueueArgs {
            kind: DevqlTaskKindArg::Ingest,
            full: false,
            paths: None,
            repair: false,
            validate: false,
            backfill: None,
            status: false,
            require_daemon,
        }),
    })
}

fn sync_enqueue_command(
    full: bool,
    paths: Option<Vec<String>>,
    repair: bool,
    validate: bool,
    status: bool,
    require_daemon: bool,
) -> DevqlCommand {
    DevqlCommand::Tasks(DevqlTasksArgs {
        command: DevqlTasksCommand::Enqueue(DevqlTaskEnqueueArgs {
            kind: DevqlTaskKindArg::Sync,
            full,
            paths,
            repair,
            validate,
            backfill: None,
            status,
            require_daemon,
        }),
    })
}

fn queued_sync_task_payload(
    task_id: &str,
    mode: &str,
    paths: Option<Vec<&str>>,
) -> serde_json::Value {
    json!({
        "taskId": task_id,
        "repoId": "repo-1",
        "repoName": "demo",
        "repoIdentity": "local/demo",
        "kind": "SYNC",
        "source": "manual_cli",
        "status": "QUEUED",
        "submittedAtUnix": 1,
        "startedAtUnix": null,
        "updatedAtUnix": 1,
        "completedAtUnix": null,
        "queuePosition": 1,
        "tasksAhead": 0,
        "error": null,
        "syncSpec": {
            "mode": mode,
            "paths": paths.unwrap_or_default(),
        },
        "ingestSpec": null,
        "syncProgress": {
            "phase": "queued",
            "currentPath": null,
            "pathsTotal": 0,
            "pathsCompleted": 0,
            "pathsRemaining": 0,
            "pathsUnchanged": 0,
            "pathsAdded": 0,
            "pathsChanged": 0,
            "pathsRemoved": 0,
            "cacheHits": 0,
            "cacheMisses": 0,
            "parseErrors": 0
        },
        "ingestProgress": null,
        "syncResult": null,
        "ingestResult": null
    })
}

fn queued_ingest_task_payload(task_id: &str, backfill: Option<usize>) -> serde_json::Value {
    json!({
        "taskId": task_id,
        "repoId": "repo-1",
        "repoName": "demo",
        "repoIdentity": "local/demo",
        "kind": "INGEST",
        "source": "manual_cli",
        "status": "QUEUED",
        "submittedAtUnix": 1,
        "startedAtUnix": null,
        "updatedAtUnix": 1,
        "completedAtUnix": null,
        "queuePosition": 1,
        "tasksAhead": 0,
        "error": null,
        "syncSpec": null,
        "ingestSpec": {
            "backfill": backfill,
        },
        "syncProgress": null,
        "ingestProgress": null,
        "syncResult": null,
        "ingestResult": null
    })
}

fn test_daemon_state_root(repo_root: &Path) -> PathBuf {
    repo_root.join(".bitloops-test-state")
}

fn write_envelope_config(repo_root: &Path, settings: serde_json::Value) {
    let config_path = repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH);
    let sqlite_path = settings["stores"]["relational"]["sqlite_path"]
        .as_str()
        .expect("relational sqlite path");
    let duckdb_path = settings["stores"]["events"]["duckdb_path"]
        .as_str()
        .expect("events duckdb path");

    fs::write(
        &config_path,
        format!(
            r#"[stores]
[stores.relational]
sqlite_path = {sqlite_path:?}

[stores.events]
duckdb_path = {duckdb_path:?}
"#
        ),
    )
    .expect("write config");
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("write repo daemon binding");
}

fn seed_devql_cli_repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();

    init_test_repo(repo_root, "main", "Alice", "alice@example.com");
    fs::create_dir_all(repo_root.join("src")).expect("create src dir");
    fs::write(
        repo_root.join("src/lib.rs"),
        "pub fn answer() -> i32 {\n    42\n}\n",
    )
    .expect("write lib.rs");
    git_ok(repo_root, &["add", "."]);
    git_ok(repo_root, &["commit", "-m", "Seed DevQL CLI repo"]);

    let daemon_state_root = test_daemon_state_root(repo_root);
    write_envelope_config(
        repo_root,
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": daemon_state_root
                        .join("stores")
                        .join("relational")
                        .join("devql.sqlite")
                },
                "events": {
                    "duckdb_path": daemon_state_root
                        .join("stores")
                        .join("event")
                        .join("events.duckdb")
                }
            },
            "semantic": {
                "provider": "disabled"
            }
        }),
    );

    dir
}

fn sqlite_path_for_repo(repo_root: &Path) -> PathBuf {
    crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config")
        .relational
        .resolve_sqlite_db_path_for_repo(repo_root)
        .expect("resolve sqlite path")
}

fn duckdb_path_for_repo(repo_root: &Path) -> PathBuf {
    crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config")
        .events
        .resolve_duckdb_db_path_for_repo(repo_root)
}

fn seed_cli_analytics_sources(repo_root: &Path) {
    use crate::host::interactions::interaction_repository::create_interaction_repository;
    use crate::host::interactions::store::InteractionEventRepository;
    use crate::host::interactions::types::{
        InteractionEvent, InteractionEventType, InteractionSession, InteractionTurn,
    };

    let sqlite_path = sqlite_path_for_repo(repo_root);
    let duckdb_path = duckdb_path_for_repo(repo_root);
    fs::create_dir_all(sqlite_path.parent().expect("sqlite parent")).expect("create sqlite dir");
    fs::create_dir_all(duckdb_path.parent().expect("duckdb parent")).expect("create duckdb dir");

    let sqlite = Connection::open(&sqlite_path).expect("open sqlite");
    sqlite
        .execute_batch(
            "
            CREATE TABLE repositories (
                repo_id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                organization TEXT NOT NULL,
                name TEXT NOT NULL,
                default_branch TEXT,
                metadata_json TEXT,
                created_at TEXT
            );
            CREATE TABLE repo_sync_state (
                repo_id TEXT PRIMARY KEY,
                repo_root TEXT NOT NULL,
                active_branch TEXT,
                head_commit_sha TEXT,
                head_tree_sha TEXT,
                parser_version TEXT NOT NULL,
                extractor_version TEXT NOT NULL,
                scope_exclusions_fingerprint TEXT,
                last_sync_started_at TEXT,
                last_sync_completed_at TEXT,
                last_sync_status TEXT,
                last_sync_reason TEXT
            );
            CREATE TABLE current_file_state (
                repo_id TEXT NOT NULL,
                path TEXT NOT NULL,
                analysis_mode TEXT NOT NULL,
                file_role TEXT NOT NULL,
                text_index_mode TEXT NOT NULL,
                language TEXT NOT NULL,
                resolved_language TEXT NOT NULL,
                dialect TEXT,
                primary_context_id TEXT,
                secondary_context_ids_json TEXT NOT NULL,
                frameworks_json TEXT NOT NULL,
                runtime_profile TEXT,
                classification_reason TEXT NOT NULL,
                context_fingerprint TEXT,
                extraction_fingerprint TEXT NOT NULL,
                head_content_id TEXT,
                index_content_id TEXT,
                worktree_content_id TEXT,
                effective_content_id TEXT NOT NULL,
                effective_source TEXT NOT NULL,
                parser_version TEXT NOT NULL,
                extractor_version TEXT NOT NULL,
                exists_in_head INTEGER NOT NULL,
                exists_in_index INTEGER NOT NULL,
                exists_in_worktree INTEGER NOT NULL,
                last_synced_at TEXT NOT NULL,
                PRIMARY KEY (repo_id, path)
            );",
        )
        .expect("create analytics sqlite tables");

    let repo = crate::host::devql::resolve_repo_identity(repo_root).expect("resolve repo identity");
    sqlite
        .execute(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch, metadata_json, created_at)
             VALUES (?1, ?2, ?3, ?4, 'main', '{}', '2026-04-22T09:00:00Z')",
            rusqlite::params![repo.repo_id, repo.provider, repo.organization, repo.name],
        )
        .expect("insert repositories row");
    sqlite
        .execute(
            "INSERT INTO repo_sync_state (repo_id, repo_root, active_branch, head_commit_sha, head_tree_sha, parser_version, extractor_version, scope_exclusions_fingerprint, last_sync_started_at, last_sync_completed_at, last_sync_status, last_sync_reason)
             VALUES (?1, ?2, 'main', 'abc', 'def', '1', '1', '', '2026-04-22T09:00:00Z', '2026-04-22T09:05:00Z', 'completed', '')",
            rusqlite::params![repo.repo_id, repo_root.to_string_lossy().to_string()],
        )
        .expect("insert repo_sync_state row");
    sqlite
        .execute(
            "INSERT INTO current_file_state (
                repo_id, path, analysis_mode, file_role, text_index_mode, language, resolved_language, dialect,
                primary_context_id, secondary_context_ids_json, frameworks_json, runtime_profile, classification_reason,
                context_fingerprint, extraction_fingerprint, head_content_id, index_content_id, worktree_content_id,
                effective_content_id, effective_source, parser_version, extractor_version, exists_in_head, exists_in_index,
                exists_in_worktree, last_synced_at
             ) VALUES (
                ?1, 'src/lib.rs', 'code', 'source_code', 'none', 'rust', 'rust', '', '', '[]', '[]', '',
                'seeded', '', 'fingerprint-1', 'head-1', 'index-1', 'worktree-1', 'effective-1', 'worktree', '1', '1', 1, 1, 1,
                '2026-04-22T09:04:00Z'
             )",
            rusqlite::params![repo.repo_id],
        )
        .expect("insert current_file_state row");

    let events_cfg = crate::config::EventsBackendConfig {
        duckdb_path: Some(duckdb_path.to_string_lossy().to_string()),
        clickhouse_url: None,
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: None,
    };
    let repository = create_interaction_repository(&events_cfg, repo_root, repo.repo_id.clone())
        .expect("create interaction repository");
    repository
        .upsert_session(&InteractionSession {
            session_id: "session-1".to_string(),
            repo_id: repo.repo_id.clone(),
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "seed".to_string(),
            agent_type: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            first_prompt: "Inspect analytics".to_string(),
            transcript_path: "/tmp/transcript.jsonl".to_string(),
            worktree_path: repo_root.to_string_lossy().to_string(),
            worktree_id: "worktree-1".to_string(),
            started_at: "2026-04-22T09:00:00Z".to_string(),
            ended_at: None,
            last_event_at: "2026-04-22T09:03:00Z".to_string(),
            updated_at: "2026-04-22T09:03:00Z".to_string(),
        })
        .expect("upsert analytics session");
    repository
        .upsert_turn(&InteractionTurn {
            turn_id: "turn-1".to_string(),
            session_id: "session-1".to_string(),
            repo_id: repo.repo_id.clone(),
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "seed".to_string(),
            turn_number: 1,
            prompt: "Inspect analytics".to_string(),
            agent_type: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            started_at: "2026-04-22T09:00:00Z".to_string(),
            ended_at: Some("2026-04-22T09:03:00Z".to_string()),
            token_usage: Some(
                crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata {
                    input_tokens: 120,
                    output_tokens: 80,
                    cache_creation_tokens: 0,
                    cache_read_tokens: 0,
                    api_call_count: 1,
                    subagent_tokens: None,
                },
            ),
            summary: "Analysed tool usage".to_string(),
            prompt_count: 1,
            transcript_offset_start: Some(0),
            transcript_offset_end: Some(100),
            transcript_fragment: "fragment".to_string(),
            files_modified: vec!["src/lib.rs".to_string()],
            checkpoint_id: None,
            updated_at: "2026-04-22T09:03:00Z".to_string(),
        })
        .expect("upsert analytics turn");
    repository
        .append_event(&InteractionEvent {
            event_id: "event-1".to_string(),
            session_id: "session-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            repo_id: repo.repo_id,
            branch: "main".to_string(),
            actor_id: "actor-1".to_string(),
            actor_name: "Alice".to_string(),
            actor_email: "alice@example.com".to_string(),
            actor_source: "seed".to_string(),
            event_type: InteractionEventType::ToolInvocationObserved,
            event_time: "2026-04-22T09:01:00Z".to_string(),
            source: "transcript_derivation".to_string(),
            sequence_number: 1,
            agent_type: "codex".to_string(),
            model: "gpt-5.4".to_string(),
            tool_use_id: "toolu-1".to_string(),
            tool_kind: "bash".to_string(),
            task_description: "rg tool analytics".to_string(),
            subagent_id: String::new(),
            payload: json!({
                "tool_name": "bash",
                "input_summary": "rg tool analytics",
                "command": "rg tool analytics",
                "command_binary": "rg",
                "command_argv": ["rg", "tool", "analytics"],
                "transcript_path": "/tmp/transcript.jsonl"
            }),
        })
        .expect("append analytics event");
}

fn with_isolated_daemon_state<T>(repo_root: &Path, f: impl FnOnce() -> T) -> T {
    let state_root = TempDir::new().expect("temp dir");
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo_root),
        &[(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_root_str.as_str()),
        )],
    );
    f()
}

fn write_current_runtime_state(repo_root: &Path) {
    let runtime_path = crate::daemon::runtime_state_path(repo_root);
    let runtime_state = crate::daemon::DaemonRuntimeState {
        version: 1,
        config_path: repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        config_root: repo_root.to_path_buf(),
        pid: std::process::id(),
        mode: crate::daemon::DaemonMode::Detached,
        service_name: None,
        url: "http://127.0.0.1:5667".to_string(),
        host: "127.0.0.1".to_string(),
        port: 5667,
        bundle_dir: repo_root.join("bundle"),
        relational_db_path: repo_root.join("relational.db"),
        events_db_path: repo_root.join("events.duckdb"),
        blob_store_path: repo_root.join("blob"),
        repo_registry_path: repo_root.join("repo-registry.json"),
        binary_fingerprint: crate::daemon::current_binary_fingerprint().unwrap_or_default(),
        updated_at_unix: 0,
    };
    fs::create_dir_all(
        runtime_path
            .parent()
            .expect("runtime state should have a parent directory"),
    )
    .expect("create runtime state parent");
    let mut bytes = serde_json::to_vec_pretty(&runtime_state).expect("serialise runtime state");
    bytes.push(b'\n');
    fs::write(&runtime_path, bytes).expect("write runtime state");
}

fn assert_cli_parses(argv: &[&str]) -> Cli {
    Cli::try_parse_from(argv.iter().copied())
        .unwrap_or_else(|err| panic!("expected `{}` to parse: {err}", argv.join(" ")))
}

fn parse_architecture_roles_command(argv: &[&str]) -> DevqlArchitectureRolesCommand {
    let parsed = assert_cli_parses(argv);

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Architecture(architecture)) = args.command else {
        panic!("expected devql architecture command");
    };
    let DevqlArchitectureCommand::Roles(roles) = architecture.command;

    roles.command
}

#[test]
fn devql_cli_parses_ingest_enqueue_defaults() {
    let parsed = Cli::try_parse_from(["bitloops", "devql", "tasks", "enqueue", "--kind", "ingest"])
        .expect("devql task ingest enqueue should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Tasks(tasks)) = args.command else {
        panic!("expected devql tasks command");
    };
    let DevqlTasksCommand::Enqueue(enqueue) = tasks.command else {
        panic!("expected devql tasks enqueue command");
    };

    assert!(matches!(enqueue.kind, DevqlTaskKindArg::Ingest));
    assert!(!enqueue.require_daemon);
}

#[test]
fn devql_cli_parses_ingest_enqueue_require_daemon_flag() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "tasks",
        "enqueue",
        "--kind",
        "ingest",
        "--require-daemon",
    ])
    .expect("devql task ingest enqueue --require-daemon should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Tasks(tasks)) = args.command else {
        panic!("expected devql tasks command");
    };
    let DevqlTasksCommand::Enqueue(enqueue) = tasks.command else {
        panic!("expected devql tasks enqueue command");
    };

    assert!(enqueue.require_daemon);
}

#[test]
fn devql_cli_parses_sync_enqueue_modes() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "tasks",
        "enqueue",
        "--kind",
        "sync",
        "--paths",
        "src/lib.rs,src/main.rs",
    ])
    .expect("devql task sync enqueue with paths should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Tasks(tasks)) = args.command else {
        panic!("expected devql tasks command");
    };
    let DevqlTasksCommand::Enqueue(sync) = tasks.command else {
        panic!("expected devql tasks enqueue command");
    };

    assert!(!sync.full);
    assert_eq!(
        sync.paths,
        Some(vec!["src/lib.rs".to_string(), "src/main.rs".to_string()])
    );
    assert!(!sync.repair);
    assert!(!sync.validate);
    assert!(!sync.status);
    assert!(!sync.require_daemon);

    let parsed = Cli::try_parse_from([
        "bitloops", "devql", "tasks", "enqueue", "--kind", "sync", "--repair",
    ])
    .expect("devql task sync repair should parse");
    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Tasks(tasks)) = args.command else {
        panic!("expected devql tasks command");
    };
    let DevqlTasksCommand::Enqueue(sync) = tasks.command else {
        panic!("expected devql tasks enqueue command");
    };
    assert!(!sync.full);
    assert_eq!(sync.paths, None);
    assert!(sync.repair);
    assert!(!sync.validate);
    assert!(!sync.status);
    assert!(!sync.require_daemon);

    let parsed = Cli::try_parse_from([
        "bitloops", "devql", "tasks", "enqueue", "--kind", "sync", "--full",
    ])
    .expect("devql task sync full should parse");
    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Tasks(tasks)) = args.command else {
        panic!("expected devql tasks command");
    };
    let DevqlTasksCommand::Enqueue(sync) = tasks.command else {
        panic!("expected devql tasks enqueue command");
    };
    assert!(sync.full);
    assert_eq!(sync.paths, None);
    assert!(!sync.repair);
    assert!(!sync.validate);
    assert!(!sync.status);
    assert!(!sync.require_daemon);

    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "tasks",
        "enqueue",
        "--kind",
        "sync",
        "--validate",
    ])
    .expect("devql task sync validate should parse");
    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Tasks(tasks)) = args.command else {
        panic!("expected devql tasks command");
    };
    let DevqlTasksCommand::Enqueue(sync) = tasks.command else {
        panic!("expected devql tasks enqueue command");
    };
    assert!(!sync.full);
    assert_eq!(sync.paths, None);
    assert!(!sync.repair);
    assert!(sync.validate);
    assert!(!sync.status);
    assert!(!sync.require_daemon);
}

#[test]
fn devql_cli_parses_sync_enqueue_status_flag() {
    let parsed = Cli::try_parse_from([
        "bitloops", "devql", "tasks", "enqueue", "--kind", "sync", "--status",
    ])
    .expect("devql task sync enqueue --status should parse");
    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Tasks(tasks)) = args.command else {
        panic!("expected devql tasks command");
    };
    let DevqlTasksCommand::Enqueue(sync) = tasks.command else {
        panic!("expected devql tasks enqueue command");
    };
    assert!(sync.status);
    assert!(!sync.require_daemon);
}

#[test]
fn devql_cli_parses_sync_enqueue_require_daemon_flag() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "tasks",
        "enqueue",
        "--kind",
        "sync",
        "--require-daemon",
    ])
    .expect("devql task sync enqueue --require-daemon should parse");
    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Tasks(tasks)) = args.command else {
        panic!("expected devql tasks command");
    };
    let DevqlTasksCommand::Enqueue(sync) = tasks.command else {
        panic!("expected devql tasks enqueue command");
    };
    assert!(sync.require_daemon);
}

#[test]
fn devql_cli_parses_architecture_roles_classify_command() {
    let classify_full = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "classify",
        "--full",
    ]);
    let DevqlArchitectureRolesCommand::Classify(classify_full) = classify_full else {
        panic!("expected architecture roles classify command");
    };
    assert!(classify_full.full);

    let classify_paths = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "classify",
        "--paths",
        "src/main.rs,src/lib.rs",
        "--json",
    ]);
    let DevqlArchitectureRolesCommand::Classify(classify_paths) = classify_paths else {
        panic!("expected architecture roles classify command");
    };
    assert_eq!(
        classify_paths.paths,
        Some(vec!["src/main.rs".to_string(), "src/lib.rs".to_string()])
    );
    assert!(classify_paths.json);
}

#[test]
fn architecture_roles_manual_chain_commands_parse() {
    assert!(matches!(
        parse_architecture_roles_command(&["bitloops", "devql", "architecture", "roles", "seed"]),
        DevqlArchitectureRolesCommand::Seed(_)
    ));
    assert!(matches!(
        parse_architecture_roles_command(&[
            "bitloops",
            "devql",
            "architecture",
            "roles",
            "rules",
            "activate",
            "rule-1",
        ]),
        DevqlArchitectureRolesCommand::Rules(_)
    ));
    assert!(matches!(
        parse_architecture_roles_command(&[
            "bitloops",
            "devql",
            "architecture",
            "roles",
            "proposal",
            "apply",
            "proposal-1",
        ]),
        DevqlArchitectureRolesCommand::Proposal(_)
    ));
    assert!(matches!(
        parse_architecture_roles_command(&[
            "bitloops",
            "devql",
            "architecture",
            "roles",
            "classify",
            "--full",
        ]),
        DevqlArchitectureRolesCommand::Classify(_)
    ));
    assert!(matches!(
        parse_architecture_roles_command(&[
            "bitloops",
            "devql",
            "architecture",
            "roles",
            "bootstrap",
        ]),
        DevqlArchitectureRolesCommand::Bootstrap(_)
    ));
}

#[test]
fn devql_cli_rejects_conflicting_sync_enqueue_modes() {
    let cases = vec![
        vec![
            "bitloops",
            "devql",
            "tasks",
            "enqueue",
            "--kind",
            "sync",
            "--full",
            "--paths",
            "src/lib.rs",
        ],
        vec![
            "bitloops", "devql", "tasks", "enqueue", "--kind", "sync", "--full", "--repair",
        ],
        vec![
            "bitloops",
            "devql",
            "tasks",
            "enqueue",
            "--kind",
            "sync",
            "--paths",
            "src/lib.rs",
            "--repair",
        ],
        vec![
            "bitloops",
            "devql",
            "tasks",
            "enqueue",
            "--kind",
            "sync",
            "--validate",
            "--repair",
        ],
        vec![
            "bitloops",
            "devql",
            "tasks",
            "enqueue",
            "--kind",
            "sync",
            "--validate",
            "--full",
        ],
        vec![
            "bitloops",
            "devql",
            "tasks",
            "enqueue",
            "--kind",
            "sync",
            "--validate",
            "--paths",
            "src/lib.rs",
        ],
        vec![
            "bitloops",
            "devql",
            "tasks",
            "enqueue",
            "--kind",
            "sync",
            "--full",
            "--paths",
            "src/lib.rs",
            "--repair",
        ],
        vec![
            "bitloops",
            "devql",
            "sync",
            "--validate",
            "--full",
            "--paths",
            "src/lib.rs",
            "--repair",
        ],
    ];

    for argv in cases {
        assert!(
            Cli::try_parse_from(argv.iter().copied()).is_err(),
            "expected conflicting sync modes to be rejected for argv: {argv:?}"
        );
    }
}

#[test]
fn format_sync_completion_summary_includes_diagnostics_when_present() {
    let summary = SyncSummary {
        success: true,
        mode: "repair".to_string(),
        parser_version: "parser@1".to_string(),
        extractor_version: "extractor@1".to_string(),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("abc123".to_string()),
        head_tree_sha: Some("def456".to_string()),
        paths_unchanged: 4,
        paths_added: 1,
        paths_changed: 2,
        paths_removed: 3,
        cache_hits: 5,
        cache_misses: 2,
        parse_errors: 1,
        validation: None,
    };

    assert_eq!(
        format_sync_completion_summary(&summary),
        "sync complete: 1 added, 2 changed, 3 removed, 4 unchanged, 5 cache hits (mode=repair, 2 cache misses, 1 parse errors)"
    );
}

#[test]
fn format_sync_completion_summary_keeps_basic_happy_path_line() {
    let summary = SyncSummary {
        success: true,
        mode: "full".to_string(),
        parser_version: "parser@1".to_string(),
        extractor_version: "extractor@1".to_string(),
        active_branch: None,
        head_commit_sha: None,
        head_tree_sha: None,
        paths_unchanged: 4,
        paths_added: 1,
        paths_changed: 2,
        paths_removed: 3,
        cache_hits: 5,
        cache_misses: 0,
        parse_errors: 0,
        validation: None,
    };

    assert_eq!(
        format_sync_completion_summary(&summary),
        "sync complete: 1 added, 2 changed, 3 removed, 4 unchanged, 5 cache hits"
    );
}

#[test]
fn format_sync_completion_summary_for_validate_reports_path_drift() {
    let summary = SyncSummary {
        success: false,
        mode: "validate".to_string(),
        parser_version: "parser@1".to_string(),
        extractor_version: "extractor@1".to_string(),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("abc123".to_string()),
        head_tree_sha: Some("def456".to_string()),
        paths_unchanged: 0,
        paths_added: 0,
        paths_changed: 0,
        paths_removed: 0,
        cache_hits: 0,
        cache_misses: 0,
        parse_errors: 0,
        validation: Some(crate::host::devql::SyncValidationSummary {
            valid: false,
            expected_artefacts: 10,
            actual_artefacts: 8,
            expected_edges: 6,
            actual_edges: 6,
            missing_artefacts: 2,
            stale_artefacts: 0,
            mismatched_artefacts: 0,
            missing_edges: 0,
            stale_edges: 0,
            mismatched_edges: 1,
            files_with_drift: vec![crate::host::devql::SyncValidationFileDrift {
                path: "src/lib.rs".to_string(),
                missing_artefacts: 2,
                stale_artefacts: 0,
                mismatched_artefacts: 0,
                missing_edges: 0,
                stale_edges: 0,
                mismatched_edges: 1,
            }],
        }),
    };

    let rendered = format_sync_completion_summary(&summary);
    assert!(
        rendered.contains("sync validation: drift detected"),
        "expected validation header, got: {rendered}"
    );
    assert!(
        rendered.contains("artefacts: expected=10 actual=8 missing=2 stale=0 mismatched=0"),
        "expected artefacts counters, got: {rendered}"
    );
    assert!(
        rendered.contains("edges: expected=6 actual=6 missing=0 stale=0 mismatched=1"),
        "expected edges counters, got: {rendered}"
    );
    assert!(
        rendered.contains(
            "src/lib.rs: artefacts missing=2 stale=0 mismatched=0; edges missing=0 stale=0 mismatched=1"
        ),
        "expected file drift entry, got: {rendered}"
    );
}

#[test]
fn devql_cli_parses_checkpoint_file_snapshot_projection_command() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "projection",
        "checkpoint-file-snapshots",
        "--batch-size",
        "25",
        "--max-checkpoints",
        "40",
        "--resume-after",
        "a1b2c3",
        "--dry-run",
    ])
    .expect("projection command should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Projection(projection)) = args.command else {
        panic!("expected devql projection command");
    };
    let DevqlProjectionCommand::CheckpointFileSnapshots(backfill) = projection.command;

    assert_eq!(backfill.batch_size, 25);
    assert_eq!(backfill.max_checkpoints, Some(40));
    assert_eq!(backfill.resume_after.as_deref(), Some("a1b2c3"));
    assert!(backfill.dry_run);
}

#[test]
fn devql_cli_parses_packs_flags() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "packs",
        "--json",
        "--with-health",
        "--apply-migrations",
        "--with-extensions",
    ])
    .expect("devql packs should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Packs(packs)) = args.command else {
        panic!("expected devql packs command");
    };

    assert!(packs.json);
    assert!(packs.with_health);
    assert!(packs.apply_migrations);
    assert!(packs.with_extensions);
}

#[test]
fn devql_cli_parses_navigation_context_status_flags() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "navigation-context",
        "status",
        "--project",
        "crates/api",
        "--view",
        "architecture_map",
        "--status",
        "stale",
        "--changed-limit",
        "3",
        "--json",
    ])
    .expect("devql navigation-context status should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::NavigationContext(navigation_context)) = args.command else {
        panic!("expected devql navigation-context command");
    };
    let DevqlNavigationContextCommand::Status(status) = navigation_context.command else {
        panic!("expected devql navigation-context status command");
    };

    assert_eq!(status.project, "crates/api");
    assert_eq!(status.view.as_deref(), Some("architecture_map"));
    assert_eq!(status.status, Some(DevqlNavigationContextStatusArg::Stale));
    assert_eq!(status.changed_limit, 3);
    assert!(status.json);
}

#[test]
fn devql_cli_parses_navigation_context_accept_flags() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "navigation-context",
        "accept",
        "architecture_map",
        "--expected-current-signature",
        "signature-1",
        "--reason",
        "reviewed",
        "--materialised-ref",
        "docs/navigation/architecture.md",
        "--json",
    ])
    .expect("devql navigation-context accept should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::NavigationContext(navigation_context)) = args.command else {
        panic!("expected devql navigation-context command");
    };
    let DevqlNavigationContextCommand::Accept(accept) = navigation_context.command else {
        panic!("expected devql navigation-context accept command");
    };

    assert_eq!(accept.view_id, "architecture_map");
    assert_eq!(
        accept.expected_current_signature.as_deref(),
        Some("signature-1")
    );
    assert_eq!(accept.reason.as_deref(), Some("reviewed"));
    assert_eq!(
        accept.materialised_ref.as_deref(),
        Some("docs/navigation/architecture.md")
    );
    assert!(accept.json);
}

#[test]
fn devql_cli_parses_navigation_context_materialise_flags() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "navigation-context",
        "materialise",
        "architecture_map",
        "--expected-current-signature",
        "signature-1",
        "--rendered",
        "--json",
    ])
    .expect("devql navigation-context materialise should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::NavigationContext(navigation_context)) = args.command else {
        panic!("expected devql navigation-context command");
    };
    let DevqlNavigationContextCommand::Materialise(materialise) = navigation_context.command else {
        panic!("expected devql navigation-context materialise command");
    };

    assert_eq!(materialise.view_id, "architecture_map");
    assert_eq!(
        materialise.expected_current_signature.as_deref(),
        Some("signature-1")
    );
    assert!(materialise.rendered);
    assert!(materialise.json);
}

#[test]
fn format_navigation_context_status_shows_changed_primitive_details() {
    let snapshot = json!({
        "totalViews": 1,
        "totalPrimitives": 42,
        "totalEdges": 7,
        "views": [{
            "viewId": "architecture_map",
            "label": "Architecture map",
            "acceptedSignature": "accepted-signature",
            "currentSignature": "current-signature",
            "status": "STALE",
            "materialisedRef": "docs/navigation/architecture.md",
            "acceptanceHistory": [{
                "acceptanceId": "acceptance-1",
                "acceptedSignature": "accepted-signature",
                "previousAcceptedSignature": "previous-accepted-signature",
                "currentSignature": "accepted-signature",
                "expectedCurrentSignature": "accepted-signature",
                "source": "manual_cli",
                "reason": "reviewed",
                "materialisedRef": "docs/navigation/architecture.md",
                "acceptedAt": "2026-05-03T00:00:00Z"
            }],
            "staleReason": {
                "changedPrimitives": [{
                    "primitiveId": "symbol-1",
                    "primitiveKind": "SYMBOL",
                    "label": "render",
                    "path": "src/render.rs",
                    "sourceKind": "TEST",
                    "changeKind": "hash_changed",
                    "previousHash": "previous-signature",
                    "currentHash": "current-signature"
                }]
            }
        }]
    });

    let rendered = format_navigation_context_status(&snapshot, 10);

    assert!(
        rendered.contains("navigation context: 1 views, 1 stale, 42 primitives, 7 edges"),
        "expected summary line, got: {rendered}"
    );
    assert!(
        rendered.contains("- architecture_map [stale] Architecture map"),
        "expected view line, got: {rendered}"
    );
    assert!(
        rendered.contains("materialised: docs/navigation/architecture.md"),
        "expected materialised ref, got: {rendered}"
    );
    assert!(
        rendered.contains("last accepted: 2026-05-03T00:00:00Z by manual_cli"),
        "expected acceptance history, got: {rendered}"
    );
    assert!(
        rendered.contains("hash_changed: SYMBOL render (src/render.rs) previous-sig->current-sign"),
        "expected changed primitive details, got: {rendered}"
    );
}

#[test]
fn format_navigation_context_materialisation_shows_snapshot_ref_and_counts() {
    let result = json!({
        "viewId": "architecture_map",
        "currentSignature": "current-signature",
        "status": "STALE",
        "materialisedRef": "navigation-context://materialisations/123",
        "primitiveCount": 42,
        "edgeCount": 7,
        "materialisedAt": "2026-05-03T00:00:00Z"
    });

    let rendered = format_navigation_context_materialisation(&result);

    assert!(
        rendered.contains("navigation context view materialised: architecture_map status=stale"),
        "expected materialisation summary, got: {rendered}"
    );
    assert!(
        rendered.contains("primitives=42 edges=7"),
        "expected materialisation counts, got: {rendered}"
    );
    assert!(
        rendered.contains("ref=navigation-context://materialisations/123"),
        "expected materialised ref, got: {rendered}"
    );
}

#[test]
fn devql_cli_parses_schema_defaults() {
    let parsed =
        Cli::try_parse_from(["bitloops", "devql", "schema"]).expect("devql schema should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Schema(schema)) = args.command else {
        panic!("expected devql schema command");
    };

    assert!(!schema.global);
    assert!(!schema.human);
}

#[test]
fn devql_cli_parses_schema_global_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "devql", "schema", "--global"])
        .expect("devql schema --global should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Schema(schema)) = args.command else {
        panic!("expected devql schema command");
    };

    assert!(schema.global);
    assert!(!schema.human);
}

#[test]
fn devql_cli_parses_schema_human_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "devql", "schema", "--human"])
        .expect("devql schema --human should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Schema(schema)) = args.command else {
        panic!("expected devql schema command");
    };

    assert!(!schema.global);
    assert!(schema.human);
}

#[test]
fn devql_cli_parses_schema_global_human_flags() {
    let parsed = Cli::try_parse_from(["bitloops", "devql", "schema", "--global", "--human"])
        .expect("devql schema --global --human should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Schema(schema)) = args.command else {
        panic!("expected devql schema command");
    };

    assert!(schema.global);
    assert!(schema.human);
}

#[test]
fn devql_cli_parses_query_compact_flag() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "query",
        "repo(\"bitloops-cli\")",
        "--compact",
    ])
    .expect("devql query should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Query(query)) = args.command else {
        panic!("expected devql query command");
    };

    assert_eq!(query.query, "repo(\"bitloops-cli\")");
    assert!(query.compact);
}

#[test]
fn devql_cli_parses_query_graphql_flag() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "query",
        "--graphql",
        "{ repo(name: \"bitloops-cli\") { name } }",
    ])
    .expect("devql raw graphql query should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Query(query)) = args.command else {
        panic!("expected devql query command");
    };

    assert_eq!(query.query, "{ repo(name: \"bitloops-cli\") { name } }");
    assert!(query.graphql);
    assert!(!query.compact);
}

#[test]
fn devql_cli_parses_analytics_sql_scope_flags() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "analytics",
        "sql",
        "SELECT * FROM analytics.repositories",
        "--repo",
        "repo-1",
        "--repo",
        "repo-2",
        "--json",
    ])
    .expect("devql analytics sql should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Analytics(analytics)) = args.command else {
        panic!("expected devql analytics command");
    };
    let DevqlAnalyticsCommand::Sql(sql) = analytics.command;

    assert_eq!(sql.query, "SELECT * FROM analytics.repositories");
    assert_eq!(sql.repos, vec!["repo-1".to_string(), "repo-2".to_string()]);
    assert!(sql.json);
    assert!(!sql.all_repos);
}

#[test]
fn devql_analytics_sql_command_executes_for_current_repo() {
    let repo = seed_devql_cli_repo();
    seed_cli_analytics_sources(repo.path());
    let runtime = test_runtime();

    runtime.block_on(async {
        let args = DevqlArgs {
            command: Some(DevqlCommand::Analytics(DevqlAnalyticsArgs {
                command: DevqlAnalyticsCommand::Sql(DevqlAnalyticsSqlArgs {
                    query: "SELECT repo_id, path FROM analytics.current_file_state".to_string(),
                    repos: Vec::new(),
                    all_repos: false,
                    json: true,
                }),
            })),
        };
        let mut sink = Vec::new();
        run_with_scope_discovery(args, &mut sink, || {
            discover_slim_cli_repo_scope(Some(repo.path()))
        })
        .await
        .expect("analytics sql command should execute");
    });
}

#[test]
fn minify_schema_sdl_collapses_whitespace_and_brace_padding() {
    let input = "type QueryRoot {\n    repo(name: String!): Repository!\n}\n";
    let rendered = minify_schema_sdl(input);

    assert_eq!(
        rendered,
        "type QueryRoot {repo(name: String!): Repository!}\n"
    );
}

#[test]
fn minify_schema_sdl_preserves_block_strings() {
    let input =
        "\"\"\"\nLine one\nLine two\n\"\"\"\ntype QueryRoot {\n    health: HealthStatus!\n}\n";
    let rendered = minify_schema_sdl(input);

    assert_eq!(
        rendered,
        "\"\"\"\nLine one\nLine two\n\"\"\" type QueryRoot {health: HealthStatus!}\n"
    );
}

#[test]
fn minify_schema_sdl_preserves_quoted_string_defaults() {
    let input = "type QueryRoot {\n    example(arg: String = \"a  b\\n c\"): String!\n}\n";
    let rendered = minify_schema_sdl(input);

    assert_eq!(
        rendered,
        "type QueryRoot {example(arg: String = \"a  b\\n c\"): String!}\n"
    );
}

#[test]
fn minify_schema_sdl_drops_padding_before_closing_braces() {
    let input = "type QueryRoot {\n    nested: Nested\n    \n}\n";
    let rendered = minify_schema_sdl(input);

    assert_eq!(rendered, "type QueryRoot {nested: Nested}\n");
}

#[test]
fn devql_cli_parses_knowledge_add_command() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "knowledge",
        "add",
        "https://github.com/bitloops/bitloops/issues/42",
        "--commit",
        "abc123",
    ])
    .expect("devql knowledge add should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Knowledge(knowledge)) = args.command else {
        panic!("expected devql knowledge command");
    };
    let DevqlKnowledgeCommand::Add(add) = knowledge.command else {
        panic!("expected knowledge add command");
    };

    assert_eq!(add.url, "https://github.com/bitloops/bitloops/issues/42");
    assert_eq!(add.commit.as_deref(), Some("abc123"));
}

#[test]
fn devql_test_harness_ingest_tests_help_marks_command_as_legacy() {
    let help_text = match Cli::try_parse_from([
        "bitloops",
        "devql",
        "test-harness",
        "ingest-tests",
        "--help",
    ]) {
        Ok(_) => panic!("--help should return a clap error"),
        Err(err) => err.to_string(),
    };

    assert!(
        help_text.contains("Legacy commit-scoped test discovery/linkage ingestion"),
        "expected ingest-tests help to mark the command as legacy:\n{help_text}"
    );
    assert!(
        help_text.contains("Prefer automatic current-state sync for workspace validation"),
        "expected ingest-tests help to steer users toward current-state sync:\n{help_text}"
    );
}

#[test]
fn devql_cli_parses_knowledge_associate_command() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "knowledge",
        "associate",
        "knowledge:item-1",
        "--to",
        "commit:abc123",
    ])
    .expect("devql knowledge associate should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Knowledge(knowledge)) = args.command else {
        panic!("expected devql knowledge command");
    };
    let DevqlKnowledgeCommand::Associate(associate) = knowledge.command else {
        panic!("expected knowledge associate command");
    };

    assert_eq!(associate.source_ref, "knowledge:item-1");
    assert_eq!(associate.target_ref, "commit:abc123");
}

#[test]
fn devql_cli_rejects_knowledge_associate_without_to() {
    let err = match Cli::try_parse_from([
        "bitloops",
        "devql",
        "knowledge",
        "associate",
        "knowledge:item-1",
    ]) {
        Ok(_) => panic!("knowledge associate without --to must fail"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("--to"));
}

#[test]
fn devql_cli_parses_architecture_roles_role_mutation_commands() {
    let seed =
        parse_architecture_roles_command(&["bitloops", "devql", "architecture", "roles", "seed"]);
    assert!(matches!(seed, DevqlArchitectureRolesCommand::Seed(_)));

    let seed_auto = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "seed",
        "--activate-rules",
        "--classify",
        "--enqueue-adjudication=false",
        "--json",
    ]);
    let DevqlArchitectureRolesCommand::Seed(seed_auto) = seed_auto else {
        panic!("expected architecture roles seed command");
    };
    assert!(seed_auto.activate_rules);
    assert!(seed_auto.classify);
    assert!(!seed_auto.enqueue_adjudication);
    assert!(seed_auto.json);

    let status = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "status",
        "--limit",
        "25",
        "--json",
    ]);
    let DevqlArchitectureRolesCommand::Status(status) = status else {
        panic!("expected architecture roles status command");
    };
    assert_eq!(status.limit, 25);
    assert!(status.json);

    let rename = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "rename",
        "role:frontend",
        "--display-name",
        "Frontend",
    ]);
    let DevqlArchitectureRolesCommand::Rename(rename) = rename else {
        panic!("expected architecture roles rename command");
    };
    assert_eq!(rename.role_ref, "role:frontend");
    assert_eq!(rename.display_name, "Frontend");

    let deprecate = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "deprecate",
        "role:frontend",
        "--replacement",
        "role:web-ui",
    ]);
    let DevqlArchitectureRolesCommand::Deprecate(deprecate) = deprecate else {
        panic!("expected architecture roles deprecate command");
    };
    assert_eq!(deprecate.role_ref, "role:frontend");
    assert_eq!(deprecate.replacement.as_deref(), Some("role:web-ui"));

    let remove = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "remove",
        "role:frontend",
        "--replacement",
        "role:web-ui",
    ]);
    let DevqlArchitectureRolesCommand::Remove(remove) = remove else {
        panic!("expected architecture roles remove command");
    };
    assert_eq!(remove.role_ref, "role:frontend");
    assert_eq!(remove.replacement.as_deref(), Some("role:web-ui"));

    let merge = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "merge",
        "role:frontend",
        "--into",
        "role:web-ui",
    ]);
    let DevqlArchitectureRolesCommand::Merge(merge) = merge else {
        panic!("expected architecture roles merge command");
    };
    assert_eq!(merge.source_role_ref, "role:frontend");
    assert_eq!(merge.target_role_ref, "role:web-ui");

    let split = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "split",
        "role:frontend",
        "--spec",
        "roles/frontend-split.json",
    ]);
    let DevqlArchitectureRolesCommand::Split(split) = split else {
        panic!("expected architecture roles split command");
    };
    assert_eq!(split.role_ref, "role:frontend");
    assert_eq!(split.spec, PathBuf::from("roles/frontend-split.json"));
}

#[test]
fn devql_cli_parses_architecture_roles_bootstrap_command() {
    let bootstrap = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "bootstrap",
        "--enqueue-adjudication=false",
        "--json",
    ]);
    let DevqlArchitectureRolesCommand::Bootstrap(bootstrap) = bootstrap else {
        panic!("expected architecture roles bootstrap command");
    };

    assert!(!bootstrap.enqueue_adjudication);
    assert!(bootstrap.json);
}

#[test]
fn parses_architecture_roles_bootstrap_skip_seed() {
    let command = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "bootstrap",
        "--skip-seed",
        "--json",
    ]);

    let DevqlArchitectureRolesCommand::Bootstrap(args) = command else {
        panic!("expected architecture roles bootstrap command");
    };
    assert!(args.skip_seed);
    assert!(args.json);
}

#[test]
fn devql_cli_parses_architecture_roles_alias_create_command() {
    let command = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "alias",
        "create",
        "ui-shell",
        "--role",
        "role:frontend",
    ]);
    let DevqlArchitectureRolesCommand::Alias(alias) = command else {
        panic!("expected architecture roles alias command");
    };
    let DevqlArchitectureRolesAliasCommand::Create(create) = alias.command;

    assert_eq!(create.alias_key, "ui-shell");
    assert_eq!(create.role_ref, "role:frontend");
}

#[test]
fn devql_cli_parses_architecture_roles_rules_commands() {
    let draft = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "rules",
        "draft",
        "--spec",
        "roles/rules-draft.json",
    ]);
    let DevqlArchitectureRolesCommand::Rules(draft) = draft else {
        panic!("expected architecture roles rules command");
    };
    let DevqlArchitectureRolesRulesCommand::Draft(draft) = draft.command else {
        panic!("expected architecture roles rules draft command");
    };
    assert_eq!(draft.spec, PathBuf::from("roles/rules-draft.json"));

    let edit = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "rules",
        "edit",
        "rule:layering",
        "--spec",
        "roles/rules-edit.json",
    ]);
    let DevqlArchitectureRolesCommand::Rules(edit) = edit else {
        panic!("expected architecture roles rules command");
    };
    let DevqlArchitectureRolesRulesCommand::Edit(edit) = edit.command else {
        panic!("expected architecture roles rules edit command");
    };
    assert_eq!(edit.rule_ref, "rule:layering");
    assert_eq!(edit.spec, PathBuf::from("roles/rules-edit.json"));

    let activate = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "rules",
        "activate",
        "rule:layering",
    ]);
    let DevqlArchitectureRolesCommand::Rules(activate) = activate else {
        panic!("expected architecture roles rules command");
    };
    let DevqlArchitectureRolesRulesCommand::Activate(activate) = activate.command else {
        panic!("expected architecture roles rules activate command");
    };
    assert_eq!(activate.rule_ref, "rule:layering");

    let disable = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "rules",
        "disable",
        "rule:layering",
    ]);
    let DevqlArchitectureRolesCommand::Rules(disable) = disable else {
        panic!("expected architecture roles rules command");
    };
    let DevqlArchitectureRolesRulesCommand::Disable(disable) = disable.command else {
        panic!("expected architecture roles rules disable command");
    };
    assert_eq!(disable.rule_ref, "rule:layering");
}

#[test]
fn devql_cli_parses_architecture_roles_proposal_commands() {
    let show = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "proposal",
        "show",
        "proposal:42",
    ]);
    let DevqlArchitectureRolesCommand::Proposal(show) = show else {
        panic!("expected architecture roles proposal command");
    };
    let DevqlArchitectureRolesProposalCommand::Show(show) = show.command else {
        panic!("expected architecture roles proposal show command");
    };
    assert_eq!(show.proposal_id, "proposal:42");

    let apply = parse_architecture_roles_command(&[
        "bitloops",
        "devql",
        "architecture",
        "roles",
        "proposal",
        "apply",
        "proposal:42",
    ]);
    let DevqlArchitectureRolesCommand::Proposal(apply) = apply else {
        panic!("expected architecture roles proposal command");
    };
    let DevqlArchitectureRolesProposalCommand::Apply(apply) = apply.command else {
        panic!("expected architecture roles proposal apply command");
    };
    assert_eq!(apply.proposal_id, "proposal:42");
}

#[test]
fn devql_run_requires_subcommand() {
    let err = test_runtime()
        .block_on(run(DevqlArgs::default()))
        .expect_err("missing subcommand should error");

    assert!(err.to_string().contains(MISSING_SUBCOMMAND_MESSAGE));
    assert!(
        err.to_string()
            .contains("bitloops devql architecture roles seed")
    );
}

#[test]
fn devql_run_global_schema_fetches_daemon_sdl_without_repo_scope() {
    let mut output = Vec::new();
    let sdl = "type QueryRoot {\n    health: HealthStatus!\n}\n";

    with_schema_sdl_fetch_hook(
        move |endpoint_path, scope| {
            assert_eq!(endpoint_path, "/devql/global/sdl");
            assert!(scope.is_none(), "global schema should not carry repo scope");
            Ok((200, sdl.to_string()))
        },
        || {
            test_runtime()
                .block_on(run_with_scope_discovery(
                    DevqlArgs {
                        command: Some(DevqlCommand::Schema(DevqlSchemaArgs {
                            global: true,
                            human: false,
                        })),
                    },
                    &mut output,
                    || -> anyhow::Result<SlimCliRepoScope> {
                        panic!("global schema should not attempt repo scope discovery");
                    },
                ))
                .expect("devql schema --global should succeed without repo scope discovery");
        },
    );

    assert_eq!(
        String::from_utf8(output).expect("utf8"),
        minify_schema_sdl(sdl)
    );
}

#[test]
fn devql_run_schema_fetches_slim_daemon_sdl_with_repo_scope() {
    let repo = seed_devql_cli_repo();
    let repo_root = repo
        .path()
        .canonicalize()
        .unwrap_or_else(|_| repo.path().to_path_buf());
    let mut output = Vec::new();
    let sdl = "type QueryRoot {\n    health: HealthStatus!\n}\n".to_string();

    with_schema_sdl_fetch_hook(
        {
            let sdl = sdl.clone();
            let repo_root = repo_root.clone();
            move |endpoint_path, scope| {
                assert_eq!(endpoint_path, "/devql/sdl");
                let scope = scope.expect("slim schema should include repo scope");
                assert_eq!(scope.repo_root, repo_root);
                assert_eq!(scope.branch_name, "main");
                Ok((200, sdl.clone()))
            }
        },
        || {
            test_runtime()
                .block_on(run_with_scope_discovery(
                    DevqlArgs {
                        command: Some(DevqlCommand::Schema(DevqlSchemaArgs {
                            global: false,
                            human: true,
                        })),
                    },
                    &mut output,
                    || {
                        crate::devql_transport::discover_slim_cli_repo_scope(Some(
                            repo_root.as_path(),
                        ))
                    },
                ))
                .expect("devql schema should fetch slim SDL from the daemon");
        },
    );

    assert_eq!(String::from_utf8(output).expect("utf8"), sdl);
}

#[test]
fn devql_run_schema_requires_repo_scope_when_not_global() {
    let dir = TempDir::new().expect("temp dir");
    let _guard = enter_process_state(Some(dir.path()), &[]);

    let err = test_runtime()
        .block_on(run(DevqlArgs {
            command: Some(DevqlCommand::Schema(DevqlSchemaArgs::default())),
        }))
        .expect_err("devql schema should require repo scope outside a git repository");

    assert_eq!(err.to_string(), SCHEMA_SCOPE_REQUIRED_MESSAGE);
}

#[test]
fn devql_run_schema_requires_running_daemon() {
    let repo = seed_devql_cli_repo();
    let state_root = TempDir::new().expect("temp dir");
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_root_str.as_str()),
        )],
    );

    let err = test_runtime()
        .block_on(run(DevqlArgs {
            command: Some(DevqlCommand::Schema(DevqlSchemaArgs::default())),
        }))
        .expect_err("devql schema should require a running daemon");

    assert_eq!(
        err.to_string(),
        "Bitloops daemon is not running. Start it with `bitloops daemon start`."
    );
}

#[test]
fn devql_run_global_schema_requires_running_daemon() {
    let dir = TempDir::new().expect("temp dir");
    let state_root = TempDir::new().expect("temp dir");
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(dir.path()),
        &[(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_root_str.as_str()),
        )],
    );

    let err = test_runtime()
        .block_on(run(DevqlArgs {
            command: Some(DevqlCommand::Schema(DevqlSchemaArgs {
                global: true,
                human: false,
            })),
        }))
        .expect_err("devql schema --global should require a running daemon");

    assert_eq!(
        err.to_string(),
        "Bitloops daemon is not running. Start it with `bitloops daemon start`."
    );
}

#[test]
fn devql_run_schema_returns_http_status_errors_from_daemon_sdl_fetch() {
    with_schema_sdl_fetch_hook(
        |_endpoint_path, _scope| Ok((503, "temporarily unavailable".to_string())),
        || {
            let err = test_runtime()
                .block_on(run_with_scope_discovery(
                    DevqlArgs {
                        command: Some(DevqlCommand::Schema(DevqlSchemaArgs {
                            global: true,
                            human: false,
                        })),
                    },
                    &mut Vec::new(),
                    || -> anyhow::Result<SlimCliRepoScope> {
                        panic!("global schema should not attempt repo scope discovery");
                    },
                ))
                .expect_err("schema fetch should surface non-200 daemon responses");

            assert!(
                err.to_string()
                    .contains("Bitloops daemon returned HTTP 503 Service Unavailable"),
                "expected HTTP status error, got: {err:#}"
            );
            assert!(
                err.to_string().contains("temporarily unavailable"),
                "expected response body snippet, got: {err:#}"
            );
        },
    );
}

#[test]
fn schema_sdl_fetch_hook_is_cleared_after_panic() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_schema_sdl_fetch_hook(
            |_endpoint_path, _scope| {
                Ok((
                    200,
                    "type QueryRoot { health: HealthStatus! }\n".to_string(),
                ))
            },
            || panic!("boom"),
        );
    }));

    assert!(
        result.is_err(),
        "expected hook installation closure to panic"
    );

    with_schema_sdl_fetch_hook(
        |_endpoint_path, _scope| {
            Ok((
                200,
                "type QueryRoot { health: HealthStatus! }\n".to_string(),
            ))
        },
        || {},
    );
}

#[test]
fn devql_run_init_executes_graphql_mutation() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let captured = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

    with_graphql_executor_hook(
        {
            let captured = Rc::clone(&captured);
            move |_repo_root: &std::path::Path, query: &str, variables: &serde_json::Value| {
                *captured.borrow_mut() = Some((query.to_string(), variables.clone()));
                Ok(json!({
                    "initSchema": {
                        "success": true,
                        "repoIdentity": "repo:bitloops",
                        "repoId": "repo-id-1",
                        "relationalBackend": "sqlite",
                        "eventsBackend": "duckdb"
                    }
                }))
            }
        },
        || {
            test_runtime()
                .block_on(run(DevqlArgs {
                    command: Some(DevqlCommand::Init(DevqlInitArgs::default())),
                }))
                .expect("devql init should succeed");
        },
    );

    let (query, variables) = captured
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert!(query.contains("initSchema"));
    assert_eq!(variables, json!({}));
}

#[test]
fn devql_run_init_requires_running_daemon() {
    let repo = seed_devql_cli_repo();
    with_isolated_daemon_state(repo.path(), || {
        let err = test_runtime()
            .block_on(run(DevqlArgs {
                command: Some(DevqlCommand::Init(DevqlInitArgs::default())),
            }))
            .expect_err("devql init should require a running daemon");

        assert!(
            err.to_string().contains("Bitloops daemon is not running"),
            "expected daemon-required error, got: {err:#}"
        );
    });
}

#[test]
fn devql_run_ingest_enqueue_executes_graphql_mutation_with_expected_input() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let captured = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

    super::graphql::with_ingest_daemon_runtime_hook(
        |_repo_root: &std::path::Path| Ok(()),
        || {
            with_graphql_executor_hook(
                {
                    let captured = Rc::clone(&captured);
                    move |_repo_root: &std::path::Path,
                          query: &str,
                          variables: &serde_json::Value| {
                        *captured.borrow_mut() = Some((query.to_string(), variables.clone()));
                        Ok(json!({
                            "enqueueTask": {
                                "merged": false,
                                "task": queued_ingest_task_payload("ingest-task-1", None)
                            }
                        }))
                    }
                },
                || {
                    test_runtime()
                        .block_on(run(DevqlArgs {
                            command: Some(ingest_enqueue_command(false)),
                        }))
                        .expect("devql ingest enqueue should succeed");
                },
            );
        },
    );

    let (query, variables) = captured
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert!(query.contains("enqueueTask"));
    assert_eq!(variables["input"]["kind"], json!("INGEST"));
}

#[test]
fn devql_run_ingest_enqueue_requires_current_daemon_before_graphql_mutation() {
    let repo = seed_devql_cli_repo();
    let bootstrap_count = Rc::new(RefCell::new(0usize));
    let query_count = Rc::new(RefCell::new(0usize));
    let ingested = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

    with_isolated_daemon_state(repo.path(), || {
        super::graphql::with_ingest_daemon_runtime_hook(
            {
                let bootstrap_count = Rc::clone(&bootstrap_count);
                move |_repo_root: &std::path::Path| {
                    *bootstrap_count.borrow_mut() += 1;
                    Ok(())
                }
            },
            || {
                with_graphql_executor_hook(
                    {
                        let query_count = Rc::clone(&query_count);
                        let ingested = Rc::clone(&ingested);
                        move |_repo_root: &std::path::Path,
                              query: &str,
                              variables: &serde_json::Value| {
                            *query_count.borrow_mut() += 1;
                            *ingested.borrow_mut() = Some((query.to_string(), variables.clone()));
                            Ok(json!({
                                "enqueueTask": {
                                    "merged": false,
                                    "task": queued_ingest_task_payload("ingest-task-2", None)
                                }
                            }))
                        }
                    },
                    || {
                        test_runtime()
                            .block_on(run(DevqlArgs {
                                command: Some(ingest_enqueue_command(false)),
                            }))
                            .expect("devql ingest enqueue should succeed");
                    },
                );
            },
        );
    });

    assert_eq!(*bootstrap_count.borrow(), 1);
    assert_eq!(*query_count.borrow(), 1);
    let (query, variables) = ingested
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert!(query.contains("enqueueTask"));
    assert_eq!(variables["input"]["kind"], json!("INGEST"));
}

#[test]
fn devql_reconcile_repo_watcher_via_runtime_graphql_calls_runtime_mutation() {
    let repo = seed_devql_cli_repo();
    with_isolated_daemon_state(repo.path(), || {
        let scope = crate::devql_transport::discover_slim_cli_repo_scope(Some(repo.path()))
            .expect("discover scope");
        let repo_id = scope.repo.repo_id.clone();
        let calls = Rc::new(RefCell::new(0usize));

        with_graphql_executor_hook(
            {
                let calls = Rc::clone(&calls);
                let repo_root = scope.repo_root.clone();
                let repo_id = repo_id.clone();
                move |actual_repo_root, query, variables| {
                    assert_eq!(actual_repo_root, repo_root.as_path());
                    assert!(
                        query.contains("reconcileWatcher("),
                        "unexpected GraphQL document: {query}"
                    );
                    assert_eq!(variables["repoId"], repo_id);
                    *calls.borrow_mut() += 1;
                    Ok(json!({
                        "reconcileWatcher": {
                            "repoId": repo_id,
                            "repoRoot": repo_root.display().to_string(),
                            "watcherEnabled": true,
                            "action": "restarted"
                        }
                    }))
                }
            },
            || {
                let record: RuntimeWatcherReconcileGraphqlRecord = test_runtime()
                    .block_on(reconcile_repo_watcher_via_runtime_graphql(&scope))
                    .expect("reconcile watcher");
                assert_eq!(record.repo_id, repo_id);
                assert_eq!(record.repo_root, scope.repo_root.display().to_string());
                assert!(record.watcher_enabled);
                assert_eq!(record.action, "restarted");
            },
        );

        assert_eq!(
            *calls.borrow(),
            1,
            "client should call runtime reconcileWatcher exactly once"
        );
    });
}

#[test]
fn devql_run_ingest_require_daemon_fails_without_bootstrap() {
    let repo = seed_devql_cli_repo();
    let bootstrap_count = Rc::new(RefCell::new(0usize));

    with_isolated_daemon_state(repo.path(), || {
        super::graphql::with_ingest_daemon_bootstrap_hook(
            {
                let bootstrap_count = Rc::clone(&bootstrap_count);
                move |_repo_root: &std::path::Path| {
                    *bootstrap_count.borrow_mut() += 1;
                    Ok(())
                }
            },
            || {
                let err = test_runtime()
                    .block_on(run(DevqlArgs {
                        command: Some(ingest_enqueue_command(true)),
                    }))
                    .expect_err(
                        "devql task ingest enqueue --require-daemon should fail without a daemon",
                    );

                assert!(
                    err.to_string().contains("Bitloops daemon is not running"),
                    "expected daemon-required error, got: {err:#}"
                );
            },
        );
    });

    assert_eq!(
        *bootstrap_count.borrow(),
        0,
        "daemon bootstrap should not be attempted when require_daemon is set"
    );
}

#[test]
fn devql_run_ingest_enqueues_via_graphql_even_when_enrichment_is_disabled() {
    let repo = seed_devql_cli_repo();
    let daemon_state_root = test_daemon_state_root(repo.path());
    fs::write(
        repo.path()
            .join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        format!(
            r#"[stores]
[stores.relational]
sqlite_path = {sqlite_path:?}

[stores.events]
duckdb_path = {duckdb_path:?}

[semantic_clones]
summary_mode = "off"
embedding_mode = "off"
"#,
            sqlite_path = daemon_state_root
                .join("stores")
                .join("relational")
                .join("devql.sqlite"),
            duckdb_path = daemon_state_root
                .join("stores")
                .join("event")
                .join("events.duckdb"),
        ),
    )
    .expect("write deterministic-only config");
    let graphql_calls = Rc::new(RefCell::new(0usize));
    with_isolated_daemon_state(repo.path(), || {
        write_current_runtime_state(repo.path());
        let cfg = crate::host::devql::DevqlConfig::from_env(
            repo.path().to_path_buf(),
            crate::host::devql::resolve_repo_identity(repo.path()).expect("resolve repo"),
        )
        .expect("build cfg");
        test_runtime()
            .block_on(crate::host::devql::execute_init_schema(
                &cfg,
                "devql cli deterministic test",
            ))
            .expect("initialise schema");

        with_graphql_executor_hook(
            {
                let graphql_calls = Rc::clone(&graphql_calls);
                move |_repo_root: &std::path::Path, _query: &str, _variables: &serde_json::Value| {
                    *graphql_calls.borrow_mut() += 1;
                    Ok(json!({
                        "enqueueTask": {
                            "merged": false,
                            "task": queued_ingest_task_payload("ingest-task-3", None)
                        }
                    }))
                }
            },
            || {
                test_runtime()
                    .block_on(run(DevqlArgs {
                        command: Some(ingest_enqueue_command(false)),
                    }))
                    .expect("daemon task ingest should succeed");
            },
        );
    });

    assert_eq!(*graphql_calls.borrow(), 1);
}

#[test]
fn devql_run_sync_executes_graphql_mutation() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let captured = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

    super::graphql::with_ingest_daemon_bootstrap_hook(
        |_repo_root: &std::path::Path| Ok(()),
        || {
            with_graphql_executor_hook(
                {
                    let captured = Rc::clone(&captured);
                    move |_repo_root: &std::path::Path,
                          query: &str,
                          variables: &serde_json::Value| {
                        *captured.borrow_mut() = Some((query.to_string(), variables.clone()));
                        Ok(json!({
                            "enqueueTask": {
                                "merged": false,
                                "task": queued_sync_task_payload("sync-task-1", "full", None)
                            }
                        }))
                    }
                },
                || {
                    test_runtime()
                        .block_on(run(DevqlArgs {
                            command: Some(sync_enqueue_command(
                                true, None, false, false, false, false,
                            )),
                        }))
                        .expect("devql sync should succeed");
                },
            );
        },
    );

    let (query, variables) = captured
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert!(
        query.contains("enqueueTask"),
        "expected enqueueTask mutation in query"
    );
    assert_eq!(variables["input"]["kind"], json!("SYNC"));
    assert_eq!(variables["input"]["sync"]["full"], json!(true));
}

#[test]
fn devql_run_sync_passes_paths_to_graphql_mutation() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let captured = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

    super::graphql::with_ingest_daemon_bootstrap_hook(
        |_repo_root: &std::path::Path| Ok(()),
        || {
            with_graphql_executor_hook(
                {
                    let captured = Rc::clone(&captured);
                    move |_repo_root: &std::path::Path,
                          query: &str,
                          variables: &serde_json::Value| {
                        *captured.borrow_mut() = Some((query.to_string(), variables.clone()));
                        Ok(json!({
                            "enqueueTask": {
                                "merged": false,
                                "task": queued_sync_task_payload(
                                    "sync-task-2",
                                    "paths",
                                    Some(vec!["src/lib.rs", "src/main.rs"]),
                                )
                            }
                        }))
                    }
                },
                || {
                    test_runtime()
                        .block_on(run(DevqlArgs {
                            command: Some(sync_enqueue_command(
                                false,
                                Some(vec!["src/lib.rs".to_string(), "src/main.rs".to_string()]),
                                false,
                                false,
                                false,
                                false,
                            )),
                        }))
                        .expect("devql sync with paths should succeed");
                },
            );
        },
    );

    let (_query, variables) = captured
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert_eq!(
        variables["input"]["sync"]["paths"],
        json!(["src/lib.rs", "src/main.rs"])
    );
}

#[test]
fn devql_run_sync_ensures_daemon_available() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let bootstrap_count = Rc::new(RefCell::new(0usize));

    super::graphql::with_ingest_daemon_bootstrap_hook(
        {
            let bootstrap_count = Rc::clone(&bootstrap_count);
            move |_repo_root: &std::path::Path| {
                *bootstrap_count.borrow_mut() += 1;
                Ok(())
            }
        },
        || {
            with_graphql_executor_hook(
                |_repo_root: &std::path::Path, _query: &str, _variables: &serde_json::Value| {
                    Ok(json!({
                        "enqueueTask": {
                            "merged": false,
                            "task": queued_sync_task_payload("sync-task-3", "auto", None)
                        }
                    }))
                },
                || {
                    test_runtime()
                        .block_on(run(DevqlArgs {
                            command: Some(sync_enqueue_command(
                                false, None, false, false, false, false,
                            )),
                        }))
                        .expect("devql sync should succeed");
                },
            );
        },
    );

    assert_eq!(
        *bootstrap_count.borrow(),
        1,
        "daemon bootstrap should be called once"
    );
}

#[test]
fn devql_run_sync_require_daemon_fails_without_bootstrap() {
    let repo = seed_devql_cli_repo();
    let bootstrap_count = Rc::new(RefCell::new(0usize));

    with_isolated_daemon_state(repo.path(), || {
        super::graphql::with_ingest_daemon_bootstrap_hook(
            {
                let bootstrap_count = Rc::clone(&bootstrap_count);
                move |_repo_root: &std::path::Path| {
                    *bootstrap_count.borrow_mut() += 1;
                    Ok(())
                }
            },
            || {
                let err = test_runtime()
                    .block_on(run(DevqlArgs {
                        command: Some(sync_enqueue_command(false, None, false, false, false, true)),
                    }))
                    .expect_err("devql sync --require-daemon should fail without a daemon");

                assert!(
                    err.to_string().contains("Bitloops daemon is not running"),
                    "expected daemon-required error, got: {err:#}"
                );
            },
        );
    });

    assert_eq!(
        *bootstrap_count.borrow(),
        0,
        "daemon bootstrap should not be attempted when require_daemon is set"
    );
}

#[test]
fn devql_run_projection_checkpoint_file_snapshots_succeeds_for_empty_repo() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);

    test_runtime()
        .block_on(run(DevqlArgs {
            command: Some(DevqlCommand::Projection(DevqlProjectionArgs {
                command: DevqlProjectionCommand::CheckpointFileSnapshots(
                    DevqlCheckpointFileSnapshotsArgs {
                        batch_size: 10,
                        max_checkpoints: Some(5),
                        resume_after: None,
                        dry_run: true,
                    },
                ),
            })),
        }))
        .expect("projection backfill should succeed for repo without checkpoints");

    let conn = Connection::open(sqlite_path_for_repo(repo.path())).expect("open sqlite");
    let projection_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM checkpoint_files", [], |row| {
            row.get(0)
        })
        .expect("count checkpoint_files rows");
    assert_eq!(projection_count, 0);
}

#[test]
fn devql_run_knowledge_add_executes_graphql_mutation() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let captured = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

    with_graphql_executor_hook(
        {
            let captured = Rc::clone(&captured);
            move |_repo_root: &std::path::Path, query: &str, variables: &serde_json::Value| {
                *captured.borrow_mut() = Some((query.to_string(), variables.clone()));
                Ok(json!({
                    "addKnowledge": {
                        "success": true,
                        "knowledgeItemVersionId": "version-1",
                        "itemCreated": true,
                        "newVersionCreated": true,
                        "knowledgeItem": {
                            "id": "knowledge-item-1",
                            "provider": "JIRA",
                            "sourceKind": "JIRA_ISSUE"
                        },
                        "association": null
                    }
                }))
            }
        },
        || {
            test_runtime()
                .block_on(run(DevqlArgs {
                    command: Some(DevqlCommand::Knowledge(DevqlKnowledgeArgs {
                        command: DevqlKnowledgeCommand::Add(DevqlKnowledgeAddArgs {
                            url: "https://example.atlassian.net/browse/CLI-1525".to_string(),
                            commit: Some("HEAD".to_string()),
                        }),
                    })),
                }))
                .expect("devql knowledge add should succeed");
        },
    );

    let (query, variables) = captured
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert!(query.contains("addKnowledge"));
    assert_eq!(
        variables["input"]["url"],
        json!("https://example.atlassian.net/browse/CLI-1525")
    );
    assert_eq!(variables["input"]["commitRef"], json!("HEAD"));
}

#[test]
fn devql_run_knowledge_associate_executes_graphql_mutation() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let captured = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

    with_graphql_executor_hook(
        {
            let captured = Rc::clone(&captured);
            move |_repo_root: &std::path::Path, query: &str, variables: &serde_json::Value| {
                *captured.borrow_mut() = Some((query.to_string(), variables.clone()));
                Ok(json!({
                    "associateKnowledge": {
                        "success": true,
                        "relation": {
                            "id": "relation-1",
                            "targetType": "COMMIT",
                            "targetId": "abc123",
                            "relationType": "associated_with",
                            "associationMethod": "manual_attachment"
                        }
                    }
                }))
            }
        },
        || {
            test_runtime()
                .block_on(run(DevqlArgs {
                    command: Some(DevqlCommand::Knowledge(DevqlKnowledgeArgs {
                        command: DevqlKnowledgeCommand::Associate(DevqlKnowledgeAssociateArgs {
                            source_ref: "knowledge:item-1".to_string(),
                            target_ref: "commit:abc123".to_string(),
                        }),
                    })),
                }))
                .expect("devql knowledge associate should succeed");
        },
    );

    let (query, variables) = captured
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert!(query.contains("associateKnowledge"));
    assert_eq!(variables["input"]["sourceRef"], json!("knowledge:item-1"));
    assert_eq!(variables["input"]["targetRef"], json!("commit:abc123"));
}

#[test]
fn devql_run_knowledge_associate_accepts_path_target_response() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let captured = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

    with_graphql_executor_hook(
        {
            let captured = Rc::clone(&captured);
            move |_repo_root: &std::path::Path, query: &str, variables: &serde_json::Value| {
                *captured.borrow_mut() = Some((query.to_string(), variables.clone()));
                Ok(json!({
                    "associateKnowledge": {
                        "success": true,
                        "relation": {
                            "id": "relation-path-1",
                            "targetType": "PATH",
                            "targetId": "axum-macros/src/from_request.rs",
                            "relationType": "associated_with",
                            "associationMethod": "manual_attachment"
                        }
                    }
                }))
            }
        },
        || {
            test_runtime()
                .block_on(run(DevqlArgs {
                    command: Some(DevqlCommand::Knowledge(DevqlKnowledgeArgs {
                        command: DevqlKnowledgeCommand::Associate(DevqlKnowledgeAssociateArgs {
                            source_ref: "knowledge:item-1".to_string(),
                            target_ref: "path:axum-macros/src/from_request.rs".to_string(),
                        }),
                    })),
                }))
                .expect("devql knowledge associate should accept path target response");
        },
    );

    let (query, variables) = captured
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert!(query.contains("associateKnowledge"));
    assert_eq!(variables["input"]["sourceRef"], json!("knowledge:item-1"));
    assert_eq!(
        variables["input"]["targetRef"],
        json!("path:axum-macros/src/from_request.rs")
    );
}

#[test]
fn devql_run_knowledge_associate_accepts_symbol_target_response() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let captured = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

    with_graphql_executor_hook(
        {
            let captured = Rc::clone(&captured);
            move |_repo_root: &std::path::Path, query: &str, variables: &serde_json::Value| {
                *captured.borrow_mut() = Some((query.to_string(), variables.clone()));
                Ok(json!({
                    "associateKnowledge": {
                        "success": true,
                        "relation": {
                            "id": "relation-symbol-1",
                            "targetType": "SYMBOL_FQN",
                            "targetId": "axum-macros/src/from_request.rs::expand",
                            "relationType": "associated_with",
                            "associationMethod": "manual_attachment"
                        }
                    }
                }))
            }
        },
        || {
            test_runtime()
                .block_on(run(DevqlArgs {
                    command: Some(DevqlCommand::Knowledge(DevqlKnowledgeArgs {
                        command: DevqlKnowledgeCommand::Associate(DevqlKnowledgeAssociateArgs {
                            source_ref: "knowledge:item-1".to_string(),
                            target_ref: "symbol_fqn:axum-macros/src/from_request.rs::expand"
                                .to_string(),
                        }),
                    })),
                }))
                .expect("devql knowledge associate should accept symbol target response");
        },
    );

    let (query, variables) = captured
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert!(query.contains("associateKnowledge"));
    assert_eq!(variables["input"]["sourceRef"], json!("knowledge:item-1"));
    assert_eq!(
        variables["input"]["targetRef"],
        json!("symbol_fqn:axum-macros/src/from_request.rs::expand")
    );
}

#[test]
fn devql_run_knowledge_refresh_executes_graphql_mutation() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let captured = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

    with_graphql_executor_hook(
        {
            let captured = Rc::clone(&captured);
            move |_repo_root: &std::path::Path, query: &str, variables: &serde_json::Value| {
                *captured.borrow_mut() = Some((query.to_string(), variables.clone()));
                Ok(json!({
                    "refreshKnowledge": {
                        "success": true,
                        "latestDocumentVersionId": "version-2",
                        "contentChanged": true,
                        "newVersionCreated": true,
                        "knowledgeItem": {
                            "id": "knowledge-item-1",
                            "provider": "JIRA",
                            "sourceKind": "JIRA_ISSUE"
                        }
                    }
                }))
            }
        },
        || {
            test_runtime()
                .block_on(run(DevqlArgs {
                    command: Some(DevqlCommand::Knowledge(DevqlKnowledgeArgs {
                        command: DevqlKnowledgeCommand::Refresh(DevqlKnowledgeRefArgs {
                            knowledge_ref: "knowledge:item-1".to_string(),
                        }),
                    })),
                }))
                .expect("devql knowledge refresh should succeed");
        },
    );

    let (query, variables) = captured
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert!(query.contains("refreshKnowledge"));
    assert_eq!(
        variables["input"]["knowledgeRef"],
        json!("knowledge:item-1")
    );
}
