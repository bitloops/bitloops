#[path = "qat_support/bundle.rs"]
mod bundle;

use bundle::{BundleResult, combine_bundle_results};

fn rerun_alias_for(suite: &'static str) -> &'static str {
    match suite {
        "onboarding" => "cargo qat-onboarding",
        "agent-smoke" => "cargo qat-agent-smoke",
        "devql-sync" => "cargo qat-devql-sync",
        "devql-capabilities" => "cargo qat-devql-capabilities",
        "devql-ingest" => "cargo qat-devql-ingest",
        other => panic!("missing rerun alias for suite `{other}`"),
    }
}

fn ok(suite: &'static str) -> BundleResult {
    BundleResult::ok(suite, rerun_alias_for(suite))
}

fn err(suite: &'static str, message: &str) -> BundleResult {
    BundleResult::err(
        suite,
        rerun_alias_for(suite),
        anyhow::anyhow!(message.to_string()),
    )
}

#[test]
fn combine_bundle_results_returns_ok_when_all_suites_pass() {
    combine_bundle_results(vec![
        ok("onboarding"),
        ok("agent-smoke"),
        ok("devql-sync"),
        ok("devql-capabilities"),
        ok("devql-ingest"),
    ])
    .expect("all suites should pass");
}

#[test]
fn combine_bundle_results_returns_onboarding_error_when_only_onboarding_fails() {
    let err = combine_bundle_results(vec![
        err("onboarding", "onboarding failed"),
        ok("agent-smoke"),
        ok("devql-sync"),
        ok("devql-capabilities"),
        ok("devql-ingest"),
    ])
    .expect_err("onboarding failure should surface");
    assert!(
        format!("{err:#}").contains("onboarding failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_names_single_failed_suite() {
    let err = combine_bundle_results(vec![
        err("devql-capabilities", "capabilities failed"),
        ok("agent-smoke"),
        ok("devql-sync"),
        ok("onboarding"),
        ok("devql-ingest"),
    ])
    .expect_err("single failure should surface the failing suite");
    let message = format!("{err:#}");
    assert!(
        message.contains("devql-capabilities"),
        "single-failure bundle error should name the failing suite: {message}"
    );
    assert!(
        message.contains("cargo qat-devql-capabilities"),
        "single-failure bundle error should include the focused rerun hint: {message}"
    );
}

#[test]
fn combine_bundle_results_returns_sync_error_when_only_sync_fails() {
    let err = combine_bundle_results(vec![
        ok("onboarding"),
        ok("agent-smoke"),
        err("devql-sync", "sync failed"),
        ok("devql-capabilities"),
        ok("devql-ingest"),
    ])
    .expect_err("sync failure should surface");
    assert!(
        format!("{err:#}").contains("sync failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_returns_agent_smoke_error_when_only_agent_smoke_fails() {
    let err = combine_bundle_results(vec![
        ok("onboarding"),
        err("agent-smoke", "agent smoke failed"),
        ok("devql-sync"),
        ok("devql-capabilities"),
        ok("devql-ingest"),
    ])
    .expect_err("agent smoke failure should surface");
    assert!(
        format!("{err:#}").contains("agent smoke failed"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn combine_bundle_results_returns_devql_capabilities_error_when_only_capabilities_fail() {
    let err = combine_bundle_results(vec![
        ok("onboarding"),
        ok("agent-smoke"),
        ok("devql-sync"),
        err("devql-capabilities", "devql failed"),
        ok("devql-ingest"),
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
        ok("onboarding"),
        ok("agent-smoke"),
        ok("devql-sync"),
        ok("devql-capabilities"),
        err("devql-ingest", "ingest failed"),
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
        err("onboarding", "onboarding failed"),
        err("agent-smoke", "agent smoke failed"),
        err("devql-sync", "sync failed"),
        err("devql-capabilities", "capabilities failed"),
        err("devql-ingest", "ingest failed"),
    ])
    .expect_err("all failures should be preserved");
    let message = format!("{err:#}");
    assert!(
        message.contains("onboarding failed"),
        "combined error missing onboarding details: {message}"
    );
    assert!(
        message.contains("agent smoke failed"),
        "combined error missing agent smoke details: {message}"
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
    assert!(
        message.contains("cargo qat-devql-capabilities"),
        "combined error missing capabilities rerun hint: {message}"
    );
    assert!(
        message.contains("cargo qat-devql-ingest"),
        "combined error missing ingest rerun hint: {message}"
    );
}
