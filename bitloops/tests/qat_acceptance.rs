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
#[ignore = "slow E2E: runs QAT onboarding + DevQL sync in parallel, then smoke, then DevQL capabilities; use `cargo qat`"]
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
#[ignore = "slow E2E: runs QAT DevQL capabilities suite; use `cargo qat-devql-capabilities`"]
async fn qat_devql_capabilities() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Devql)
        .await
        .expect("QAT DevQL capabilities suite failed");
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
    let (onboarding, devql_sync) = tokio::join!(
        runner(binary.clone(), Suite::Onboarding),
        runner(binary.clone(), Suite::DevqlSync)
    );
    let smoke = runner(binary.clone(), Suite::Smoke).await;
    let devql = runner(binary, Suite::Devql).await;
    combine_bundle_results(onboarding, devql_sync, smoke, devql)
}

async fn run_bundle_from_futures<OnboardingFuture, DevqlSyncFuture, SmokeFuture, DevqlFuture>(
    onboarding: OnboardingFuture,
    devql_sync: DevqlSyncFuture,
    smoke: SmokeFuture,
    devql: DevqlFuture,
) -> Result<()>
where
    OnboardingFuture: Future<Output = Result<()>>,
    DevqlSyncFuture: Future<Output = Result<()>>,
    SmokeFuture: Future<Output = Result<()>>,
    DevqlFuture: Future<Output = Result<()>>,
{
    let (onboarding, devql_sync) = tokio::join!(onboarding, devql_sync);
    let smoke = smoke.await;
    let devql = devql.await;
    combine_bundle_results(onboarding, devql_sync, smoke, devql)
}

fn combine_bundle_results(
    onboarding: Result<()>,
    devql_sync: Result<()>,
    smoke: Result<()>,
    devql: Result<()>,
) -> Result<()> {
    match (onboarding, devql_sync, smoke, devql) {
        (Ok(()), Ok(()), Ok(()), Ok(())) => Ok(()),
        (Err(err), Ok(()), Ok(()), Ok(())) => Err(err),
        (Ok(()), Err(err), Ok(()), Ok(())) => Err(err),
        (Ok(()), Ok(()), Err(err), Ok(())) => Err(err),
        (Ok(()), Ok(()), Ok(()), Err(err)) => Err(err),
        (onboarding, devql_sync, smoke, devql) => {
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
            if let Err(err) = devql {
                message.push_str(&format!("\n- devql: {err:#}"));
            }
            Err(anyhow::anyhow!(message))
        }
    }
}

#[test]
fn combine_bundle_results_returns_ok_when_all_suites_pass() {
    combine_bundle_results(Ok(()), Ok(()), Ok(()), Ok(())).expect("all suites should pass");
}

