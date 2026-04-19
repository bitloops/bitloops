//! Tuning constants for the managed runtime download pipeline.
//!
//! Test builds use deliberately small thresholds so the parallel paths can be
//! exercised against the in-process mock server without shipping multi-megabyte
//! fixtures.

pub(crate) const MANAGED_RELEASE_DOWNLOAD_BUFFER_BYTES: usize = 1024 * 1024;

#[cfg(not(test))]
pub(crate) const MIN_PARALLEL_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024;
#[cfg(test)]
pub(crate) const MIN_PARALLEL_DOWNLOAD_BYTES: u64 = 1024;

#[cfg(not(test))]
pub(crate) const TARGET_PARALLEL_DOWNLOAD_PART_BYTES: u64 = 16 * 1024 * 1024;
#[cfg(test)]
pub(crate) const TARGET_PARALLEL_DOWNLOAD_PART_BYTES: u64 = 1024;

#[cfg(not(test))]
pub(crate) const MAX_PARALLEL_DOWNLOAD_PARTS: usize = 6;
#[cfg(test)]
pub(crate) const MAX_PARALLEL_DOWNLOAD_PARTS: usize = 4;

#[cfg(not(test))]
pub(crate) const PARALLEL_RANGE_TIMEOUT_GRACE_SECS: u64 = 15;
#[cfg(not(test))]
pub(crate) const PARALLEL_RANGE_TIMEOUT_MIN_BYTES_PER_SEC: u64 = 1024 * 1024;
