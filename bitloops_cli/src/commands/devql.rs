use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::engine::devql::{DevqlConfig, resolve_repo_identity, run_ingest, run_init, run_query};
use crate::engine::paths;

pub use crate::engine::devql::run_connection_status;

const MISSING_SUBCOMMAND_MESSAGE: &str = "missing subcommand. Use one of: `bitloops devql init`, `bitloops devql ingest`, `bitloops devql query`, `bitloops devql connection-status`";

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlArgs {
    #[command(subcommand)]
    pub command: Option<DevqlCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlCommand {
    /// Create ClickHouse/Postgres schema required by DevQL MVP.
    Init(DevqlInitArgs),
    /// Ingest checkpoint/event data into ClickHouse and file artefacts into Postgres.
    Ingest(DevqlIngestArgs),
    /// Execute an MVP DevQL query.
    Query(DevqlQueryArgs),
    /// Check backend connectivity for Postgres and ClickHouse.
    ConnectionStatus(DevqlConnectionStatusArgs),
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

pub async fn run(args: DevqlArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(MISSING_SUBCOMMAND_MESSAGE);
    };

    if matches!(&command, DevqlCommand::ConnectionStatus(_)) {
        return run_connection_status().await;
    }

    let repo_root = paths::repo_root()?;
    let repo = resolve_repo_identity(&repo_root)?;
    let cfg = DevqlConfig::from_env(repo_root, repo);

    match command {
        DevqlCommand::Init(_) => run_init(&cfg).await,
        DevqlCommand::Ingest(args) => run_ingest(&cfg, args.init, args.max_checkpoints).await,
        DevqlCommand::Query(args) => run_query(&cfg, &args.query, args.compact).await,
        DevqlCommand::ConnectionStatus(_) => unreachable!("handled before repo setup"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Cli, Commands};
    use crate::test_support::process_state::{enter_process_state, git_command};
    use clap::Parser;
    use std::path::Path;
    use tempfile::TempDir;

    fn git_ok(repo_root: &Path, args: &[&str]) {
        let out = git_command()
            .args(args)
            .current_dir(repo_root)
            .output()
            .unwrap_or_else(|err| panic!("failed to start git {:?}: {err}", args));
        assert!(
            out.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn seed_git_repo() -> TempDir {
        let dir = TempDir::new().expect("temp dir");
        git_ok(dir.path(), &["init"]);
        git_ok(dir.path(), &["checkout", "-B", "main"]);
        git_ok(dir.path(), &["config", "user.name", "Bitloops Test"]);
        git_ok(
            dir.path(),
            &["config", "user.email", "bitloops-test@example.com"],
        );
        dir
    }

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
    fn devql_run_requires_subcommand() {
        let err = test_runtime()
            .block_on(run(DevqlArgs::default()))
            .expect_err("missing subcommand should error");

        assert!(err.to_string().contains(MISSING_SUBCOMMAND_MESSAGE));
    }

    #[test]
    fn devql_run_init_requires_pg_dsn_after_repo_resolution() {
        let repo = seed_git_repo();
        let home = TempDir::new().expect("home dir");
        let home_path = home.path().to_string_lossy().to_string();
        let _guard = enter_process_state(
            Some(repo.path()),
            &[
                ("HOME", Some(home_path.as_str())),
                ("USERPROFILE", Some(home_path.as_str())),
                ("BITLOOPS_DEVQL_PG_DSN", None),
                ("BITLOOPS_DEVQL_CH_URL", None),
                ("BITLOOPS_DEVQL_CH_USER", None),
                ("BITLOOPS_DEVQL_CH_PASSWORD", None),
                ("BITLOOPS_DEVQL_CH_DATABASE", None),
            ],
        );

        let err = test_runtime()
            .block_on(run(DevqlArgs {
                command: Some(DevqlCommand::Init(DevqlInitArgs::default())),
            }))
            .expect_err("missing PG DSN should error before DB setup");

        assert!(
            err.to_string()
                .contains("BITLOOPS_DEVQL_PG_DSN is required"),
            "unexpected error: {err:#}"
        );
    }
}
