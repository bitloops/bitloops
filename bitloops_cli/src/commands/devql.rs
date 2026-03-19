use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::engine::devql::capabilities::knowledge::{
    run_knowledge_add_via_host, run_knowledge_associate_via_host, run_knowledge_refresh_via_host,
    run_knowledge_versions_via_host,
};
use crate::engine::devql::{DevqlConfig, resolve_repo_identity, run_ingest, run_init, run_query};
use crate::engine::paths;

pub use crate::engine::devql::run_connection_status;

pub(crate) const MISSING_SUBCOMMAND_MESSAGE: &str = "missing subcommand. Use one of: `bitloops devql init`, `bitloops devql ingest`, `bitloops devql query`, `bitloops devql connection-status`, `bitloops devql knowledge add`, `bitloops devql knowledge associate`, `bitloops devql knowledge refresh`, `bitloops devql knowledge versions`";

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
        DevqlCommand::Init(_) => run_init(&cfg).await,
        DevqlCommand::Ingest(args) => run_ingest(&cfg, args.init, args.max_checkpoints).await,
        DevqlCommand::Query(args) => run_query(&cfg, &args.query, args.compact).await,
        DevqlCommand::ConnectionStatus(_) => unreachable!("handled before repo setup"),
        DevqlCommand::Knowledge(_) => unreachable!("handled before cfg setup"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Cli, Commands};
    use clap::Parser;

    fn test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
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
}
