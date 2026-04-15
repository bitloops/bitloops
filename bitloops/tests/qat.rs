#![allow(dead_code)]

mod qat_support;

#[tokio::test]
#[ignore = "slow E2E: runs all bundled QAT suites in parallel; use `cargo qat`"]
async fn qat() {
    qat_support::entrypoints::run_bundle_entrypoint()
        .await
        .expect("QAT bundle suite failed");
}
