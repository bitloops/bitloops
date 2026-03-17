use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Commands};

pub mod commands;
pub mod queries;

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init {
            db: db_path,
            seed,
            commit,
        } => commands::init::handle(&db_path, seed, &commit),
        Commands::IngestTests {
            repo_dir,
            commit,
            db: db_path,
        } => commands::ingest_tests::handle(&db_path, &repo_dir, &commit),
        Commands::IngestProductionArtefacts {
            repo_dir,
            commit,
            db: db_path,
        } => commands::ingest_production_artefacts::handle(&db_path, &repo_dir, &commit),
        Commands::IngestCoverage {
            lcov,
            input,
            commit,
            scope,
            tool,
            test_artefact_id,
            format,
            db: db_path,
        } => commands::ingest_coverage::handle(
            &db_path,
            lcov.as_deref(),
            input.as_deref(),
            &commit,
            &scope,
            &tool,
            test_artefact_id.as_deref(),
            format.as_deref(),
        ),
        Commands::IngestCoverageBatch {
            manifest,
            commit,
            db: db_path,
        } => commands::ingest_coverage_batch::handle(&db_path, &manifest, &commit),
        Commands::IngestResults {
            jest_json,
            commit,
            db: db_path,
        } => commands::ingest_results::handle(&db_path, &jest_json, &commit),
        Commands::Query {
            artefact,
            commit,
            classification,
            view,
            min_strength,
            db: db_path,
        } => queries::query_artefact_harness::handle(
            &db_path,
            &artefact,
            &commit,
            classification.as_deref(),
            view,
            min_strength,
        ),
        Commands::List {
            commit,
            kind,
            db: db_path,
        } => queries::list_artefacts::handle(&db_path, &commit, kind.as_deref()),
    }
}
