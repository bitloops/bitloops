#![allow(dead_code)]

mod qat_support;

#[tokio::test]
#[ignore = "slow E2E: runs QAT Agent Smoke suite; use `cargo qat-agent-smoke`"]
async fn qat_agent_smoke() {
    qat_support::entrypoints::run_suite_entrypoint(qat_support::runner::Suite::AgentSmoke)
        .await
        .expect("QAT Agent Smoke suite failed");
}
