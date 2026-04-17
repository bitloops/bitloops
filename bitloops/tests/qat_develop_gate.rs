#![allow(dead_code)]

mod qat_support;

#[tokio::test]
#[ignore = "slow E2E: runs the curated QAT Develop Gate subset; use `cargo qat-develop-gate`"]
async fn qat_develop_gate() {
    qat_support::entrypoints::run_develop_gate_entrypoint()
        .await
        .expect("QAT Develop Gate failed");
}
