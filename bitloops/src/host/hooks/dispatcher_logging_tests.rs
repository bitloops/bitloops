use std::fs;
use std::path::Path;

use anyhow::Result;
use tempfile::TempDir;

use super::*;
use crate::telemetry::logging;
use crate::test_support::logger_lock::with_logger_test_lock;
use crate::test_support::process_state::with_process_state;

const TEST_STATE_DIR_OVERRIDE_ENV: &str = "BITLOOPS_TEST_STATE_DIR_OVERRIDE";

fn with_test_logging_state<T>(repo_root: &Path, f: impl FnOnce() -> T) -> T {
    let state_root = repo_root.join("state-root");
    let state_root_value = state_root.display().to_string();
    with_process_state(
        Some(repo_root),
        &[(TEST_STATE_DIR_OVERRIDE_ENV, Some(state_root_value.as_str()))],
        f,
    )
}

#[test]
fn run_agent_hook_with_logging_records_failure_entry() {
    let dir = TempDir::new().expect("temp dir");

    with_test_logging_state(dir.path(), || {
        with_logger_test_lock(|| {
            logging::reset_logger_for_tests();

            let result: Result<()> = run_agent_hook_with_logging(
                dir.path(),
                AGENT_NAME_CODEX,
                crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_SESSION_START,
                registry::STRATEGY_NAME_MANUAL_COMMIT,
                || Err(anyhow::anyhow!("simulated agent hook failure")),
            );

            assert!(result.is_err(), "failing hook should return an error");

            let content =
                fs::read_to_string(logging::log_file_path()).expect("hook log file should exist");
            assert!(
                content.contains("\"msg\":\"hook failed\""),
                "expected hook failure entry, got: {content}"
            );
            assert!(
                content.contains("simulated agent hook failure"),
                "expected hook failure details, got: {content}"
            );
        });
    });
}
