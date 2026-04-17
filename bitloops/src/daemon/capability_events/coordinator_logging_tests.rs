use log::Level;

use super::*;
use crate::test_support::log_capture::capture_logs;

fn sample_run(status: CapabilityEventRunStatus) -> CapabilityEventRunRecord {
    CapabilityEventRunRecord {
        run_id: "run-1".to_string(),
        repo_id: "repo-1".to_string(),
        capability_id: "test_harness".to_string(),
        consumer_id: "test_harness.current_state".to_string(),
        handler_id: "test_harness.current_state".to_string(),
        from_generation_seq: 2,
        to_generation_seq: 5,
        reconcile_mode: "merged_delta".to_string(),
        event_kind: "current_state_consumer".to_string(),
        lane_key: "repo-1:test_harness.current_state".to_string(),
        event_payload_json: String::new(),
        status,
        attempts: 1,
        submitted_at_unix: 10,
        started_at_unix: Some(20),
        updated_at_unix: 30,
        completed_at_unix: None,
        error: Some("stale error".to_string()),
    }
}

#[test]
fn terminal_or_retry_logs_retryable_failure() {
    let run = sample_run(CapabilityEventRunStatus::Running);

    let (completion, records) =
        capture_logs(|| terminal_or_retry(run.clone(), anyhow::anyhow!("temporary failure")));

    assert!(matches!(
        completion,
        RunCompletion::RetryableFailure { error, .. } if error == "temporary failure"
    ));
    assert!(records.iter().any(|record| {
        record.level == Level::Warn
            && record
                .message
                .contains("current-state consumer run failed and will retry")
            && record.message.contains(&run.run_id)
    }));
}

#[test]
fn terminal_or_retry_logs_terminal_failure() {
    let mut run = sample_run(CapabilityEventRunStatus::Running);
    run.attempts = MAX_RUN_ATTEMPTS;

    let (completion, records) =
        capture_logs(|| terminal_or_retry(run.clone(), anyhow::anyhow!("permanent failure")));

    assert!(matches!(
        completion,
        RunCompletion::Failed { error, .. } if error == "permanent failure"
    ));
    assert!(records.iter().any(|record| {
        record.level == Level::Error
            && record.message.contains("current-state consumer run failed")
            && record.message.contains(&run.run_id)
    }));
}
