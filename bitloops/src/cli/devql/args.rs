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

#[derive(Debug, Clone, clap::Args)]
pub struct DevqlSyncArgs {
    /// Run a full workspace reconciliation.
    #[arg(long, conflicts_with_all = ["paths", "repair"])]
    pub full: bool,

    /// Reconcile only the specified workspace paths.
    #[arg(long, value_delimiter = ',', conflicts_with_all = ["full", "repair"])]
    pub paths: Option<Vec<String>>,

    /// Rebuild sync state from the current workspace and repair stored state.
    #[arg(long, conflicts_with_all = ["full", "paths"])]
    pub repair: bool,
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
