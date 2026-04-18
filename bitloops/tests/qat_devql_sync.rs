#![allow(dead_code)]

mod qat_support;

#[tokio::test]
#[ignore = "slow E2E: runs QAT DevQL sync suite; use `cargo qat-devql-sync` or `cargo qat`"]
async fn qat_devql_sync() {
    qat_support::entrypoints::run_suite_entrypoint(qat_support::runner::Suite::DevqlSync)
        .await
        .expect("QAT DevQL sync suite failed");
}
