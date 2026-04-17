#![allow(dead_code)]

mod qat_support;

#[tokio::test]
#[ignore = "slow E2E: runs QAT DevQL capabilities suite; use `cargo qat-devql-capabilities` or `cargo qat`"]
async fn qat_devql_capabilities() {
    qat_support::entrypoints::run_suite_entrypoint(qat_support::runner::Suite::Devql)
        .await
        .expect("QAT DevQL capabilities suite failed");
}
