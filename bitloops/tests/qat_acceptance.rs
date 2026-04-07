mod qat_support;

use anyhow::Result;
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
#[ignore = "slow E2E: runs QAT onboarding + DevQL sync suites in parallel, then smoke; use `cargo qat`"]
async fn qat() {
    let binary = resolve_binary();
    run_bundle(binary).await.expect("QAT bundle suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT smoke suite; use `cargo test --test qat_acceptance qat_smoke -- --ignored`"]
async fn qat_smoke() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Smoke)
        .await
        .expect("QAT smoke suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT DevQL suite; use `cargo test --test qat_acceptance qat_devql -- --ignored`"]
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
#[ignore = "slow E2E: runs QAT quickstart suite; use `cargo test --test qat_acceptance qat_quickstart -- --ignored`"]
async fn qat_quickstart() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Quickstart)
        .await
        .expect("QAT quickstart suite failed");
}

async fn run_bundle(binary: PathBuf) -> Result<()> {
    let (onboarding, devql_sync) = tokio::join!(
        runner::run_suite(binary.clone(), Suite::Onboarding),
        runner::run_suite(binary.clone(), Suite::DevqlSync)
    );
    let smoke = runner::run_suite(binary, Suite::Smoke).await;
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
fn combine_bundle_results_returns_ok_when_both_suites_pass() {
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
