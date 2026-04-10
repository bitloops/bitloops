use super::graphql::{with_graphql_executor_hook, with_schema_sdl_fetch_hook};
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

fn test_daemon_state_root(repo_root: &Path) -> PathBuf {
    repo_root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| repo_root.to_path_buf())
        .join(".bitloops-test-state")
        .join(
            repo_root
                .file_name()
                .map(|name| name.to_os_string())
                .unwrap_or_default(),
        )
}

fn write_envelope_config(repo_root: &Path, settings: serde_json::Value) {
    let sqlite_path = settings["stores"]["relational"]["sqlite_path"]
        .as_str()
        .expect("relational sqlite path");
    let duckdb_path = settings["stores"]["events"]["duckdb_path"]
        .as_str()
        .expect("events duckdb path");

    fs::write(
        repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
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

#[test]
fn devql_cli_parses_ingest_defaults() {
    let parsed =
        Cli::try_parse_from(["bitloops", "devql", "ingest"]).expect("devql ingest should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Ingest(ingest)) = args.command else {
        panic!("expected devql ingest command");
    };

    assert!(!ingest.require_daemon);
}

#[test]
fn devql_cli_parses_ingest_require_daemon_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "devql", "ingest", "--require-daemon"])
        .expect("devql ingest --require-daemon should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Ingest(ingest)) = args.command else {
        panic!("expected devql ingest command");
    };

    assert!(ingest.require_daemon);
}

#[test]
fn devql_cli_parses_sync_modes() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "sync",
        "--paths",
        "src/lib.rs,src/main.rs",
    ])
    .expect("devql sync with paths should parse");

    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Sync(sync)) = args.command else {
        panic!("expected devql sync command");
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

    let parsed = Cli::try_parse_from(["bitloops", "devql", "sync", "--repair"])
        .expect("devql sync repair should parse");
    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Sync(sync)) = args.command else {
        panic!("expected devql sync command");
    };
    assert!(!sync.full);
    assert_eq!(sync.paths, None);
    assert!(sync.repair);
    assert!(!sync.validate);
    assert!(!sync.status);
    assert!(!sync.require_daemon);

    let parsed = Cli::try_parse_from(["bitloops", "devql", "sync", "--full"])
        .expect("devql sync full should parse");
    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Sync(sync)) = args.command else {
        panic!("expected devql sync command");
    };
    assert!(sync.full);
    assert_eq!(sync.paths, None);
    assert!(!sync.repair);
    assert!(!sync.validate);
    assert!(!sync.status);
    assert!(!sync.require_daemon);

    let parsed = Cli::try_parse_from(["bitloops", "devql", "sync", "--validate"])
        .expect("devql sync validate should parse");
    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Sync(sync)) = args.command else {
        panic!("expected devql sync command");
    };
    assert!(!sync.full);
    assert_eq!(sync.paths, None);
    assert!(!sync.repair);
    assert!(sync.validate);
    assert!(!sync.status);
    assert!(!sync.require_daemon);
}

#[test]
fn devql_cli_parses_sync_status_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "devql", "sync", "--status"])
        .expect("devql sync --status should parse");
    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Sync(sync)) = args.command else {
        panic!("expected devql sync command");
    };
    assert!(sync.status);
    assert!(!sync.require_daemon);
}

#[test]
fn devql_cli_parses_sync_require_daemon_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "devql", "sync", "--require-daemon"])
        .expect("devql sync --require-daemon should parse");
    let Some(Commands::Devql(args)) = parsed.command else {
        panic!("expected devql command");
    };
    let Some(DevqlCommand::Sync(sync)) = args.command else {
        panic!("expected devql sync command");
    };
    assert!(sync.require_daemon);
}

