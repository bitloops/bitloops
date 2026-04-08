mod qat_support;

use anyhow::Result;
use std::future::Future;
use std::path::PathBuf;

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
#[ignore = "slow E2E: runs QAT onboarding + DevQL sync + smoke suites in parallel; use `cargo qat`"]
async fn qat() {
    let binary = resolve_binary();
    run_bundle(binary).await.expect("QAT bundle suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT smoke suite; use `cargo test --features qat-tests --test qat_acceptance qat_smoke -- --ignored`"]
async fn qat_smoke() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Smoke)
        .await
        .expect("QAT smoke suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT DevQL suite; use `cargo test --features qat-tests --test qat_acceptance qat_devql -- --ignored`"]
async fn qat_devql() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Devql)
        .await
        .expect("QAT DevQL suite failed");
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
    run_bundle_from_futures(
        runner(binary.clone(), Suite::Onboarding),
        runner(binary.clone(), Suite::DevqlSync),
        runner(binary, Suite::Smoke),
    )
    .await
}

async fn run_bundle_from_futures<OnboardingFuture, DevqlSyncFuture, SmokeFuture>(
    onboarding: OnboardingFuture,
    devql_sync: DevqlSyncFuture,
    smoke: SmokeFuture,
) -> Result<()>
where
    OnboardingFuture: Future<Output = Result<()>>,
    DevqlSyncFuture: Future<Output = Result<()>>,
    SmokeFuture: Future<Output = Result<()>>,
{
    let (onboarding, devql_sync, smoke) = tokio::join!(onboarding, devql_sync, smoke);
    combine_bundle_results(onboarding, devql_sync, smoke)
}

fn combine_bundle_results(
    onboarding: Result<()>,
    devql_sync: Result<()>,
    smoke: Result<()>,
) -> Result<()> {
    match (onboarding, devql_sync, smoke) {
        (Ok(()), Ok(()), Ok(())) => Ok(()),
        (Err(err), Ok(()), Ok(())) => Err(err),
        (Ok(()), Err(err), Ok(())) => Err(err),
        (Ok(()), Ok(()), Err(err)) => Err(err),
        (onboarding, devql_sync, smoke) => {
            let mut message = String::from("QAT bundle reported failures:");
            if let Err(err) = onboarding {
                message.push_str(&format!("\n- onboarding: {err:#}"));
            }
            if let Err(err) = devql_sync {
                message.push_str(&format!("\n- devql-sync: {err:#}"));
            }
            if let Err(err) = smoke {
                message.push_str(&format!("\n- smoke: {err:#}"));
            }
            Err(anyhow::anyhow!(message))
        }
    }
}

#[test]
fn combine_bundle_results_returns_ok_when_all_suites_pass() {
    combine_bundle_results(Ok(()), Ok(()), Ok(())).expect("all suites should pass");
}

#[test]
fn combine_bundle_results_returns_onboarding_error_when_only_onboarding_fails() {
    let err = combine_bundle_results(Err(anyhow::anyhow!("onboarding failed")), Ok(()), Ok(()))
        .expect_err("onboarding failure should surface");
    assert!(
        format!("{err:#}").contains("onboarding failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_returns_sync_error_when_only_sync_fails() {
    let err = combine_bundle_results(Ok(()), Err(anyhow::anyhow!("sync failed")), Ok(()))
        .expect_err("sync failure should surface");
    assert!(
        format!("{err:#}").contains("sync failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_returns_smoke_error_when_only_smoke_fails() {
    let err = combine_bundle_results(Ok(()), Ok(()), Err(anyhow::anyhow!("smoke failed")))
        .expect_err("smoke failure should surface");
    assert!(
        format!("{err:#}").contains("smoke failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_reports_all_failures() {
    let err = combine_bundle_results(
        Err(anyhow::anyhow!("onboarding failed")),
        Err(anyhow::anyhow!("sync failed")),
        Err(anyhow::anyhow!("smoke failed")),
    )
    .expect_err("all failures should be preserved");
    let message = format!("{err:#}");
    assert!(
        message.contains("onboarding failed"),
        "combined error missing onboarding details: {message}"
    );
    assert!(
        message.contains("sync failed"),
        "combined error missing sync details: {message}"
    );
    assert!(
        message.contains("smoke failed"),
        "combined error missing smoke details: {message}"
    );
}

#[tokio::test]
async fn run_bundle_launches_onboarding_devql_sync_and_smoke_together() {
    let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(4));
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
                    Suite::Onboarding | Suite::DevqlSync | Suite::Smoke => {}
                    _ => panic!("unexpected suite in bundle test"),
                }
                started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                notify.notify_waiters();
                barrier.wait().await;
                Ok(())
            }
        }
    };

    let bundle = tokio::spawn(run_bundle_with_runner(PathBuf::from("bitloops"), runner));

    loop {
        if started.load(std::sync::atomic::Ordering::SeqCst) == 3 {
            break;
        }
        notify.notified().await;
    }

    assert_eq!(
        started.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "expected onboarding, devql-sync, and smoke to all start before completion"
    );

    barrier.wait().await;

    bundle
        .await
        .expect("bundle task should join")
        .expect("bundle should succeed");
}

#[tokio::test]
async fn run_bundle_from_futures_starts_all_suites_before_release() {
    use std::sync::Arc;
    use tokio::sync::{Barrier, mpsc};
    use tokio::time::{Duration, timeout};

    let (tx, mut rx) = mpsc::unbounded_channel::<&'static str>();
    let barrier = Arc::new(Barrier::new(4));
    let make_suite = |name: &'static str| {
        let tx = tx.clone();
        let barrier = Arc::clone(&barrier);
        async move {
            tx.send(name).expect("suite start should send");
            barrier.wait().await;
            Ok(())
        }
    };

    let bundle = tokio::spawn(run_bundle_from_futures(
        make_suite("onboarding"),
        make_suite("devql-sync"),
        make_suite("smoke"),
    ));

    let mut started = Vec::new();
    for _ in 0..3 {
        let name = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("suite start should not time out")
            .expect("suite start should be received");
        started.push(name);
    }
    started.sort_unstable();
    assert_eq!(started, vec!["devql-sync", "onboarding", "smoke"]);

    barrier.wait().await;

    bundle
        .await
        .expect("bundle task should join")
        .expect("bundle should succeed");
}
