#[path = "qat_support/bundle.rs"]
mod bundle;

use bundle::combine_bundle_results;

#[test]
fn combine_bundle_results_returns_ok_when_both_suites_pass() {
    combine_bundle_results(Ok(()), Ok(())).expect("both suites should pass");
}

#[test]
fn combine_bundle_results_returns_onboarding_error_when_only_onboarding_fails() {
    let err = combine_bundle_results(Err(anyhow::anyhow!("onboarding failed")), Ok(()))
        .expect_err("onboarding failure should surface");
    assert!(
        format!("{err:#}").contains("onboarding failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_returns_sync_error_when_only_sync_fails() {
    let err = combine_bundle_results(Ok(()), Err(anyhow::anyhow!("sync failed")))
        .expect_err("sync failure should surface");
    assert!(
        format!("{err:#}").contains("sync failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_reports_both_failures() {
    let err = combine_bundle_results(
        Err(anyhow::anyhow!("onboarding failed")),
        Err(anyhow::anyhow!("sync failed")),
    )
    .expect_err("both failures should be preserved");
    let message = format!("{err:#}");
    assert!(
        message.contains("onboarding failed"),
        "combined error missing onboarding details: {message}"
    );
    assert!(
        message.contains("sync failed"),
        "combined error missing sync details: {message}"
    );
}
