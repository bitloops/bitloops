#![allow(dead_code)]

use std::sync::{Mutex, OnceLock};

fn logger_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn with_logger_test_lock<T>(f: impl FnOnce() -> T) -> T {
    let _guard = logger_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f()
}
