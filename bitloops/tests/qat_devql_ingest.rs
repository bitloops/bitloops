#![allow(dead_code)]

mod qat_support;

#[tokio::test]
#[ignore = "slow E2E: runs QAT DevQL ingest suite; use `cargo qat-devql-ingest` or `cargo qat`"]
async fn qat_devql_ingest() {
    qat_support::entrypoints::run_suite_entrypoint(qat_support::runner::Suite::DevqlIngest)
        .await
        .expect("QAT DevQL ingest suite failed");
}
