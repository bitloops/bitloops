#![allow(dead_code)]

mod qat_support;

#[tokio::test]
#[ignore = "slow E2E: runs all bundled QAT suites in parallel; use `cargo qat`"]
async fn qat() {
    if let Err(err) = qat_support::entrypoints::run_bundle_entrypoint().await {
        panic!("{err:#}");
    }
}
