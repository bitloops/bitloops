use anyhow::Result;
use std::future::Future;
use std::path::PathBuf;

use super::bundle::{BundleResult, combine_bundle_results};
use super::runner::{self, Suite};
use super::subsets::{DEVELOP_GATE_RERUN_ALIAS, DEVELOP_GATE_SUITES, DEVELOP_GATE_TAG_EXPR};

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

pub(crate) async fn run_develop_gate_entrypoint() -> Result<()> {
    let binary = resolve_binary();
    run_serial_suites_with_runner(
        binary,
        DEVELOP_GATE_SUITES,
        Some(DEVELOP_GATE_TAG_EXPR),
        DEVELOP_GATE_RERUN_ALIAS,
        runner::run_suite_with_tags,
    )
    .await
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
    // Keep the lightweight onboarding/smoke suites parallel for bundle latency, but
    // serialize the DevQL-heavy suites because they are more likely to contend on
    // SQLite-backed materialization paths when the bundled run fans out fully.
    let (onboarding, smoke) = tokio::join!(
        runner(binary.clone(), Suite::Onboarding),
        runner(binary.clone(), Suite::AgentSmoke),
    );
    let devql_sync = runner(binary.clone(), Suite::DevqlSync).await;
    let devql_capabilities = runner(binary.clone(), Suite::Devql).await;
    let devql_ingest = runner(binary, Suite::DevqlIngest).await;

    combine_bundle_results(vec![
        BundleResult::from_result(
            Suite::Onboarding.id(),
            Suite::Onboarding.rerun_alias(),
            onboarding,
        ),
        BundleResult::from_result(
            Suite::AgentSmoke.id(),
            Suite::AgentSmoke.rerun_alias(),
            smoke,
        ),
        BundleResult::from_result(
            Suite::DevqlSync.id(),
            Suite::DevqlSync.rerun_alias(),
            devql_sync,
        ),
        BundleResult::from_result(
            Suite::Devql.id(),
            Suite::Devql.rerun_alias(),
            devql_capabilities,
        ),
        BundleResult::from_result(
            Suite::DevqlIngest.id(),
            Suite::DevqlIngest.rerun_alias(),
            devql_ingest,
        ),
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
    let (onboarding, smoke) = tokio::join!(onboarding, smoke);
    let devql_sync = devql_sync.await;
    let devql_capabilities = devql_capabilities.await;
    let devql_ingest = devql_ingest.await;

    combine_bundle_results(vec![
        BundleResult::from_result(
            Suite::Onboarding.id(),
            Suite::Onboarding.rerun_alias(),
            onboarding,
        ),
        BundleResult::from_result(
            Suite::AgentSmoke.id(),
            Suite::AgentSmoke.rerun_alias(),
            smoke,
        ),
        BundleResult::from_result(
            Suite::DevqlSync.id(),
            Suite::DevqlSync.rerun_alias(),
            devql_sync,
        ),
        BundleResult::from_result(
            Suite::Devql.id(),
            Suite::Devql.rerun_alias(),
            devql_capabilities,
        ),
        BundleResult::from_result(
            Suite::DevqlIngest.id(),
            Suite::DevqlIngest.rerun_alias(),
            devql_ingest,
        ),
    ])
}

pub(crate) async fn run_serial_suites_with_runner<Runner, SuiteFuture>(
    binary: PathBuf,
    suites: &[Suite],
    tags_filter: Option<&'static str>,
    rerun_alias: &'static str,
    runner: Runner,
) -> Result<()>
where
    Runner: Fn(PathBuf, Suite, Option<&'static str>) -> SuiteFuture,
    SuiteFuture: Future<Output = Result<()>>,
{
    let mut results = Vec::with_capacity(suites.len());
    for suite in suites {
        let result = runner(binary.clone(), *suite, tags_filter).await;
        results.push(BundleResult::from_result(suite.id(), rerun_alias, result));
    }
    combine_bundle_results(results)
}
