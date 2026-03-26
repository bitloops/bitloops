use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use serde::de::DeserializeOwned;
use serde_json::json;
use std::path::Path;

use crate::capability_packs::knowledge::{
    run_knowledge_add_via_host, run_knowledge_associate_via_host, run_knowledge_refresh_via_host,
    run_knowledge_versions_via_host,
};
use crate::host::devql::{
    DevqlConfig, IngestionCounters, InitSchemaSummary, format_ingestion_summary,
    format_init_schema_summary, resolve_repo_identity, run_capability_packs_report, run_query,
};
use crate::utils::paths;

pub use crate::host::devql::run_connection_status;

pub(crate) const MISSING_SUBCOMMAND_MESSAGE: &str = "missing subcommand. Use one of: `bitloops devql init`, `bitloops devql ingest`, `bitloops devql query`, `bitloops devql connection-status`, `bitloops devql packs`, `bitloops devql knowledge add`, `bitloops devql knowledge associate`, `bitloops devql knowledge refresh`, `bitloops devql knowledge versions`";

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlArgs {
    #[command(subcommand)]
    pub command: Option<DevqlCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlCommand {
    /// Create schema for configured relational/events backends.
    Init(DevqlInitArgs),
    /// Ingest checkpoint/events and relational artefacts for configured backends.
    Ingest(DevqlIngestArgs),
    /// Execute a DevQL query.
    Query(DevqlQueryArgs),
    /// Check backend connectivity for Postgres and ClickHouse.
    ConnectionStatus(DevqlConnectionStatusArgs),
    /// List registered capability packs, migrations, and host policy (optional health checks).
    Packs(DevqlPacksArgs),
    /// Manage repository-scoped external knowledge.
    Knowledge(DevqlKnowledgeArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlInitArgs {}

#[derive(Args, Debug, Clone)]
pub struct DevqlIngestArgs {
    /// Bootstrap tables before ingestion.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub init: bool,

    /// Limit checkpoints processed (newest-first).
    #[arg(long, default_value_t = 500)]
    pub max_checkpoints: usize,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlQueryArgs {
    /// DevQL pipeline query string.
    pub query: String,

    /// Print compact JSON.
    #[arg(long, default_value_t = false)]
    pub compact: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlConnectionStatusArgs {}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlPacksArgs {
    /// Emit JSON instead of human-readable text.
    #[arg(long, default_value_t = false)]
    pub json: bool,

    /// Run each pack's registered health checks (may read config and probe store paths).
    #[arg(long, default_value_t = false)]
    pub with_health: bool,

    /// Apply registered pack migrations before reporting (same as ingest/init migration pass).
    #[arg(long, default_value_t = false)]
    pub apply_migrations: bool,

    /// Include `CoreExtensionHost` (language packs + extension capability descriptors, readiness, diagnostics).
    #[arg(long, default_value_t = false)]
    pub with_extensions: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlKnowledgeArgs {
    #[command(subcommand)]
    pub command: DevqlKnowledgeCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlKnowledgeCommand {
    /// Manually add repository-scoped external knowledge by URL.
    Add(DevqlKnowledgeAddArgs),
    /// Associate existing knowledge to a typed Bitloops target.
    Associate(DevqlKnowledgeAssociateArgs),
    /// Refresh an existing knowledge source from provider and create a new immutable version if changed.
    Refresh(DevqlKnowledgeRefArgs),
    /// List immutable document versions for a knowledge item.
    Versions(DevqlKnowledgeRefArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DevqlKnowledgeAddArgs {
    pub url: String,

    #[arg(long)]
    pub commit: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlKnowledgeAssociateArgs {
    pub source_ref: String,

    #[arg(long = "to")]
    pub target_ref: String,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlKnowledgeRefArgs {
    pub knowledge_ref: String,
}

const INIT_SCHEMA_MUTATION: &str = r#"
    mutation InitSchema {
      initSchema {
        success
        repoIdentity
        repoId
        relationalBackend
        eventsBackend
      }
    }
"#;

const INGEST_MUTATION: &str = r#"
    mutation Ingest($input: IngestInput!) {
      ingest(input: $input) {
        success
        initRequested
        checkpointsProcessed
        eventsInserted
        artefactsUpserted
        checkpointsWithoutCommit
        temporaryRowsPromoted
        semanticFeatureRowsUpserted
        semanticFeatureRowsSkipped
        symbolEmbeddingRowsUpserted
        symbolEmbeddingRowsSkipped
        symbolCloneEdgesUpserted
        symbolCloneSourcesScored
      }
    }
"#;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitSchemaMutationData {
    init_schema: InitSchemaSummary,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct IngestMutationData {
    ingest: IngestionCounters,
}

async fn execute_devql_graphql<T: DeserializeOwned>(
    repo_root: &Path,
    query: &str,
    variables: serde_json::Value,
) -> Result<T> {
    let data =
        crate::graphql::execute_in_process(repo_root.to_path_buf(), query, variables).await?;
    Ok(serde_json::from_value(data)?)
}

async fn run_init_via_graphql(repo_root: &Path) -> Result<()> {
    let response: InitSchemaMutationData =
        execute_devql_graphql(repo_root, INIT_SCHEMA_MUTATION, json!({})).await?;
    println!("{}", format_init_schema_summary(&response.init_schema));
    Ok(())
}

async fn run_ingest_via_graphql(
    repo_root: &Path,
    init: bool,
    max_checkpoints: usize,
) -> Result<()> {
    let response: IngestMutationData = execute_devql_graphql(
        repo_root,
        INGEST_MUTATION,
        json!({
            "input": {
                "init": init,
                "maxCheckpoints": max_checkpoints,
            }
        }),
    )
    .await?;
    println!("{}", format_ingestion_summary(&response.ingest));
    Ok(())
}

pub async fn run(args: DevqlArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(MISSING_SUBCOMMAND_MESSAGE);
    };

    if matches!(&command, DevqlCommand::ConnectionStatus(_)) {
        return run_connection_status().await;
    }

    let repo_root = paths::repo_root()?;
    let repo = resolve_repo_identity(&repo_root)?;

    if let DevqlCommand::Knowledge(args) = command {
        return match args.command {
            DevqlKnowledgeCommand::Add(add) => {
                run_knowledge_add_via_host(&repo_root, &repo, &add.url, add.commit.as_deref()).await
            }
            DevqlKnowledgeCommand::Associate(associate) => {
                run_knowledge_associate_via_host(
                    &repo_root,
                    &repo,
                    &associate.source_ref,
                    &associate.target_ref,
                )
                .await
            }
            DevqlKnowledgeCommand::Refresh(refresh) => {
                run_knowledge_refresh_via_host(&repo_root, &repo, &refresh.knowledge_ref).await
            }
            DevqlKnowledgeCommand::Versions(versions) => {
                run_knowledge_versions_via_host(&repo_root, &repo, &versions.knowledge_ref).await
            }
        };
    }

    let cfg = DevqlConfig::from_env(repo_root, repo)?;

    match command {
        DevqlCommand::Init(_) => run_init_via_graphql(&cfg.repo_root).await,
        DevqlCommand::Ingest(args) => {
            run_ingest_via_graphql(&cfg.repo_root, args.init, args.max_checkpoints).await
        }
        DevqlCommand::Query(args) => run_query(&cfg, &args.query, args.compact).await,
        DevqlCommand::Packs(args) => run_capability_packs_report(
            &cfg,
            args.json,
            args.apply_migrations,
            args.with_health,
            args.with_extensions,
        ),
        DevqlCommand::ConnectionStatus(_) => unreachable!("handled before repo setup"),
        DevqlCommand::Knowledge(_) => unreachable!("handled before cfg setup"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use crate::test_support::git_fixtures::{git_ok, init_test_repo};
    use crate::test_support::process_state::enter_process_state;
    use clap::Parser;
    use rusqlite::Connection;
    use std::fs;
    use std::path::{Path, PathBuf};
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
        let parsed = Cli::try_parse_from(["bitloops", "devql", "ingest"])
            .expect("devql ingest should parse");

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
}
