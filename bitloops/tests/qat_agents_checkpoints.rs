#![allow(dead_code)]

mod qat_support;

#[tokio::test]
#[ignore = "slow E2E: runs QAT agents-checkpoints suite; use `cargo test --features qat-tests --test qat_agents_checkpoints -- --ignored`"]
async fn qat_agents_checkpoints() {
    qat_support::entrypoints::run_suite_entrypoint(qat_support::runner::Suite::AgentsCheckpoints)
        .await
        .expect("QAT agents-checkpoints suite failed");
}
