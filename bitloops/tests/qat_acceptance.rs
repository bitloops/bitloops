mod qat_support;

use std::path::PathBuf;

use qat_support::runner::{self, Suite};

fn resolve_binary() -> PathBuf {
    if let Ok(path_raw) = std::env::var("BITLOOPS_QAT_BINARY") {
        let path = PathBuf::from(path_raw);
        assert!(
            path.exists(),
            "BITLOOPS_QAT_BINARY points to {}, which does not exist",
            path.display()
        );
        return path;
    }

    PathBuf::from(env!("CARGO_BIN_EXE_bitloops"))
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT smoke suite; use `cargo test --test qat_acceptance qat_smoke -- --ignored`"]
async fn qat_smoke() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Smoke)
        .await
        .expect("QAT smoke suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT DevQL suite; use `cargo test --test qat_acceptance qat_devql -- --ignored`"]
async fn qat_devql() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Devql)
        .await
        .expect("QAT DevQL suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT DevQL sync suite; use `cargo qat-devql-sync` or `cargo qat`"]
async fn qat_devql_sync() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::DevqlSync)
        .await
        .expect("QAT DevQL sync suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT Claude Code suite; use `cargo test --test qat_acceptance qat_claude_code -- --ignored`"]
async fn qat_claude_code() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::ClaudeCode)
        .await
        .expect("QAT Claude Code suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT onboarding suite; use `cargo qat-onboarding` or `cargo qat`"]
async fn qat_onboarding() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Onboarding)
        .await
        .expect("QAT onboarding suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT quickstart suite; use `cargo test --test qat_acceptance qat_quickstart -- --ignored`"]
async fn qat_quickstart() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Quickstart)
        .await
        .expect("QAT quickstart suite failed");
}
