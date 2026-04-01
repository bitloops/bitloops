use super::graphql::with_graphql_executor_hook;
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

fn write_envelope_config(repo_root: &Path, settings: serde_json::Value) {
    let sqlite_path = settings["stores"]["relational"]["sqlite_path"]
        .as_str()
        .expect("relational sqlite path");
    let duckdb_path = settings["stores"]["events"]["duckdb_path"]
        .as_str()
        .expect("events duckdb path");
    let embedding_provider = settings["stores"]["embedding_provider"]
        .as_str()
        .expect("embedding provider");
    let semantic_provider = settings["semantic"]["provider"]
        .as_str()
        .expect("semantic provider");

    fs::write(
        repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        format!(
            r#"[stores]
embedding_provider = {embedding_provider:?}

[stores.relational]
sqlite_path = {sqlite_path:?}

[stores.events]
duckdb_path = {duckdb_path:?}

[semantic]
provider = {semantic_provider:?}
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

    write_envelope_config(
        repo_root,
        json!({
            "stores": {
                "relational": {
                    "sqlite_path": ".bitloops/stores/devql.sqlite"
                },
                "events": {
                    "duckdb_path": ".bitloops/stores/events.duckdb"
                },
                "embedding_provider": "disabled"
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

    assert!(ingest.init);
    assert_eq!(ingest.max_checkpoints, 500);
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

    with_graphql_executor_hook(
        {
            let captured = Rc::clone(&captured);
            move |_repo_root: &std::path::Path, query: &str, variables: &serde_json::Value| {
                *captured.borrow_mut() = Some((query.to_string(), variables.clone()));
                Ok(json!({
                    "ingest": {
                        "success": true,
                        "initRequested": false,
                        "checkpointsProcessed": 2,
                        "eventsInserted": 3,
                        "artefactsUpserted": 5,
                        "checkpointsWithoutCommit": 0,
                        "temporaryRowsPromoted": 0,
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
                        init: false,
                        max_checkpoints: 42,
                    })),
                }))
                .expect("devql ingest should succeed");
        },
    );

    let (query, variables) = captured
        .borrow_mut()
        .take()
        .expect("graphql mutation should be captured");
    assert!(query.contains("ingest"));
    assert_eq!(
        variables,
        json!({
            "input": {
                "init": false,
                "maxCheckpoints": 42
            }
        })
    );
}

#[test]
fn devql_run_ingest_requires_running_daemon() {
    let repo = seed_devql_cli_repo();
    with_isolated_daemon_state(repo.path(), || {
        let err = test_runtime()
            .block_on(run(DevqlArgs {
                command: Some(DevqlCommand::Ingest(DevqlIngestArgs {
                    init: true,
                    max_checkpoints: 500,
                })),
            }))
            .expect_err("devql ingest should require a running daemon");

        assert!(
            err.to_string().contains("Bitloops daemon is not running"),
            "expected daemon-required error, got: {err:#}"
        );
    });
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
        .query_row(
            "SELECT COUNT(*) FROM checkpoint_file_snapshots",
            [],
            |row| row.get(0),
        )
        .expect("count checkpoint_file_snapshots rows");
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