#[test]
fn devql_cli_rejects_conflicting_sync_modes() {
    let cases = vec![
        vec![
            "bitloops",
            "devql",
            "sync",
            "--full",
            "--paths",
            "src/lib.rs",
        ],
        vec!["bitloops", "devql", "sync", "--full", "--repair"],
        vec![
            "bitloops",
            "devql",
            "sync",
            "--paths",
            "src/lib.rs",
            "--repair",
        ],
        vec!["bitloops", "devql", "sync", "--validate", "--repair"],
        vec!["bitloops", "devql", "sync", "--validate", "--full"],
        vec![
            "bitloops",
            "devql",
            "sync",
            "--validate",
            "--paths",
            "src/lib.rs",
        ],
        vec![
            "bitloops",
            "devql",
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
fn devql_run_requires_subcommand() {
    let err = test_runtime()
        .block_on(run(DevqlArgs::default()))
        .expect_err("missing subcommand should error");

    assert!(err.to_string().contains(MISSING_SUBCOMMAND_MESSAGE));
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
fn devql_run_ingest_executes_graphql_mutation_with_expected_input() {
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
                            "ingest": {
                                "success": true,
                                "commitsProcessed": 2,
                                "checkpointCompanionsProcessed": 1,
                                "eventsInserted": 3,
                                "artefactsUpserted": 5,
                                "semanticFeatureRowsUpserted": 0,
                                "semanticFeatureRowsSkipped": 0,
                                "symbolEmbeddingRowsUpserted": 0,
                                "symbolEmbeddingRowsSkipped": 0,
                                "symbolCloneEdgesUpserted": 0,
                                "symbolCloneSourcesScored": 0
                            }
                        }))
                    }
                },
                || {
                    test_runtime()
                        .block_on(run(DevqlArgs {
                            command: Some(DevqlCommand::Ingest(DevqlIngestArgs {
                                require_daemon: false,
                            })),
                        }))
                        .expect("devql ingest should succeed");
                },
            );
        },
    );

    let (query, variables) = captured
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert!(query.contains("ingest"));
    assert_eq!(variables, json!({}));
}

