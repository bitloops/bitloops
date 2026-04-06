use std::fs;
use tempfile::tempdir;

#[test]
fn sync_lock_acquires_and_releases() {
    let dir = tempdir().expect("temp dir");
    let lock_path = dir.path().join(".bitloops").join("sync.lock");

    let lock =
        crate::host::devql::sync::lock::SyncLock::acquire(dir.path()).expect("acquire sync lock");
    assert!(lock.is_held());
    assert!(lock_path.exists(), "lock file should exist while held");

    drop(lock);

    assert!(
        !lock_path.exists(),
        "lock file should be removed when lock is dropped"
    );

    let lock2 = crate::host::devql::sync::lock::SyncLock::acquire(dir.path())
        .expect("re-acquire sync lock");
    assert!(lock2.is_held());
}

#[test]
fn sync_lock_fails_fast_when_held() {
    let dir = tempdir().expect("temp dir");
    let _lock =
        crate::host::devql::sync::lock::SyncLock::acquire(dir.path()).expect("acquire sync lock");

    let result = crate::host::devql::sync::lock::SyncLock::try_acquire(dir.path());

    assert!(result.is_err(), "second acquisition should fail fast");
}

#[test]
fn sync_lock_partial_file_is_treated_as_held() {
    let dir = tempdir().expect("temp dir");
    let lock_dir = dir.path().join(".bitloops");
    let lock_path = lock_dir.join("sync.lock");
    fs::create_dir_all(&lock_dir).expect("create lock dir");
    fs::write(&lock_path, format!("{}\n", std::process::id())).expect("write partial lock");

    let result = crate::host::devql::sync::lock::SyncLock::try_acquire(dir.path());

    assert!(
        result.is_err(),
        "partial lock file should be treated as held"
    );
    assert_eq!(
        fs::read_to_string(&lock_path).expect("read partial lock"),
        format!("{}\n", std::process::id()),
        "partial lock file should not be cleared as stale"
    );
}

#[test]
fn sync_lock_malformed_file_is_treated_as_held() {
    let dir = tempdir().expect("temp dir");
    let lock_dir = dir.path().join(".bitloops");
    let lock_path = lock_dir.join("sync.lock");
    fs::create_dir_all(&lock_dir).expect("create lock dir");
    fs::write(&lock_path, "not-a-pid\nmalformed-token\n").expect("write malformed lock");

    let result = crate::host::devql::sync::lock::SyncLock::try_acquire(dir.path());

    assert!(
        result.is_err(),
        "malformed lock file should be treated as held"
    );
    assert_eq!(
        fs::read_to_string(&lock_path).expect("read malformed lock"),
        "not-a-pid\nmalformed-token\n",
        "malformed lock file should not be cleared as stale"
    );
}
