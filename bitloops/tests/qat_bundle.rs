#[path = "qat_support/bundle.rs"]
mod bundle;

use bundle::{BundleResult, combine_bundle_results};

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
fn combine_bundle_results_returns_devql_ingest_error_when_only_ingest_fails() {
    let err = combine_bundle_results(vec![
        BundleResult::ok("onboarding"),
        BundleResult::ok("smoke"),
        BundleResult::ok("devql-sync"),
        BundleResult::ok("devql-capabilities"),
        BundleResult::err("devql-ingest", anyhow::anyhow!("ingest failed")),
    ])
    .expect_err("sync failure should surface");
    assert!(
        format!("{err:#}").contains("ingest failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_reports_all_failed_suite_names() {
    let err = combine_bundle_results(vec![
        BundleResult::err("onboarding", anyhow::anyhow!("onboarding failed")),
        BundleResult::err("smoke", anyhow::anyhow!("smoke failed")),
        BundleResult::err("devql-sync", anyhow::anyhow!("sync failed")),
        BundleResult::err("devql-capabilities", anyhow::anyhow!("capabilities failed")),
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
        message.contains("capabilities failed"),
        "combined error missing capabilities details: {message}"
    );
    assert!(
        message.contains("ingest failed"),
        "combined error missing ingest details: {message}"
    );
    assert!(
        message.contains("devql-capabilities"),
        "combined error missing capabilities suite name: {message}"
    );
    assert!(
        message.contains("devql-ingest"),
        "combined error missing ingest suite name: {message}"
    );
}
