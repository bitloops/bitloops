use std::future::Future;
use std::sync::{Mutex, OnceLock};

fn logger_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn logger_test_guard() -> std::sync::MutexGuard<'static, ()> {
    logger_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn with_logger_test_lock<T>(f: impl FnOnce() -> T) -> T {
    let _guard = logger_test_guard();
    f()
}

pub(crate) async fn with_logger_test_lock_async<T>(future: impl Future<Output = T>) -> T {
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let lock_task = tokio::task::spawn_blocking(move || {
        let _guard = logger_test_guard();
        let _ = ready_tx.send(());
        let _ = release_rx.blocking_recv();
    });

    ready_rx
        .await
        .expect("logger test lock task should acquire the lock");
    let result = future.await;
    let _ = release_tx.send(());
    lock_task
        .await
        .expect("logger test lock task should exit cleanly");
    result
}

#[cfg(test)]
mod tests {
    use super::{with_logger_test_lock, with_logger_test_lock_async};

    #[test]
    fn lock_wrapper_executes_closure() {
        let value = with_logger_test_lock(|| 42);
        assert_eq!(value, 42);
    }

    #[tokio::test]
    async fn async_lock_wrapper_executes_future() {
        let value = with_logger_test_lock_async(async { 42 }).await;
        assert_eq!(value, 42);
    }
}
