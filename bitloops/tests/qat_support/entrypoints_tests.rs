use super::*;

#[tokio::test]
async fn run_bundle_starts_all_suites_before_completion() {
    let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(6));
    let runner = {
        let started = std::sync::Arc::clone(&started);
        let notify = std::sync::Arc::clone(&notify);
        let barrier = std::sync::Arc::clone(&barrier);
        move |_binary: PathBuf, suite: Suite| {
            let started = std::sync::Arc::clone(&started);
            let notify = std::sync::Arc::clone(&notify);
            let barrier = std::sync::Arc::clone(&barrier);
            async move {
                match suite {
                    Suite::Onboarding
                    | Suite::Smoke
                    | Suite::DevqlSync
                    | Suite::Devql
                    | Suite::DevqlIngest => {
                        started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        notify.notify_waiters();
                        barrier.wait().await;
                    }
                    _ => panic!("unexpected suite in bundle test"),
                }
                Ok(())
            }
        }
    };

    let bundle = tokio::spawn(run_bundle_with_runner(PathBuf::from("bitloops"), runner));

    loop {
        if started.load(std::sync::atomic::Ordering::SeqCst) == 5 {
            break;
        }
        notify.notified().await;
    }

    assert_eq!(
        started.load(std::sync::atomic::Ordering::SeqCst),
        5,
        "expected all bundled suites to start before the bundle completes"
    );

    barrier.wait().await;

    bundle
        .await
        .expect("bundle task should join")
        .expect("bundle should succeed");
}

#[tokio::test]
async fn run_bundle_from_futures_runs_all_suites_in_parallel() {
    use std::sync::Arc;
    use tokio::sync::{Barrier, Mutex, mpsc};
    use tokio::time::{Duration, timeout};

    let (tx, mut rx) = mpsc::unbounded_channel::<&'static str>();
    let barrier = Arc::new(Barrier::new(6));
    let completion_order = Arc::new(Mutex::new(Vec::new()));
    let make_suite = |name: &'static str| {
        let tx = tx.clone();
        let barrier = Arc::clone(&barrier);
        let completion_order = Arc::clone(&completion_order);
        async move {
            tx.send(name).expect("suite start should send");
            barrier.wait().await;
            completion_order.lock().await.push(name);
            Ok(())
        }
    };

    let bundle = tokio::spawn(run_bundle_from_futures(
        make_suite("onboarding"),
        make_suite("smoke"),
        make_suite("devql-sync"),
        make_suite("devql-capabilities"),
        make_suite("devql-ingest"),
    ));

    let mut observed = Vec::new();
    for _ in 0..5 {
        let suite = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("suite start should not time out")
            .expect("suite start should be received");
        observed.push(suite);
    }

    assert!(
        observed.contains(&"onboarding"),
        "bundle should start onboarding in the parallel fan-out, observed {observed:?}"
    );
    assert!(
        observed.contains(&"smoke"),
        "bundle should start smoke in the parallel fan-out, observed {observed:?}"
    );
    assert!(
        observed.contains(&"devql-sync"),
        "bundle should start devql-sync in the parallel fan-out, observed {observed:?}"
    );
    assert!(
        observed.contains(&"devql-capabilities"),
        "bundle should start devql-capabilities in the parallel fan-out, observed {observed:?}"
    );
    assert!(
        observed.contains(&"devql-ingest"),
        "bundle should start devql-ingest in the parallel fan-out, observed {observed:?}"
    );

    barrier.wait().await;

    bundle
        .await
        .expect("bundle task should join")
        .expect("bundle should succeed");

    let completion_order = completion_order.lock().await.clone();
    assert!(
        completion_order.contains(&"onboarding"),
        "bundle should complete onboarding: {completion_order:?}"
    );
    assert!(
        completion_order.contains(&"smoke"),
        "bundle should complete smoke: {completion_order:?}"
    );
    assert!(
        completion_order.contains(&"devql-sync"),
        "bundle should complete devql-sync: {completion_order:?}"
    );
    assert!(
        completion_order.contains(&"devql-capabilities"),
        "bundle should complete devql-capabilities: {completion_order:?}"
    );
    assert!(
        completion_order.contains(&"devql-ingest"),
        "bundle should complete devql-ingest: {completion_order:?}"
    );
}
