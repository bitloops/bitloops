use clap::{Args, Subcommand};

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
    /// Synchronize current workspace artefacts into DevQL state.
    Sync(DevqlSyncArgs),
    /// Backfill or repair DevQL relational projections.
    Projection(DevqlProjectionArgs),
    /// Print the DevQL GraphQL schema SDL.
    Schema(DevqlSchemaArgs),
    /// Execute a DevQL query.
    Query(DevqlQueryArgs),
    /// Check backend connectivity for Postgres and ClickHouse.
    ConnectionStatus(DevqlConnectionStatusArgs),
    /// List registered capability packs, migrations, and host policy (optional health checks).
    Packs(DevqlPacksArgs),
    /// Manage repository-scoped external knowledge.
    Knowledge(DevqlKnowledgeArgs),
    /// Test harness ingestion for DevQL production artefacts.
    TestHarness(DevqlTestHarnessArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlInitArgs {}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlIngestArgs {}

#[derive(Debug, Clone, clap::Args)]
pub struct DevqlSyncArgs {
    /// Run a full workspace reconciliation.
    #[arg(long, conflicts_with_all = ["paths", "repair", "validate"])]
    pub full: bool,

    /// Reconcile only the specified workspace paths.
    #[arg(long, value_delimiter = ',', conflicts_with_all = ["full", "repair", "validate"])]
    pub paths: Option<Vec<String>>,

    /// Rebuild sync state from the current workspace and repair stored state.
    #[arg(long, conflicts_with_all = ["full", "paths", "validate"])]
    pub repair: bool,

    /// Validate current-state tables against a full read-only workspace reconciliation.
    #[arg(long, conflicts_with_all = ["full", "paths", "repair"])]
    pub validate: bool,

    /// Follow the queued sync task until it reaches a terminal state.
    #[arg(long, default_value_t = false)]
    pub status: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlProjectionArgs {
    #[command(subcommand)]
    pub command: DevqlProjectionCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlProjectionCommand {
    /// Backfill or repair the checkpoint_file_snapshots projection.
    CheckpointFileSnapshots(DevqlCheckpointFileSnapshotsArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DevqlCheckpointFileSnapshotsArgs {
    /// Apply writes in bounded checkpoint batches.
    #[arg(long, default_value_t = 200)]
    pub batch_size: usize,

    /// Stop after this many checkpoints (after any resume filter).
    #[arg(long)]
    pub max_checkpoints: Option<usize>,

    /// Resume after the specified checkpoint_id in the stored checkpoint order.
    #[arg(long)]
    pub resume_after: Option<String>,

    /// Report counters without mutating checkpoint_file_snapshots.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlSchemaArgs {
    /// Print the full/global DevQL GraphQL schema.
    #[arg(long = "global", default_value_t = false)]
    pub global: bool,

    /// Print human-readable formatted SDL instead of minified SDL.
    #[arg(long, default_value_t = false)]
    pub human: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlQueryArgs {
    /// Force the input to be treated as a raw GraphQL document.
    #[arg(long, default_value_t = false)]
    pub graphql: bool,

    /// GraphQL document or DevQL DSL pipeline.
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

#[derive(Args, Debug, Clone)]
pub struct DevqlTestHarnessArgs {
    #[command(subcommand)]
    pub command: DevqlTestHarnessCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlTestHarnessCommand {
    /// Parse test files, discover suites/scenarios, and link tests to production artefacts.
    IngestTests(DevqlTestHarnessIngestTestsArgs),
    /// Ingest coverage report (LCOV or LLVM JSON).
    IngestCoverage(DevqlTestHarnessIngestCoverageArgs),
    /// Batch-ingest coverage from a JSON manifest.
    IngestCoverageBatch(DevqlTestHarnessIngestCoverageBatchArgs),
    /// Ingest Jest JSON test results.
    IngestResults(DevqlTestHarnessIngestResultsArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DevqlTestHarnessIngestTestsArgs {
    #[arg(long)]
    pub commit: String,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlTestHarnessIngestCoverageArgs {
    #[arg(long)]
    pub lcov: Option<std::path::PathBuf>,
    #[arg(long)]
    pub input: Option<std::path::PathBuf>,
    #[arg(long)]
    pub commit: String,
    #[arg(long)]
    pub scope: String,
    #[arg(long, default_value = "unknown")]
    pub tool: String,
    #[arg(long)]
    pub test_artefact_id: Option<String>,
    #[arg(long)]
    pub format: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlTestHarnessIngestCoverageBatchArgs {
    #[arg(long)]
    pub manifest: std::path::PathBuf,
    #[arg(long)]
    pub commit: String,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlTestHarnessIngestResultsArgs {
    #[arg(long)]
    pub jest_json: std::path::PathBuf,
    #[arg(long)]
    pub commit: String,
}
