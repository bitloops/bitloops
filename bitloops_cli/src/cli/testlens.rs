use std::path::Path;

use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::capability_packs::test_harness::ingest::{coverage_batch, results};
use crate::capability_packs::test_harness::query as query_test_harness;
use crate::capability_packs::test_harness::storage as test_harness_engine;
use crate::capability_packs::test_harness::types::{
    TEST_HARNESS_COVERAGE_INGESTER_ID, TEST_HARNESS_LINKAGE_INGESTER_ID,
};
use crate::cli::testlens_types::{DEFAULT_QUERY_VIEW, QueryViewArg};
use crate::host::devql::capability_host::DevqlCapabilityHost;
use crate::host::devql::resolve_repo_identity;
use crate::models::{CoverageFormat, ScopeKind};
use crate::utils::paths;

const MISSING_SUBCOMMAND_MESSAGE: &str = "missing subcommand. Use one of: `bitloops testlens init`, `bitloops testlens ingest-tests`, `bitloops testlens ingest-coverage`, `bitloops testlens ingest-coverage-batch`, `bitloops testlens ingest-results`, `bitloops testlens query`, `bitloops testlens list`";

#[derive(Args, Debug, Clone, Default)]
pub struct TestLensArgs {
    #[command(subcommand)]
    pub command: Option<TestLensCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum TestLensCommand {
    /// Ensure test-harness schema exists in the configured relational store.
    Init(TestLensInitArgs),
    /// Parse test files, discover suites/scenarios, and link tests to production artefacts.
    IngestTests(TestLensIngestTestsArgs),
    /// Ingest coverage report (LCOV or LLVM JSON).
    IngestCoverage(TestLensIngestCoverageArgs),
    /// Batch-ingest coverage from a JSON manifest.
    IngestCoverageBatch(TestLensIngestCoverageBatchArgs),
    /// Ingest Jest JSON test results.
    IngestResults(TestLensIngestResultsArgs),
    /// Query the test harness for an artefact.
    Query(TestLensQueryArgs),
    /// List known artefacts.
    List(TestLensListArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct TestLensInitArgs {}

#[derive(Args, Debug, Clone)]
pub struct TestLensIngestTestsArgs {
    #[arg(long)]
    pub commit: String,
}

#[derive(Args, Debug, Clone)]
pub struct TestLensIngestCoverageArgs {
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
pub struct TestLensIngestCoverageBatchArgs {
    #[arg(long)]
    pub manifest: std::path::PathBuf,
    #[arg(long)]
    pub commit: String,
}

#[derive(Args, Debug, Clone)]
pub struct TestLensIngestResultsArgs {
    #[arg(long)]
    pub jest_json: std::path::PathBuf,
    #[arg(long)]
    pub commit: String,
}

#[derive(Args, Debug, Clone)]
pub struct TestLensQueryArgs {
    #[arg(long)]
    pub artefact: String,
    #[arg(long)]
    pub commit: String,
    #[arg(long)]
    pub classification: Option<String>,
    #[arg(long, value_enum, default_value_t = DEFAULT_QUERY_VIEW)]
    pub view: QueryViewArg,
    #[arg(long)]
    pub min_strength: Option<f64>,
}

#[derive(Args, Debug, Clone)]
pub struct TestLensListArgs {
    #[arg(long)]
    pub commit: String,
    #[arg(long)]
    pub kind: Option<String>,
}

pub async fn run(args: TestLensArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(MISSING_SUBCOMMAND_MESSAGE);
    };

    let repo_root = paths::repo_root()?;

    match command {
        TestLensCommand::Init(_) => test_harness_engine::init_schema_for_repo(&repo_root),
        TestLensCommand::IngestTests(args) => run_ingest_tests(&repo_root, &args).await,
        TestLensCommand::IngestCoverage(args) => run_ingest_coverage(&repo_root, &args).await,
        TestLensCommand::IngestCoverageBatch(args) => {
            run_ingest_coverage_batch(&repo_root, &args).await
        }
        TestLensCommand::IngestResults(args) => run_ingest_results(&repo_root, &args),
        TestLensCommand::Query(args) => run_query(&repo_root, &args),
        TestLensCommand::List(args) => run_list(&repo_root, &args),
    }
}

async fn run_ingest_tests(repo_root: &Path, args: &TestLensIngestTestsArgs) -> Result<()> {
    let repo = resolve_repo_identity(repo_root)?;
    let mut host = DevqlCapabilityHost::builtin(repo_root.to_path_buf(), repo)?;
    host.ensure_migrations_applied_sync()?;

    let payload = serde_json::json!({ "commit_sha": args.commit });
    let result = host
        .invoke_ingester("test_harness", TEST_HARNESS_LINKAGE_INGESTER_ID, payload)
        .await?;
    println!("{}", result.render_human());
    Ok(())
}

async fn run_ingest_coverage(repo_root: &Path, args: &TestLensIngestCoverageArgs) -> Result<()> {
    let scope_kind = args.scope.parse::<ScopeKind>().map_err(|_| {
        anyhow::anyhow!(
            "invalid scope: {} (expected workspace, package, test-scenario, or doctest)",
            args.scope
        )
    })?;
    let coverage_path = args
        .lcov
        .as_deref()
        .or(args.input.as_deref())
        .ok_or_else(|| anyhow::anyhow!("either --lcov or --input must be provided"))?;
    let format = resolve_format(args.format.as_deref(), coverage_path)?;

    if scope_kind == ScopeKind::TestScenario {
        if args.test_artefact_id.is_none() {
            bail!("--test-artefact-id is required when scope is test-scenario");
        }
        if format == CoverageFormat::Lcov {
            bail!(
                "LCOV format is not supported for scope=test-scenario (too lossy for per-test attribution); use --format llvm-json"
            );
        }
    }

    let repo = resolve_repo_identity(repo_root)?;
    let mut host = DevqlCapabilityHost::builtin(repo_root.to_path_buf(), repo)?;
    host.ensure_migrations_applied_sync()?;

    let payload = serde_json::json!({
        "coverage_path": coverage_path.to_string_lossy(),
        "commit_sha": args.commit,
        "scope_kind": args.scope,
        "tool": args.tool,
        "test_artefact_id": args.test_artefact_id,
        "format": format.as_str(),
    });

    let result = host
        .invoke_ingester("test_harness", TEST_HARNESS_COVERAGE_INGESTER_ID, payload)
        .await?;
    println!("{}", result.render_human());
    Ok(())
}

async fn run_ingest_coverage_batch(
    repo_root: &Path,
    args: &TestLensIngestCoverageBatchArgs,
) -> Result<()> {
    let entries = coverage_batch::parse_manifest_entries(&args.manifest)?;
    let manifest_dir = args
        .manifest
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));

    let repo = resolve_repo_identity(repo_root)?;
    let mut host = DevqlCapabilityHost::builtin(repo_root.to_path_buf(), repo)?;
    host.ensure_migrations_applied_sync()?;

    for (index, entry) in entries.iter().enumerate() {
        let coverage_path = manifest_dir.join(&entry.path);
        if !coverage_path.exists() {
            anyhow::bail!(
                "manifest entry {} references non-existent file: {}",
                index,
                coverage_path.display()
            );
        }

        entry.scope.parse::<ScopeKind>().map_err(|_| {
            anyhow::anyhow!(
                "invalid scope: {} (expected workspace, package, test-scenario, or doctest)",
                entry.scope
            )
        })?;
        let format = entry.format.parse::<CoverageFormat>().map_err(|_| {
            anyhow::anyhow!(
                "unknown format: {} (expected lcov or llvm-json)",
                entry.format
            )
        })?;

        let payload = serde_json::json!({
            "coverage_path": coverage_path.to_string_lossy(),
            "commit_sha": args.commit,
            "scope_kind": entry.scope,
            "tool": entry.tool,
            "test_artefact_id": entry.test_artefact_id,
            "format": format.as_str(),
        });

        host.invoke_ingester("test_harness", TEST_HARNESS_COVERAGE_INGESTER_ID, payload)
            .await?;
    }

    println!(
        "batch ingested {} coverage entries for commit {}",
        entries.len(),
        args.commit
    );
    Ok(())
}

fn run_ingest_results(repo_root: &Path, args: &TestLensIngestResultsArgs) -> Result<()> {
    let mut repository = test_harness_engine::open_repository_for_repo(repo_root)?;
    let summary = results::execute(&mut repository, &args.jest_json, &args.commit)?;
    results::print_summary(&args.commit, &summary);
    Ok(())
}

fn run_query(repo_root: &Path, args: &TestLensQueryArgs) -> Result<()> {
    let repository = test_harness_engine::open_repository_for_repo(repo_root)?;
    let json = query_test_harness::render_query_artefact_harness(
        &repository,
        &args.artefact,
        &args.commit,
        args.classification.as_deref(),
        args.view,
        args.min_strength,
    )?;
    println!("{json}");
    Ok(())
}

fn run_list(repo_root: &Path, args: &TestLensListArgs) -> Result<()> {
    let repository = test_harness_engine::open_repository_for_repo(repo_root)?;
    let json =
        query_test_harness::render_list_artefacts(&repository, &args.commit, args.kind.as_deref())?;
    println!("{json}");
    Ok(())
}

fn resolve_format(format_str: Option<&str>, path: &Path) -> Result<CoverageFormat> {
    if let Some(fmt) = format_str {
        return fmt
            .parse::<CoverageFormat>()
            .map_err(|_| anyhow::anyhow!("unknown format: {fmt} (expected lcov or llvm-json)"));
    }

    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "json" => Ok(CoverageFormat::LlvmJson),
        _ => Ok(CoverageFormat::Lcov),
    }
}
