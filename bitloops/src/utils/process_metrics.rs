#[cfg(unix)]
pub(crate) fn current_process_max_rss_kb() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: `getrusage` initializes the `rusage` struct on success.
    let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if status != 0 {
        return 0;
    }
    // SAFETY: the struct was initialized by `getrusage` above.
    let usage = unsafe { usage.assume_init() };
    let raw = u64::try_from(usage.ru_maxrss).unwrap_or_default();
    #[cfg(target_os = "macos")]
    {
        raw / 1024
    }
    #[cfg(not(target_os = "macos"))]
    {
        raw
    }
}

#[cfg(not(unix))]
pub(crate) fn current_process_max_rss_kb() -> u64 {
    0
}
