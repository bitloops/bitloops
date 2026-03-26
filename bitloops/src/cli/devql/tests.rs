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
    let config_dir = repo_root.join(".bitloops");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "version": "1.0",
            "scope": "project",
            "settings": settings
        }))
        .expect("serialise config"),
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
            move |_repo_root, query, variables| {
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
fn devql_run_init_executes_graphql_mutation_and_is_idempotent() {
    let repo = seed_devql_cli_repo();
    let sqlite_path = sqlite_path_for_repo(repo.path());
    let _guard = enter_process_state(Some(repo.path()), &[]);

    test_runtime()
        .block_on(run(DevqlArgs {
            command: Some(DevqlCommand::Init(DevqlInitArgs::default())),
        }))
        .expect("devql init should succeed");
    test_runtime()
        .block_on(run(DevqlArgs {
            command: Some(DevqlCommand::Init(DevqlInitArgs::default())),
        }))
        .expect("second devql init should succeed");

    let conn = Connection::open(sqlite_path).expect("open sqlite");
    for table in ["repositories", "artefacts", "artefacts_current"] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("query sqlite schema");
        assert_eq!(count, 1, "expected sqlite table `{table}`");
    }
}

#[test]
fn devql_run_ingest_executes_graphql_mutation_with_expected_input() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let captured = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

    with_graphql_executor_hook(
        {
            let captured = Rc::clone(&captured);
            move |_repo_root, query, variables| {
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
fn devql_run_ingest_executes_graphql_mutation_and_persists_repository_row() {
    let repo = seed_devql_cli_repo();
    let sqlite_path = sqlite_path_for_repo(repo.path());
    let _guard = enter_process_state(Some(repo.path()), &[]);

    test_runtime()
        .block_on(run(DevqlArgs {
            command: Some(DevqlCommand::Ingest(DevqlIngestArgs {
                init: true,
                max_checkpoints: 500,
            })),
        }))
        .expect("devql ingest should succeed");

    let conn = Connection::open(sqlite_path).expect("open sqlite");
    let repository_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM repositories", [], |row| row.get(0))
        .expect("count repositories");
    assert_eq!(repository_count, 1, "expected one repository row");
}

#[test]
fn devql_run_knowledge_add_executes_graphql_mutation() {
    let repo = seed_devql_cli_repo();
    let _guard = enter_process_state(Some(repo.path()), &[]);
    let captured = Rc::new(RefCell::new(None::<(String, serde_json::Value)>));

    with_graphql_executor_hook(
        {
            let captured = Rc::clone(&captured);
            move |_repo_root, query, variables| {
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
            move |_repo_root, query, variables| {
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
            move |_repo_root, query, variables| {
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
