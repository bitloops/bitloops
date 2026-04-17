use std::future::Future;
use std::sync::{Mutex, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};

use super::logger_lock::{with_logger_test_lock, with_logger_test_lock_async};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedLogRecord {
    pub(crate) level: Level,
    pub(crate) target: String,
    pub(crate) message: String,
}

#[derive(Default)]
struct TestCaptureLogger {
    records: Mutex<Vec<CapturedLogRecord>>,
}

impl TestCaptureLogger {
    fn clear(&self) {
        if let Ok(mut records) = self.records.lock() {
            records.clear();
        }
    }

    fn take(&self) -> Vec<CapturedLogRecord> {
        self.records
            .lock()
            .map(|mut records| std::mem::take(&mut *records))
            .unwrap_or_default()
    }
}

impl Log for TestCaptureLogger {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if let Ok(mut records) = self.records.lock() {
            records.push(CapturedLogRecord {
                level: record.level(),
                target: record.target().to_string(),
                message: record.args().to_string(),
            });
        }
    }

    fn flush(&self) {}
}

fn installed_logger() -> &'static TestCaptureLogger {
    static LOGGER: OnceLock<&'static TestCaptureLogger> = OnceLock::new();
    LOGGER.get_or_init(|| {
        let logger = Box::leak(Box::new(TestCaptureLogger::default()));
        let _ = log::set_logger(logger);
        log::set_max_level(LevelFilter::Trace);
        logger
    })
}

pub(crate) fn capture_logs<T>(f: impl FnOnce() -> T) -> (T, Vec<CapturedLogRecord>) {
    with_logger_test_lock(|| {
        let logger = installed_logger();
        logger.clear();
        let result = f();
        let records = logger.take();
        (result, records)
    })
}

pub(crate) async fn capture_logs_async<T>(
    future: impl Future<Output = T>,
) -> (T, Vec<CapturedLogRecord>) {
    with_logger_test_lock_async(async {
        let logger = installed_logger();
        logger.clear();
        let result = future.await;
        let records = logger.take();
        (result, records)
    })
    .await
}
