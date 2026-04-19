#![allow(dead_code)]

mod qat_support;

#[tokio::test]
#[ignore = "slow E2E: runs QAT onboarding suite; use `cargo qat-onboarding` or `cargo qat`"]
async fn qat_onboarding() {
    qat_support::entrypoints::run_suite_entrypoint(qat_support::runner::Suite::Onboarding)
        .await
        .expect("QAT onboarding suite failed");
}
