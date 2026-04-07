use std::path::Path;

use anyhow::{Result, bail};

use crate::capability_packs::test_harness::ingest::{coverage_batch, results};
use crate::capability_packs::test_harness::storage as test_harness_engine;
use crate::capability_packs::test_harness::types::{
    TEST_HARNESS_COVERAGE_INGESTER_ID, TEST_HARNESS_LINKAGE_INGESTER_ID,
};
use crate::host::capability_host::DevqlCapabilityHost;
use crate::host::devql::resolve_repo_identity;
use crate::host::relational_store::DefaultRelationalStore;
use crate::models::{CoverageFormat, ScopeKind};

use super::args::{
    DevqlTestHarnessArgs, DevqlTestHarnessCommand, DevqlTestHarnessIngestCoverageArgs,
    DevqlTestHarnessIngestCoverageBatchArgs, DevqlTestHarnessIngestResultsArgs,
    DevqlTestHarnessIngestTestsArgs,
};

pub(super) async fn run(args: DevqlTestHarnessArgs, repo_root: &Path) -> Result<()> {
    match args.command {
        DevqlTestHarnessCommand::IngestTests(args) => run_ingest_tests(repo_root, &args).await,
        DevqlTestHarnessCommand::IngestCoverage(args) => {
            run_ingest_coverage(repo_root, &args).await
        }
        DevqlTestHarnessCommand::IngestCoverageBatch(args) => {
            run_ingest_coverage_batch(repo_root, &args).await
        }
        DevqlTestHarnessCommand::IngestResults(args) => run_ingest_results(repo_root, &args),
    }
}

async fn run_ingest_tests(repo_root: &Path, args: &DevqlTestHarnessIngestTestsArgs) -> Result<()> {
    let repo = resolve_repo_identity(repo_root)?;
    let host = DevqlCapabilityHost::builtin(repo_root.to_path_buf(), repo)?;
    host.ensure_migrations_applied_sync()?;

    let payload = serde_json::json!({ "commit_sha": args.commit });
    let result = host
        .invoke_ingester("test_harness", TEST_HARNESS_LINKAGE_INGESTER_ID, payload)
        .await?;
    println!("{}", result.render_human());
    Ok(())
}

async fn run_ingest_coverage(
    repo_root: &Path,
    args: &DevqlTestHarnessIngestCoverageArgs,
) -> Result<()> {
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
    let host = DevqlCapabilityHost::builtin(repo_root.to_path_buf(), repo)?;
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
    args: &DevqlTestHarnessIngestCoverageBatchArgs,
) -> Result<()> {
    let entries = coverage_batch::parse_manifest_entries(&args.manifest)?;
    let manifest_dir = args
        .manifest
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));

    let repo = resolve_repo_identity(repo_root)?;
    let host = DevqlCapabilityHost::builtin(repo_root.to_path_buf(), repo)?;
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

fn run_ingest_results(repo_root: &Path, args: &DevqlTestHarnessIngestResultsArgs) -> Result<()> {
    let relational_store = DefaultRelationalStore::open_local_for_repo_root(repo_root)?;
    let pool = relational_store.local_sqlite_pool_allow_create()?;
    let relational = crate::host::capability_host::gateways::SqliteRelationalGateway::new(pool);
    let mut repository = test_harness_engine::open_repository_for_repo(repo_root)?;
    let summary = results::execute(&mut repository, &relational, &args.jest_json, &args.commit)?;
    results::print_summary(&args.commit, &summary);
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
