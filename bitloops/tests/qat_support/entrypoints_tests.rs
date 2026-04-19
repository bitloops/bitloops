use super::*;

#[tokio::test]
async fn run_bundle_starts_onboarding_and_agent_smoke_before_devql_phase() {
    use std::sync::Arc;
    use tokio::sync::{Barrier, mpsc};
    use tokio::time::{Duration, timeout};

    let (tx, mut rx) = mpsc::unbounded_channel::<&'static str>();
    let first_phase_barrier = Arc::new(Barrier::new(3));
    let make_first_phase_suite = |name: &'static str| {
        let tx = tx.clone();
        let barrier = Arc::clone(&first_phase_barrier);
        async move {
            tx.send(name).expect("suite start should send");
            barrier.wait().await;
            Ok(())
        }
    };
    let make_later_suite = |name: &'static str| {
        let tx = tx.clone();
        async move {
            tx.send(name).expect("suite start should send");
            Ok(())
        }
    };

    let bundle = tokio::spawn(run_bundle_from_futures(
        make_first_phase_suite("onboarding"),
        make_first_phase_suite("agent-smoke"),
        make_later_suite("devql-sync"),
        make_later_suite("devql-capabilities"),
        make_later_suite("devql-ingest"),
    ));

    let first = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("first suite start should not time out")
        .expect("first suite start should be received");
    let second = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("second suite start should not time out")
        .expect("second suite start should be received");

    assert!(
        matches!(
            (first, second),
            ("onboarding", "agent-smoke") | ("agent-smoke", "onboarding")
        ),
        "bundle should start onboarding and agent-smoke first, observed {first:?} and {second:?}"
    );
    assert!(
        timeout(Duration::from_millis(200), rx.recv())
            .await
            .is_err(),
        "devql suites should not start until the onboarding/agent-smoke phase completes"
    );

    first_phase_barrier.wait().await;

    let devql_sync = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("devql-sync start should not time out")
        .expect("devql-sync start should be received");
    assert_eq!(devql_sync, "devql-sync");

    bundle
        .await
        .expect("bundle task should join")
        .expect("bundle should succeed");
}

#[tokio::test]
async fn run_bundle_runs_devql_suites_sequentially_after_parallel_phase() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::{Barrier, Mutex, mpsc};

    let (tx, mut rx) = mpsc::unbounded_channel::<&'static str>();
    let first_phase_barrier = Arc::new(Barrier::new(3));
    let active_devql = Arc::new(AtomicUsize::new(0));
    let max_active_devql = Arc::new(AtomicUsize::new(0));
    let completion_order = Arc::new(Mutex::new(Vec::new()));

    let make_first_phase_suite = |name: &'static str| {
        let tx = tx.clone();
        let barrier = Arc::clone(&first_phase_barrier);
        let completion_order = Arc::clone(&completion_order);
        async move {
            tx.send(name).expect("suite start should send");
            barrier.wait().await;
            completion_order.lock().await.push(name);
            Ok(())
        }
    };
    let make_serial_devql_suite = |name: &'static str| {
        let tx = tx.clone();
        let active_devql = Arc::clone(&active_devql);
        let max_active_devql = Arc::clone(&max_active_devql);
        let completion_order = Arc::clone(&completion_order);
        async move {
            tx.send(name).expect("suite start should send");
            let current_active = active_devql.fetch_add(1, Ordering::SeqCst) + 1;
            max_active_devql.fetch_max(current_active, Ordering::SeqCst);
            tokio::task::yield_now().await;
            active_devql.fetch_sub(1, Ordering::SeqCst);
            completion_order.lock().await.push(name);
            Ok(())
        }
    };

    let bundle = tokio::spawn(run_bundle_from_futures(
        make_first_phase_suite("onboarding"),
        make_first_phase_suite("agent-smoke"),
        make_serial_devql_suite("devql-sync"),
        make_serial_devql_suite("devql-capabilities"),
        make_serial_devql_suite("devql-ingest"),
    ));

    let _ = rx.recv().await.expect("onboarding or smoke should start");
    let _ = rx.recv().await.expect("onboarding or smoke should start");
    first_phase_barrier.wait().await;

    let observed = vec![
        rx.recv().await.expect("devql-sync should start"),
        rx.recv().await.expect("devql-capabilities should start"),
        rx.recv().await.expect("devql-ingest should start"),
    ];
    assert_eq!(
        observed,
        vec!["devql-sync", "devql-capabilities", "devql-ingest"],
        "devql-heavy suites should start in a fixed serial order after the initial parallel phase"
    );

    bundle
        .await
        .expect("bundle task should join")
        .expect("bundle should succeed");

    assert_eq!(
        max_active_devql.load(Ordering::SeqCst),
        1,
        "devql-heavy suites should not overlap in the bundled run"
    );

    let completion_order = completion_order.lock().await.clone();
    assert!(
        completion_order.ends_with(&["devql-sync", "devql-capabilities", "devql-ingest",]),
        "devql-heavy suites should complete in serial order, got {completion_order:?}"
    );
}

#[tokio::test]
async fn run_serial_suites_with_runner_executes_requested_suites_in_order() {
    use tokio::sync::{Mutex, mpsc};

    let (tx, mut rx) = mpsc::unbounded_channel::<(&'static str, Option<&'static str>)>();
    let completions = std::sync::Arc::new(Mutex::new(Vec::new()));
    let suites = [
        Suite::AgentSmoke,
        Suite::DevqlSync,
        Suite::AgentsCheckpoints,
    ];
    let rerun_alias = "cargo qat-develop-gate";

    run_serial_suites_with_runner(
        PathBuf::from("/tmp/bitloops"),
        &suites,
        Some("@develop_gate"),
        rerun_alias,
        {
            let completions = std::sync::Arc::clone(&completions);
            move |_binary, suite, tags_filter| {
                let tx = tx.clone();
                let completions = std::sync::Arc::clone(&completions);
                async move {
                    tx.send((suite.id(), tags_filter))
                        .expect("suite start should send");
                    completions.lock().await.push(suite.id());
                    Ok(())
                }
            }
        },
    )
    .await
    .expect("serial subset run should succeed");

    let observed = vec![
        rx.recv().await.expect("first suite should start"),
        rx.recv().await.expect("second suite should start"),
        rx.recv().await.expect("third suite should start"),
    ];
    assert_eq!(
        observed,
        vec![
            ("agent-smoke", Some("@develop_gate")),
            ("devql-sync", Some("@develop_gate")),
            ("agents-checkpoints", Some("@develop_gate")),
        ],
        "serial subset run should execute suites in the configured order with the shared tag filter"
    );

    let completions = completions.lock().await.clone();
    assert_eq!(
        completions,
        vec!["agent-smoke", "devql-sync", "agents-checkpoints"],
        "serial subset run should complete in the configured order"
    );
}

#[tokio::test]
async fn run_serial_suites_with_runner_reports_develop_gate_rerun_hint() {
    let err = run_serial_suites_with_runner(
        PathBuf::from("/tmp/bitloops"),
        &[Suite::DevqlSync],
        Some("@develop_gate"),
        "cargo qat-develop-gate",
        |_binary, suite, _tags_filter| async move { anyhow::bail!("{} failed", suite.id()) },
    )
    .await
    .expect_err("serial subset run should surface failures");

    let message = format!("{err:#}");
    assert!(
        message.contains("devql-sync"),
        "develop gate failure should name the failing suite: {message}"
    );
    assert!(
        message.contains("cargo qat-develop-gate"),
        "develop gate failure should point to the gate rerun alias: {message}"
    );
}
