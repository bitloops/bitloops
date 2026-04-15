#![allow(dead_code)]

mod qat_support;

#[tokio::test]
#[ignore = "slow E2E: runs QAT quickstart suite; use `cargo test --features qat-tests --test qat_quickstart -- --ignored`"]
async fn qat_quickstart() {
    qat_support::entrypoints::run_suite_entrypoint(qat_support::runner::Suite::Quickstart)
        .await
        .expect("QAT quickstart suite failed");
}
