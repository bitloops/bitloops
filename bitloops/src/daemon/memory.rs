#[cfg(target_os = "linux")]
use std::fs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessMemorySnapshot {
    pub(crate) resident_bytes: Option<u64>,
    pub(crate) phys_footprint_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PageReleaseResult {
    pub(crate) strategy: &'static str,
    pub(crate) released: bool,
}

pub(crate) trait MemoryMaintenance: std::fmt::Debug + Send + Sync {
    fn capture_process_memory(&self) -> Option<ProcessMemorySnapshot>;
    fn release_unused_pages(&self) -> PageReleaseResult;
}

#[derive(Debug, Default)]
pub(crate) struct PlatformMemoryMaintenance;

impl MemoryMaintenance for PlatformMemoryMaintenance {
    fn capture_process_memory(&self) -> Option<ProcessMemorySnapshot> {
        capture_process_memory()
    }

    fn release_unused_pages(&self) -> PageReleaseResult {
        release_unused_pages()
    }
}

pub(crate) fn capture_process_memory() -> Option<ProcessMemorySnapshot> {
    #[cfg(target_os = "macos")]
    {
        capture_process_memory_macos()
    }
    #[cfg(target_os = "linux")]
    {
        capture_process_memory_linux()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        capture_process_memory_unsupported()
    }
}

pub(crate) fn release_unused_pages() -> PageReleaseResult {
    #[cfg(target_os = "macos")]
    {
        release_unused_pages_macos()
    }
    #[cfg(target_os = "linux")]
    {
        release_unused_pages_linux()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        release_unused_pages_unsupported()
    }
}

#[cfg(target_os = "macos")]
fn capture_process_memory_macos() -> Option<ProcessMemorySnapshot> {
    let mut info = std::mem::MaybeUninit::<TaskVmInfoRev1>::zeroed();
    let mut count = TASK_VM_INFO_REV1_COUNT;
    let status = unsafe {
        task_info(
            mach_task_self(),
            TASK_VM_INFO,
            info.as_mut_ptr().cast(),
            &mut count,
        )
    };
    if status != KERN_SUCCESS {
        return None;
    }
    let info = unsafe { info.assume_init() };
    Some(ProcessMemorySnapshot {
        resident_bytes: Some(info.resident_size),
        phys_footprint_bytes: Some(info.phys_footprint),
    })
}

#[cfg(target_os = "linux")]
fn capture_process_memory_linux() -> Option<ProcessMemorySnapshot> {
    let statm = fs::read_to_string("/proc/self/statm").ok()?;
    let mut parts = statm.split_whitespace();
    let _ = parts.next()?;
    let resident_pages = parts.next()?.parse::<u64>().ok()?;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return None;
    }
    Some(ProcessMemorySnapshot {
        resident_bytes: resident_pages.checked_mul(page_size as u64),
        phys_footprint_bytes: None,
    })
}

#[cfg(any(test, not(any(target_os = "macos", target_os = "linux"))))]
fn capture_process_memory_unsupported() -> Option<ProcessMemorySnapshot> {
    None
}

#[cfg(target_os = "macos")]
fn release_unused_pages_macos() -> PageReleaseResult {
    release_unused_pages_mimalloc()
}

#[cfg(target_os = "linux")]
fn release_unused_pages_linux() -> PageReleaseResult {
    release_unused_pages_mimalloc()
}

#[cfg(any(test, not(any(target_os = "macos", target_os = "linux"))))]
fn release_unused_pages_unsupported() -> PageReleaseResult {
    PageReleaseResult {
        strategy: "unsupported",
        released: false,
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn release_unused_pages_mimalloc() -> PageReleaseResult {
    unsafe { libmimalloc_sys::mi_collect(true) };
    PageReleaseResult {
        strategy: "mimalloc_collect",
        released: true,
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn mach_task_self() -> libc::mach_port_t;
    fn task_info(
        target_task: libc::mach_port_t,
        flavor: libc::c_int,
        task_info_out: *mut libc::integer_t,
        task_info_out_count: *mut libc::mach_msg_type_number_t,
    ) -> libc::c_int;
}

#[cfg(target_os = "macos")]
const KERN_SUCCESS: libc::c_int = 0;

#[cfg(target_os = "macos")]
const TASK_VM_INFO: libc::c_int = 22;

#[cfg(target_os = "macos")]
const TASK_VM_INFO_REV1_COUNT: libc::mach_msg_type_number_t =
    (std::mem::size_of::<TaskVmInfoRev1>() / std::mem::size_of::<libc::integer_t>())
        as libc::mach_msg_type_number_t;

#[cfg(target_os = "macos")]
#[repr(C)]
struct TaskVmInfoRev1 {
    _virtual_size: u64,
    _region_count: libc::integer_t,
    _page_size: libc::integer_t,
    resident_size: u64,
    _resident_size_peak: u64,
    _device: u64,
    _device_peak: u64,
    _internal: u64,
    _internal_peak: u64,
    _external: u64,
    _external_peak: u64,
    _reusable: u64,
    _reusable_peak: u64,
    _purgeable_volatile_pmap: u64,
    _purgeable_volatile_resident: u64,
    _purgeable_volatile_virtual: u64,
    _compressed: u64,
    _compressed_peak: u64,
    _compressed_lifetime: u64,
    phys_footprint: u64,
}

#[cfg(test)]
mod tests {
    use super::{
        PageReleaseResult, ProcessMemorySnapshot, capture_process_memory_unsupported,
        release_unused_pages_unsupported,
    };

    #[test]
    fn unsupported_memory_capture_is_best_effort_none() {
        assert_eq!(capture_process_memory_unsupported(), None);
    }

    #[test]
    fn unsupported_page_release_is_noop() {
        assert_eq!(
            release_unused_pages_unsupported(),
            PageReleaseResult {
                strategy: "unsupported",
                released: false,
            }
        );
    }

    #[test]
    fn process_memory_snapshot_allows_partial_values() {
        let snapshot = ProcessMemorySnapshot {
            resident_bytes: Some(1024),
            phys_footprint_bytes: None,
        };
        assert_eq!(snapshot.resident_bytes, Some(1024));
        assert_eq!(snapshot.phys_footprint_bytes, None);
    }

    #[test]
    fn page_release_result_preserves_strategy_and_release_flag() {
        let result = PageReleaseResult {
            strategy: "mimalloc_collect",
            released: true,
        };

        assert_eq!(result.strategy, "mimalloc_collect");
        assert!(result.released);
    }
}
