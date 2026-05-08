#![allow(dead_code)]

mod qat_support;

#[tokio::test]
#[ignore = "slow E2E: runs QAT DevQL sync producer scenarios; use `cargo qat-devql-sync-producer`"]
async fn qat_devql_sync_producer() {
    qat_support::entrypoints::run_devql_sync_producer_entrypoint()
        .await
        .expect("QAT DevQL sync producer suite failed");
}
