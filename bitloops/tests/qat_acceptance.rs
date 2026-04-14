mod qat_support;

use anyhow::Result;
use std::future::Future;
use std::path::PathBuf;

use qat_support::bundle::{BundleResult, combine_bundle_results};
use qat_support::runner::{self, Suite};

fn resolve_binary() -> PathBuf {
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

#[tokio::test]
#[ignore = "slow E2E: runs all bundled QAT suites in parallel; use `cargo qat`"]
async fn qat() {
    let binary = resolve_binary();
    run_bundle(binary).await.expect("QAT bundle suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT smoke suite; use `cargo test --features qat-tests --test qat_acceptance qat_smoke -- --ignored` or `cargo qat`"]
async fn qat_smoke() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Smoke)
        .await
        .expect("QAT smoke suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT DevQL capabilities suite; use `cargo qat-devql-capabilities` or `cargo qat`"]
async fn qat_devql_capabilities() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Devql)
        .await
        .expect("QAT DevQL capabilities suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT DevQL ingest suite; use `cargo qat-devql-ingest` or `cargo qat`"]
async fn qat_devql_ingest() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::DevqlIngest)
        .await
        .expect("QAT DevQL ingest suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT DevQL sync suite; use `cargo qat-devql-sync` or `cargo qat`"]
async fn qat_devql_sync() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::DevqlSync)
        .await
        .expect("QAT DevQL sync suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT onboarding suite; use `cargo qat-onboarding` or `cargo qat`"]
async fn qat_onboarding() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Onboarding)
        .await
        .expect("QAT onboarding suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT quickstart suite; use `cargo test --features qat-tests --test qat_acceptance qat_quickstart -- --ignored`"]
async fn qat_quickstart() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Quickstart)
        .await
        .expect("QAT quickstart suite failed");
}

async fn run_bundle(binary: PathBuf) -> Result<()> {
    run_bundle_with_runner(binary, runner::run_suite).await
}

async fn run_bundle_with_runner<Runner, SuiteFuture>(binary: PathBuf, runner: Runner) -> Result<()>
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

async fn run_bundle_from_futures<
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

#[test]
fn combine_bundle_results_returns_ok_when_all_suites_pass() {
    combine_bundle_results(vec![
        BundleResult::ok("onboarding"),
        BundleResult::ok("smoke"),
        BundleResult::ok("devql-sync"),
        BundleResult::ok("devql-capabilities"),
        BundleResult::ok("devql-ingest"),
    ])
    .expect("all suites should pass");
}

