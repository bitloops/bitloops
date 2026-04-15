#![allow(dead_code)]

mod qat_support;

#[tokio::test]
#[ignore = "slow E2E: runs QAT smoke suite; use `cargo test --features qat-tests --test qat_smoke -- --ignored` or `cargo qat`"]
async fn qat_smoke() {
    qat_support::entrypoints::run_suite_entrypoint(qat_support::runner::Suite::Smoke)
        .await
        .expect("QAT smoke suite failed");
}