#[test]
fn combine_bundle_results_returns_onboarding_error_when_only_onboarding_fails() {
    let err = combine_bundle_results(
        Err(anyhow::anyhow!("onboarding failed")),
        Ok(()),
        Ok(()),
        Ok(()),
    )
    .expect_err("onboarding failure should surface");
    assert!(
        format!("{err:#}").contains("onboarding failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_returns_sync_error_when_only_sync_fails() {
    let err = combine_bundle_results(Ok(()), Err(anyhow::anyhow!("sync failed")), Ok(()), Ok(()))
        .expect_err("sync failure should surface");
    assert!(
        format!("{err:#}").contains("sync failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_returns_smoke_error_when_only_smoke_fails() {
    let err = combine_bundle_results(Ok(()), Ok(()), Err(anyhow::anyhow!("smoke failed")), Ok(()))
        .expect_err("smoke failure should surface");
    assert!(
        format!("{err:#}").contains("smoke failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_returns_devql_error_when_only_devql_fails() {
    let err = combine_bundle_results(Ok(()), Ok(()), Ok(()), Err(anyhow::anyhow!("devql failed")))
        .expect_err("devql failure should surface");
    assert!(
        format!("{err:#}").contains("devql failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_reports_all_failures() {
    let err = combine_bundle_results(
        Err(anyhow::anyhow!("onboarding failed")),
        Err(anyhow::anyhow!("sync failed")),
        Err(anyhow::anyhow!("smoke failed")),
        Err(anyhow::anyhow!("devql failed")),
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
    assert!(
        message.contains("devql failed"),
        "combined error missing devql details: {message}"
    );
}

#[tokio::test]
async fn run_bundle_starts_onboarding_and_devql_sync_before_smoke_and_devql() {
    let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
    let parallel_stage_complete = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let runner = {
        let started = std::sync::Arc::clone(&started);
        let notify = std::sync::Arc::clone(&notify);
        let barrier = std::sync::Arc::clone(&barrier);
        let parallel_stage_complete = std::sync::Arc::clone(&parallel_stage_complete);
        move |_binary: PathBuf, suite: Suite| {
            let started = std::sync::Arc::clone(&started);
            let notify = std::sync::Arc::clone(&notify);
            let barrier = std::sync::Arc::clone(&barrier);
            let parallel_stage_complete = std::sync::Arc::clone(&parallel_stage_complete);
            async move {
                match suite {
                    Suite::Onboarding | Suite::DevqlSync => {
                        started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        notify.notify_waiters();
                        barrier.wait().await;
                        parallel_stage_complete.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    Suite::Smoke | Suite::Devql => {
                        assert!(
                            parallel_stage_complete.load(std::sync::atomic::Ordering::SeqCst),
                            "smoke/devql should not start before onboarding/devql-sync finish"
                        );
                    }
                    _ => panic!("unexpected suite in bundle test"),
                }
                Ok(())
            }
        }
    };

    let bundle = tokio::spawn(run_bundle_with_runner(PathBuf::from("bitloops"), runner));

    loop {
        if started.load(std::sync::atomic::Ordering::SeqCst) == 2 {
            break;
        }
        notify.notified().await;
    }

    assert_eq!(
        started.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "expected onboarding and devql-sync to both start before later bundle stages"
    );

    barrier.wait().await;

    bundle
        .await
        .expect("bundle task should join")
        .expect("bundle should succeed");
}

#[tokio::test]
async fn run_bundle_from_futures_runs_parallel_stage_then_smoke_then_devql() {
    use std::sync::Arc;
    use tokio::sync::{Barrier, Mutex, mpsc};
    use tokio::time::{Duration, timeout};

    let (tx, mut rx) = mpsc::unbounded_channel::<&'static str>();
    let barrier = Arc::new(Barrier::new(3));
    let completion_order = Arc::new(Mutex::new(Vec::new()));
    let make_suite = |name: &'static str| {
        let tx = tx.clone();
        let barrier = Arc::clone(&barrier);
        let completion_order = Arc::clone(&completion_order);
        async move {
            tx.send(name).expect("suite start should send");
            if matches!(name, "onboarding" | "devql-sync") {
                barrier.wait().await;
            }
            completion_order.lock().await.push(name);
            Ok(())
        }
    };

    let bundle = tokio::spawn(run_bundle_from_futures(
        make_suite("onboarding"),
        make_suite("devql-sync"),
        make_suite("smoke"),
        make_suite("devql"),
    ));

    let first = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("first suite start should not time out")
        .expect("first suite start should be received");
    let second = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("second suite start should not time out")
        .expect("second suite start should be received");
    let observed = [first, second];
    assert!(
        observed.contains(&"onboarding"),
        "parallel stage should start onboarding first, observed {observed:?}"
    );
    assert!(
        observed.contains(&"devql-sync"),
        "parallel stage should start devql-sync first, observed {observed:?}"
    );
    assert!(
        timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err(),
        "smoke/devql should wait for the parallel stage to finish"
    );

    barrier.wait().await;

    let third = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("smoke start should not time out")
        .expect("smoke start should be received");
    let fourth = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("devql start should not time out")
        .expect("devql start should be received");
    assert_eq!(third, "smoke", "smoke should start before devql");
    assert_eq!(fourth, "devql", "devql should start after smoke");

    bundle
        .await
        .expect("bundle task should join")
        .expect("bundle should succeed");

    let completion_order = completion_order.lock().await.clone();
    let first_two = &completion_order[..2];
    assert!(
        first_two.contains(&"onboarding"),
        "parallel stage should complete onboarding before later stages: {completion_order:?}"
    );
    assert!(
        first_two.contains(&"devql-sync"),
        "parallel stage should complete devql-sync before later stages: {completion_order:?}"
    );
    assert_eq!(
        &completion_order[2..],
        &["smoke", "devql"],
        "bundle should complete the parallel stage, then smoke, then devql order"
    );
}
