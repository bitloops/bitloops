use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

pub const DEFAULT_DB_PATH: &str = "./testlens.db";
pub const DEFAULT_SEED_COMMIT: &str = "abc123";
pub const DEFAULT_QUERY_VIEW: QueryViewArg = QueryViewArg::Full;

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum QueryViewArg {
    Full,
    Summary,
    Tests,
    Coverage,
}

#[derive(Parser, Debug)]
#[command(name = "testlens", version, about = "Prototype verification lens CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize database with schema
    Init {
        #[arg(long, default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
        #[arg(long, default_value_t = false)]
        seed: bool,
        #[arg(long, default_value = DEFAULT_SEED_COMMIT)]
        commit: String,
    },
    /// Parse test files, discover suites/scenarios, and link tests to production artefacts
    IngestTests {
        #[arg(long)]
        repo_dir: PathBuf,
        #[arg(long)]
        commit: String,
        #[arg(long, default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
    },
    /// Parse production files and materialize production artefacts
    IngestProductionArtefacts {
        #[arg(long)]
        repo_dir: PathBuf,
        #[arg(long)]
        commit: String,
        #[arg(long, default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
    },
    /// Ingest LCOV coverage report
    IngestCoverage {
        #[arg(long)]
        lcov: PathBuf,
        #[arg(long)]
        commit: String,
        #[arg(long, default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
    },
    /// Ingest Jest JSON test results
    IngestResults {
        #[arg(long)]
        jest_json: PathBuf,
        #[arg(long)]
        commit: String,
        #[arg(long, default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
    },
    /// Query test harness for an artefact
    Query {
        #[arg(long)]
        artefact: String,
        #[arg(long)]
        commit: String,
        #[arg(long)]
        classification: Option<String>,
        #[arg(long, value_enum, default_value_t = DEFAULT_QUERY_VIEW)]
        view: QueryViewArg,
        #[arg(long)]
        min_strength: Option<f64>,
        #[arg(long, default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
    },
    /// List known artefacts
    List {
        #[arg(long)]
        commit: String,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long, default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
    },
}