#[test]
fn devql_run_ingest_requires_current_daemon_before_graphql_mutation() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let bootstrap_count = Rc::new(RefCell::new(0usize));
    let query_count = Rc::new(RefCell::new(0usize));
    let ingested = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

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
                            "ingest": {
                                "success": true,
                                "commitsProcessed": 0,
                                "checkpointCompanionsProcessed": 0,
                                "eventsInserted": 0,
                                "artefactsUpserted": 0,
                                "semanticFeatureRowsUpserted": 0,
                                "semanticFeatureRowsSkipped": 0,
                                "symbolEmbeddingRowsUpserted": 0,
                                "symbolEmbeddingRowsSkipped": 0,
                                "symbolCloneEdgesUpserted": 0,
                                "symbolCloneSourcesScored": 0
                            }
                        }))
                    }
                },
                || {
                    test_runtime()
                        .block_on(run(DevqlArgs {
                            command: Some(DevqlCommand::Ingest(DevqlIngestArgs {
                                require_daemon: false,
                            })),
                        }))
                        .expect("devql ingest should succeed");
                },
            );
        },
    );

    assert_eq!(*bootstrap_count.borrow(), 1);
    assert_eq!(*query_count.borrow(), 1);
    let (query, variables) = ingested
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert!(query.contains("ingest"));
    assert_eq!(variables, json!({}));
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
                        command: Some(DevqlCommand::Ingest(DevqlIngestArgs {
                            require_daemon: true,
                        })),
                    }))
                    .expect_err("devql ingest --require-daemon should fail without a daemon");

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
fn devql_run_ingest_stays_local_when_enrichment_is_disabled() {
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
                    panic!("graphql should not be used when enrichment is disabled");
                }
            },
            || {
                test_runtime()
                    .block_on(run(DevqlArgs {
                        command: Some(DevqlCommand::Ingest(DevqlIngestArgs::default())),
                    }))
                    .expect("local deterministic ingest should succeed");
            },
        );
    });

    assert_eq!(*graphql_calls.borrow(), 0);
    with_isolated_daemon_state(repo.path(), || {
        let err = test_runtime()
            .block_on(run(DevqlArgs {
                command: Some(DevqlCommand::Ingest(DevqlIngestArgs::default())),
            }))
            .expect_err("deterministic-only ingest should require a current daemon");
        assert!(
            format!("{err:#}").contains("bitloops init")
                || format!("{err:#}").contains("daemon restart"),
            "unexpected error: {err:#}"
        );
    });
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
                            "enqueueSync": {
                                "merged": false,
                                "task": {
                                    "taskId": "sync-task-1",
                                    "repoId": "repo-1",
                                    "repoName": "demo",
                                    "repoIdentity": "local/demo",
                                    "source": "manual_cli",
                                    "mode": "full",
                                    "status": "queued",
                                    "phase": "queued",
                                    "submittedAtUnix": 1,
                                    "startedAtUnix": null,
                                    "updatedAtUnix": 1,
                                    "completedAtUnix": null,
                                    "queuePosition": 1,
                                    "tasksAhead": 0,
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
                                    "parseErrors": 0,
                                    "error": null,
                                    "summary": null
                                }
                            }
                        }))
                    }
                },
                || {
                    test_runtime()
                        .block_on(run(DevqlArgs {
                            command: Some(DevqlCommand::Sync(DevqlSyncArgs {
                                full: true,
                                paths: None,
                                repair: false,
                                validate: false,
                                status: false,
                                require_daemon: false,
                            })),
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
        query.contains("enqueueSync"),
        "expected enqueueSync mutation in query"
    );
    assert_eq!(variables["input"]["full"], json!(true));
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
                            "enqueueSync": {
                                "merged": false,
                                "task": {
                                    "taskId": "sync-task-2",
                                    "repoId": "repo-1",
                                    "repoName": "demo",
                                    "repoIdentity": "local/demo",
                                    "source": "manual_cli",
                                    "mode": "paths",
                                    "status": "queued",
                                    "phase": "queued",
                                    "submittedAtUnix": 1,
                                    "startedAtUnix": null,
                                    "updatedAtUnix": 1,
                                    "completedAtUnix": null,
                                    "queuePosition": 1,
                                    "tasksAhead": 0,
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
                                    "parseErrors": 0,
                                    "error": null,
                                    "summary": null
                                }
                            }
                        }))
                    }
                },
                || {
                    test_runtime()
                        .block_on(run(DevqlArgs {
                            command: Some(DevqlCommand::Sync(DevqlSyncArgs {
                                full: false,
                                paths: Some(vec![
                                    "src/lib.rs".to_string(),
                                    "src/main.rs".to_string(),
                                ]),
                                repair: false,
                                validate: false,
                                status: false,
                                require_daemon: false,
                            })),
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
        variables["input"]["paths"],
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
                        "enqueueSync": {
                            "merged": false,
                            "task": {
                                "taskId": "sync-task-3",
                                "repoId": "repo-1",
                                "repoName": "demo",
                                "repoIdentity": "local/demo",
                                "source": "manual_cli",
                                "mode": "auto",
                                "status": "queued",
                                "phase": "queued",
                                "submittedAtUnix": 1,
                                "startedAtUnix": null,
                                "updatedAtUnix": 1,
                                "completedAtUnix": null,
                                "queuePosition": 1,
                                "tasksAhead": 0,
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
                                "parseErrors": 0,
                                "error": null,
                                "summary": null
                            }
                        }
                    }))
                },
                || {
                    test_runtime()
                        .block_on(run(DevqlArgs {
                            command: Some(DevqlCommand::Sync(DevqlSyncArgs {
                                full: false,
                                paths: None,
                                repair: false,
                                validate: false,
                                status: false,
                                require_daemon: false,
                            })),
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
                        command: Some(DevqlCommand::Sync(DevqlSyncArgs {
                            full: false,
                            paths: None,
                            repair: false,
                            validate: false,
                            status: false,
                            require_daemon: true,
                        })),
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
