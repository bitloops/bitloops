use clap::{Args, Subcommand, ValueEnum};

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlArgs {
    #[command(subcommand)]
    pub command: Option<DevqlCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlCommand {
    /// Create schema for configured relational/events backends.
    Init(DevqlInitArgs),
    /// Run read-only SQL against the daemon-wide analytics query layer.
    Analytics(DevqlAnalyticsArgs),
    /// Manage daemon-owned DevQL sync and ingest tasks.
    Tasks(DevqlTasksArgs),
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
    /// Manage repository-scoped architecture metadata and role taxonomy proposals.
    Architecture(DevqlArchitectureArgs),
    /// Manage repository-scoped external knowledge.
    Knowledge(DevqlKnowledgeArgs),
    /// Inspect and rebaseline codebase navigation context views.
    NavigationContext(DevqlNavigationContextArgs),
    /// Test harness ingestion for DevQL production artefacts.
    TestHarness(DevqlTestHarnessArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlInitArgs {}

#[derive(Args, Debug, Clone)]
pub struct DevqlAnalyticsArgs {
    #[command(subcommand)]
    pub command: DevqlAnalyticsCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlAnalyticsCommand {
    /// Execute a read-only SQL query over analytics.* and analytics_raw.* views.
    Sql(DevqlAnalyticsSqlArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DevqlAnalyticsSqlArgs {
    /// SQL query to execute.
    pub query: String,

    /// Select an explicit repository by repo id, identity, or unique name.
    #[arg(long = "repo", conflicts_with = "all_repos")]
    pub repos: Vec<String>,

    /// Query all known repositories in the current daemon scope.
    #[arg(long = "all-repos", default_value_t = false)]
    pub all_repos: bool,

    /// Emit JSON instead of the default tabular output.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlTasksArgs {
    #[command(subcommand)]
    pub command: DevqlTasksCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlTasksCommand {
    /// Enqueue a sync or ingest task.
    Enqueue(DevqlTaskEnqueueArgs),
    /// Watch a task until it reaches a terminal state.
    Watch(DevqlTaskWatchArgs),
    /// Show the current repository task queue status.
    Status(DevqlTaskStatusArgs),
    /// List recent tasks for the current repository.
    List(DevqlTaskListArgs),
    /// Pause scheduling for the current repository task queue.
    Pause(DevqlTaskPauseArgs),
    /// Resume scheduling for the current repository task queue.
    Resume(DevqlTaskResumeArgs),
    /// Cancel a queued task.
    Cancel(DevqlTaskCancelArgs),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum DevqlTaskKindArg {
    Sync,
    Ingest,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum DevqlTaskStatusArg {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlTaskEnqueueArgs {
    /// Which kind of task to enqueue.
    #[arg(long, value_enum)]
    pub kind: DevqlTaskKindArg,

    /// Run a full workspace reconciliation for sync tasks.
    #[arg(long, conflicts_with_all = ["paths", "repair", "validate"])]
    pub full: bool,

    /// Reconcile only the specified workspace paths for sync tasks.
    #[arg(long, value_delimiter = ',', conflicts_with_all = ["full", "repair", "validate"])]
    pub paths: Option<Vec<String>>,

    /// Rebuild sync state from the current workspace and repair stored state.
    #[arg(long, conflicts_with_all = ["full", "paths", "validate"])]
    pub repair: bool,

    /// Validate current-state tables against a read-only full reconciliation.
    #[arg(long, conflicts_with_all = ["full", "paths", "repair"])]
    pub validate: bool,

    /// Limit ingest to the most recent N commits.
    #[arg(long)]
    pub backfill: Option<usize>,

    /// Follow the queued task until it reaches a terminal state.
    #[arg(long, default_value_t = false)]
    pub status: bool,

    /// Fail immediately if the daemon is not already running.
    #[arg(long, default_value_t = false)]
    pub require_daemon: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlTaskWatchArgs {
    pub task_id: String,

    /// Fail immediately if the daemon is not already running.
    #[arg(long, default_value_t = false)]
    pub require_daemon: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlTaskStatusArgs {}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlTaskListArgs {
    #[arg(long, value_enum)]
    pub kind: Option<DevqlTaskKindArg>,

    #[arg(long, value_enum)]
    pub status: Option<DevqlTaskStatusArg>,

    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlTaskPauseArgs {
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlTaskResumeArgs {}

#[derive(Args, Debug, Clone)]
pub struct DevqlTaskCancelArgs {
    pub task_id: String,
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
pub struct DevqlArchitectureArgs {
    #[command(subcommand)]
    pub command: DevqlArchitectureCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlArchitectureCommand {
    /// Manage architecture role taxonomy, rules, aliases, and proposals.
    Roles(DevqlArchitectureRolesArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesArgs {
    #[command(subcommand)]
    pub command: DevqlArchitectureRolesCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlArchitectureRolesCommand {
    /// Seed a repository-specific architecture role taxonomy.
    Seed(DevqlArchitectureRolesSeedArgs),
    /// Seed roles, activate seed-owned rules, and run full classification.
    Bootstrap(DevqlArchitectureRolesBootstrapArgs),
    /// Re-run architecture role classification from current canonical state.
    Classify(DevqlArchitectureRolesClassifyArgs),
    /// Show ambiguous role adjudication queue and needs-review assignments.
    Status(DevqlArchitectureRolesStatusArgs),
    /// Rename a role's display name.
    Rename(DevqlArchitectureRolesRenameArgs),
    /// Deprecate a role, optionally pointing at a replacement role.
    Deprecate(DevqlArchitectureRolesDeprecateArgs),
    /// Remove a role, optionally migrating assignments to a replacement role.
    Remove(DevqlArchitectureRolesRemoveArgs),
    /// Merge one role into another role.
    Merge(DevqlArchitectureRolesMergeArgs),
    /// Split one role using a spec file.
    Split(DevqlArchitectureRolesSplitArgs),
    /// Manage role aliases.
    Alias(DevqlArchitectureRolesAliasArgs),
    /// Manage architecture role detection rules.
    Rules(DevqlArchitectureRolesRulesArgs),
    /// Inspect and apply pending role change proposals.
    Proposal(DevqlArchitectureRolesProposalArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevqlArchitectureRolesSeedArgs {
    /// Activate seed-owned draft rules after seeding.
    #[arg(long = "activate-rules", default_value_t = false)]
    pub activate_rules: bool,

    /// Run a full deterministic classification after activating seed-owned rules.
    #[arg(long, default_value_t = false)]
    pub classify: bool,

    /// Enqueue ambiguous/high-impact classification results for asynchronous LLM adjudication.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub enqueue_adjudication: bool,

    /// Emit JSON instead of human-readable text.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesBootstrapArgs {
    /// Enqueue ambiguous/high-impact classification results for asynchronous LLM adjudication.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub enqueue_adjudication: bool,

    /// Emit JSON instead of human-readable text.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesClassifyArgs {
    /// Reclassify all current files and artefacts.
    #[arg(long, conflicts_with = "paths")]
    pub full: bool,

    /// Reclassify selected paths from current canonical state.
    #[arg(long, value_delimiter = ',', conflicts_with = "full")]
    pub paths: Option<Vec<String>>,

    /// Mark active role assignments whose paths no longer exist in current canonical state as stale.
    #[arg(long, default_value_t = false)]
    pub repair_stale: bool,

    /// Enqueue ambiguous/high-impact results for asynchronous LLM adjudication.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub enqueue_adjudication: bool,

    /// Emit JSON instead of human-readable text.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesStatusArgs {
    /// Limit returned queue and review items.
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// Emit JSON instead of human-readable text.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesRenameArgs {
    pub role_ref: String,

    #[arg(long = "display-name")]
    pub display_name: String,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesDeprecateArgs {
    pub role_ref: String,

    #[arg(long)]
    pub replacement: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesRemoveArgs {
    pub role_ref: String,

    #[arg(long)]
    pub replacement: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesMergeArgs {
    pub source_role_ref: String,

    #[arg(long = "into")]
    pub target_role_ref: String,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesSplitArgs {
    pub role_ref: String,

    #[arg(long)]
    pub spec: std::path::PathBuf,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesAliasArgs {
    #[command(subcommand)]
    pub command: DevqlArchitectureRolesAliasCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlArchitectureRolesAliasCommand {
    /// Create a role alias that resolves to an existing role.
    Create(DevqlArchitectureRolesAliasCreateArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesAliasCreateArgs {
    pub alias_key: String,

    #[arg(long = "role")]
    pub role_ref: String,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesRulesArgs {
    #[command(subcommand)]
    pub command: DevqlArchitectureRolesRulesCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlArchitectureRolesRulesCommand {
    /// Draft a new rule from a spec file.
    Draft(DevqlArchitectureRolesRulesDraftArgs),
    /// Edit an existing rule from a spec file.
    Edit(DevqlArchitectureRolesRulesEditArgs),
    /// Activate a rule.
    Activate(DevqlArchitectureRolesRulesRefArgs),
    /// Disable a rule.
    Disable(DevqlArchitectureRolesRulesRefArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesRulesDraftArgs {
    #[arg(long)]
    pub spec: std::path::PathBuf,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesRulesEditArgs {
    pub rule_ref: String,

    #[arg(long)]
    pub spec: std::path::PathBuf,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesRulesRefArgs {
    pub rule_ref: String,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesProposalArgs {
    #[command(subcommand)]
    pub command: DevqlArchitectureRolesProposalCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlArchitectureRolesProposalCommand {
    /// Show a pending proposal.
    Show(DevqlArchitectureRolesProposalRefArgs),
    /// Apply a pending proposal.
    Apply(DevqlArchitectureRolesProposalRefArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DevqlArchitectureRolesProposalRefArgs {
    pub proposal_id: String,
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
pub struct DevqlNavigationContextArgs {
    #[command(subcommand)]
    pub command: DevqlNavigationContextCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlNavigationContextCommand {
    /// List navigation context views and stale dependency changes.
    Status(DevqlNavigationContextStatusArgs),
    /// Materialise a navigation context view snapshot into the system of record.
    Materialise(DevqlNavigationContextMaterialiseArgs),
    /// Accept a navigation context view's current signature as the new baseline.
    Accept(DevqlNavigationContextAcceptArgs),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum DevqlNavigationContextStatusArg {
    Fresh,
    Stale,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlNavigationContextStatusArgs {
    /// Project path to inspect.
    #[arg(long, default_value = ".")]
    pub project: String,

    /// Limit output to one view id.
    #[arg(long)]
    pub view: Option<String>,

    /// Limit output to fresh or stale views.
    #[arg(long, value_enum)]
    pub status: Option<DevqlNavigationContextStatusArg>,

    /// Emit JSON instead of human-readable text.
    #[arg(long, default_value_t = false)]
    pub json: bool,

    /// Maximum changed primitives to show per stale view in text mode.
    #[arg(long = "changed-limit", default_value_t = 10)]
    pub changed_limit: usize,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlNavigationContextAcceptArgs {
    /// View id to accept.
    pub view_id: String,

    /// Reject acceptance if the current signature no longer matches this value.
    #[arg(long)]
    pub expected_current_signature: Option<String>,

    /// Human-readable reason to store with the acceptance.
    #[arg(long)]
    pub reason: Option<String>,

    /// Materialised artefact reference reviewed for this baseline.
    #[arg(long = "materialised-ref")]
    pub materialised_ref: Option<String>,

    /// Emit JSON instead of human-readable text.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlNavigationContextMaterialiseArgs {
    /// View id to materialise.
    pub view_id: String,

    /// Reject materialisation if the current signature no longer matches this value.
    #[arg(long)]
    pub expected_current_signature: Option<String>,

    /// Emit JSON instead of human-readable text.
    #[arg(long, default_value_t = false)]
    pub json: bool,

    /// Print the rendered materialisation text instead of a compact summary.
    #[arg(long = "rendered", default_value_t = false)]
    pub rendered: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DevqlTestHarnessArgs {
    #[command(subcommand)]
    pub command: DevqlTestHarnessCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevqlTestHarnessCommand {
    /// Legacy commit-scoped test discovery/linkage ingestion.
    ///
    /// Prefer automatic current-state sync for workspace validation.
    /// Keep this command for historical or commit-scoped materialization.
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
    /// Commit SHA to materialize into the historical test-harness tables.
    ///
    /// This legacy command is commit-scoped. Prefer automatic current-state sync
    /// for workspace validation and source-level linkage queries.
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
