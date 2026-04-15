use anyhow::Result;
use std::future::Future;
use std::path::PathBuf;

use super::bundle::{BundleResult, combine_bundle_results};
use super::runner::{self, Suite};

pub(crate) fn resolve_binary() -> PathBuf {
    if let Ok(path_raw) = std::env::var("BITLOOPS_QAT_BINARY") {
        let path = PathBuf::from(path_raw);
        assert!(
            path.exists(),
            "BITLOOPS_QAT_BINARY points to {}, which does not exist",
            path.display()
        );
        return path;
    }

    PathBuf::from(env!("CARGO_BIN_EXE_bitloops"))
}

pub(crate) async fn run_suite_entrypoint(suite: Suite) -> Result<()> {
    let binary = resolve_binary();
    runner::run_suite(binary, suite).await
}

pub(crate) async fn run_bundle_entrypoint() -> Result<()> {
    let binary = resolve_binary();
    run_bundle_with_runner(binary, runner::run_suite).await
}

pub(crate) async fn run_bundle_with_runner<Runner, SuiteFuture>(
    binary: PathBuf,
    runner: Runner,
) -> Result<()>
where
    Runner: Fn(PathBuf, Suite) -> SuiteFuture,
    SuiteFuture: Future<Output = Result<()>>,
{
    let (onboarding, smoke, devql_sync, devql_capabilities, devql_ingest) = tokio::join!(
        runner(binary.clone(), Suite::Onboarding),
        runner(binary.clone(), Suite::Smoke),
        runner(binary.clone(), Suite::DevqlSync),
        runner(binary.clone(), Suite::Devql),
        runner(binary, Suite::DevqlIngest),
    );

    combine_bundle_results(vec![
        BundleResult::from_result("onboarding", onboarding),
        BundleResult::from_result("smoke", smoke),
        BundleResult::from_result("devql-sync", devql_sync),
        BundleResult::from_result("devql-capabilities", devql_capabilities),
        BundleResult::from_result("devql-ingest", devql_ingest),
    ])
}

pub(crate) async fn run_bundle_from_futures<
    OnboardingFuture,
    SmokeFuture,
    DevqlSyncFuture,
    DevqlCapabilitiesFuture,
    DevqlIngestFuture,
>(
    onboarding: OnboardingFuture,
    smoke: SmokeFuture,
    devql_sync: DevqlSyncFuture,
    devql_capabilities: DevqlCapabilitiesFuture,
    devql_ingest: DevqlIngestFuture,
) -> Result<()>
where
    OnboardingFuture: Future<Output = Result<()>>,
    SmokeFuture: Future<Output = Result<()>>,
    DevqlSyncFuture: Future<Output = Result<()>>,
    DevqlCapabilitiesFuture: Future<Output = Result<()>>,
    DevqlIngestFuture: Future<Output = Result<()>>,
{
    let (onboarding, smoke, devql_sync, devql_capabilities, devql_ingest) = tokio::join!(
        onboarding,
        smoke,
        devql_sync,
        devql_capabilities,
        devql_ingest,
    );

    combine_bundle_results(vec![
        BundleResult::from_result("onboarding", onboarding),
        BundleResult::from_result("smoke", smoke),
        BundleResult::from_result("devql-sync", devql_sync),
        BundleResult::from_result("devql-capabilities", devql_capabilities),
        BundleResult::from_result("devql-ingest", devql_ingest),
    ])
}