#[test]
fn combine_bundle_results_returns_onboarding_error_when_only_onboarding_fails() {
    let err = combine_bundle_results(vec![
        BundleResult::err("onboarding", anyhow::anyhow!("onboarding failed")),
        BundleResult::ok("smoke"),
        BundleResult::ok("devql-sync"),
        BundleResult::ok("devql-capabilities"),
        BundleResult::ok("devql-ingest"),
    ])
    .expect_err("onboarding failure should surface");
    assert!(
        format!("{err:#}").contains("onboarding failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_returns_sync_error_when_only_sync_fails() {
    let err = combine_bundle_results(vec![
        BundleResult::ok("onboarding"),
        BundleResult::ok("smoke"),
        BundleResult::err("devql-sync", anyhow::anyhow!("sync failed")),
        BundleResult::ok("devql-capabilities"),
        BundleResult::ok("devql-ingest"),
    ])
    .expect_err("sync failure should surface");
    assert!(
        format!("{err:#}").contains("sync failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_returns_smoke_error_when_only_smoke_fails() {
    let err = combine_bundle_results(vec![
        BundleResult::ok("onboarding"),
        BundleResult::err("smoke", anyhow::anyhow!("smoke failed")),
        BundleResult::ok("devql-sync"),
        BundleResult::ok("devql-capabilities"),
        BundleResult::ok("devql-ingest"),
    ])
    .expect_err("smoke failure should surface");
    assert!(
        format!("{err:#}").contains("smoke failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_returns_devql_capabilities_error_when_only_capabilities_fail() {
    let err = combine_bundle_results(vec![
        BundleResult::ok("onboarding"),
        BundleResult::ok("smoke"),
        BundleResult::ok("devql-sync"),
        BundleResult::err("devql-capabilities", anyhow::anyhow!("devql failed")),
        BundleResult::ok("devql-ingest"),
    ])
    .expect_err("devql failure should surface");
    assert!(
        format!("{err:#}").contains("devql failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_reports_all_failures() {
    let err = combine_bundle_results(vec![
        BundleResult::err("onboarding", anyhow::anyhow!("onboarding failed")),
        BundleResult::err("smoke", anyhow::anyhow!("smoke failed")),
        BundleResult::err("devql-sync", anyhow::anyhow!("sync failed")),
        BundleResult::err("devql-capabilities", anyhow::anyhow!("devql failed")),
        BundleResult::err("devql-ingest", anyhow::anyhow!("ingest failed")),
    ])
    .expect_err("all failures should be preserved");
    let message = format!("{err:#}");
    assert!(
        message.contains("onboarding failed"),
        "combined error missing onboarding details: {message}"
    );
    assert!(
        message.contains("smoke failed"),
        "combined error missing smoke details: {message}"
    );
    assert!(
        message.contains("sync failed"),
        "combined error missing sync details: {message}"
    );
    assert!(
        message.contains("devql failed"),
        "combined error missing devql details: {message}"
    );
    assert!(
        message.contains("ingest failed"),
        "combined error missing ingest details: {message}"
    );
}

#[tokio::test]
async fn run_bundle_starts_all_suites_before_completion() {
    let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(6));
    let runner = {
        let started = std::sync::Arc::clone(&started);
        let notify = std::sync::Arc::clone(&notify);
        let barrier = std::sync::Arc::clone(&barrier);
        move |_binary: PathBuf, suite: Suite| {
            let started = std::sync::Arc::clone(&started);
            let notify = std::sync::Arc::clone(&notify);
            let barrier = std::sync::Arc::clone(&barrier);
            async move {
                match suite {
                    Suite::Onboarding
                    | Suite::Smoke
                    | Suite::DevqlSync
                    | Suite::Devql
                    | Suite::DevqlIngest => {
                        started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        notify.notify_waiters();
                        barrier.wait().await;
                    }
                    _ => panic!("unexpected suite in bundle test"),
                }
                Ok(())
            }
        }
    };

    let bundle = tokio::spawn(run_bundle_with_runner(PathBuf::from("bitloops"), runner));

    loop {
        if started.load(std::sync::atomic::Ordering::SeqCst) == 5 {
            break;
        }
        notify.notified().await;
    }

    assert_eq!(
        started.load(std::sync::atomic::Ordering::SeqCst),
        5,
        "expected all bundled suites to start before the bundle completes"
    );

    barrier.wait().await;

    bundle
        .await
        .expect("bundle task should join")
        .expect("bundle should succeed");
}

#[tokio::test]
async fn run_bundle_from_futures_runs_all_suites_in_parallel() {
    use std::sync::Arc;
    use tokio::sync::{Barrier, Mutex, mpsc};
    use tokio::time::{Duration, timeout};

    let (tx, mut rx) = mpsc::unbounded_channel::<&'static str>();
    let barrier = Arc::new(Barrier::new(6));
    let completion_order = Arc::new(Mutex::new(Vec::new()));
    let make_suite = |name: &'static str| {
        let tx = tx.clone();
        let barrier = Arc::clone(&barrier);
        let completion_order = Arc::clone(&completion_order);
        async move {
            tx.send(name).expect("suite start should send");
            barrier.wait().await;
            completion_order.lock().await.push(name);
            Ok(())
        }
    };

    let bundle = tokio::spawn(run_bundle_from_futures(
        make_suite("onboarding"),
        make_suite("smoke"),
        make_suite("devql-sync"),
        make_suite("devql-capabilities"),
        make_suite("devql-ingest"),
    ));

    let mut observed = Vec::new();
    for _ in 0..5 {
        let suite = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("suite start should not time out")
            .expect("suite start should be received");
        observed.push(suite);
    }

    assert!(
        observed.contains(&"onboarding"),
        "bundle should start onboarding in the parallel fan-out, observed {observed:?}"
    );
    assert!(
        observed.contains(&"smoke"),
        "bundle should start smoke in the parallel fan-out, observed {observed:?}"
    );
    assert!(
        observed.contains(&"devql-sync"),
        "bundle should start devql-sync in the parallel fan-out, observed {observed:?}"
    );
    assert!(
        observed.contains(&"devql-capabilities"),
        "bundle should start devql-capabilities in the parallel fan-out, observed {observed:?}"
    );
    assert!(
        observed.contains(&"devql-ingest"),
        "bundle should start devql-ingest in the parallel fan-out, observed {observed:?}"
    );

    barrier.wait().await;

    bundle
        .await
        .expect("bundle task should join")
        .expect("bundle should succeed");

    let completion_order = completion_order.lock().await.clone();
    assert!(
        completion_order.contains(&"onboarding"),
        "bundle should complete onboarding: {completion_order:?}"
    );
    assert!(
        completion_order.contains(&"smoke"),
        "bundle should complete smoke: {completion_order:?}"
    );
    assert!(
        completion_order.contains(&"devql-sync"),
        "bundle should complete devql-sync: {completion_order:?}"
    );
    assert!(
        completion_order.contains(&"devql-capabilities"),
        "bundle should complete devql-capabilities: {completion_order:?}"
    );
    assert!(
        completion_order.contains(&"devql-ingest"),
        "bundle should complete devql-ingest: {completion_order:?}"
    );
}
