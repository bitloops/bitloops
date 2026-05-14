use std::env;
#[cfg(target_os = "linux")]
use std::fs;
use std::time::Duration;

pub(crate) const DEVQL_SYNC_MEMORY_ENV: &str = "BITLOOPS_DEVQL_SYNC_MEMORY";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SyncExecutionStats {
    pub(crate) workspace_inspection: Duration,
    pub(crate) desired_manifest_build: Duration,
    pub(crate) stored_manifest_load: Duration,
    pub(crate) cache_lookup_total: Duration,
    pub(crate) extraction_total: Duration,
    pub(crate) materialisation_prep_total: Duration,
    pub(crate) cache_store_total: Duration,
    pub(crate) materialisation_total: Duration,
    pub(crate) current_edge_reconcile_total: Duration,
    pub(crate) capability_event_enqueue_total: Duration,
    pub(crate) gc: Duration,
    pub(crate) sqlite_commits: usize,
    pub(crate) sqlite_rows_written: usize,
    pub(crate) prepare_worker_count: usize,
    pub(crate) memory_snapshots: Vec<SyncMemorySnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyncMemorySnapshot {
    pub(crate) checkpoint: String,
    pub(crate) resident_bytes: u64,
    pub(crate) physical_footprint_bytes: Option<u64>,
}

impl SyncExecutionStats {
    pub(crate) fn log(&self, repo_id: &str, mode: &str) {
        log::info!(
            "DevQL sync stats for repo `{repo_id}` mode `{mode}`: workspace={}ms manifest={}ms stored={}ms cache_lookup={}ms extraction={}ms prep={}ms cache_store={}ms materialisation={}ms current_edge_reconcile={}ms capability_event_enqueue={}ms gc={}ms sqlite_commits={} sqlite_rows_written={} workers={}",
            self.workspace_inspection.as_millis(),
            self.desired_manifest_build.as_millis(),
            self.stored_manifest_load.as_millis(),
            self.cache_lookup_total.as_millis(),
            self.extraction_total.as_millis(),
            self.materialisation_prep_total.as_millis(),
            self.cache_store_total.as_millis(),
            self.materialisation_total.as_millis(),
            self.current_edge_reconcile_total.as_millis(),
            self.capability_event_enqueue_total.as_millis(),
            self.gc.as_millis(),
            self.sqlite_commits,
            self.sqlite_rows_written,
            self.prepare_worker_count,
        );
        if let Some(summary) = self.memory_trace_summary(repo_id, mode) {
            log::info!("{summary}");
        }
    }

    pub(crate) fn maybe_record_memory_snapshot(&mut self, repo_id: &str, checkpoint: &str) {
        self.record_memory_snapshot(repo_id, checkpoint, false);
    }

    pub(crate) fn maybe_record_peak_memory_snapshot(&mut self, repo_id: &str, checkpoint: &str) {
        self.record_memory_snapshot(repo_id, checkpoint, true);
    }

    fn record_memory_snapshot(&mut self, repo_id: &str, checkpoint: &str, only_if_new_peak: bool) {
        if !sync_memory_trace_enabled() {
            return;
        }
        let Some(snapshot) = capture_current_process_memory().map(|memory| {
            SyncMemorySnapshot::new(
                checkpoint,
                memory.resident_bytes,
                memory.physical_footprint_bytes,
            )
        }) else {
            return;
        };
        if only_if_new_peak && !self.snapshot_sets_new_peak(&snapshot) {
            return;
        }

        log::info!(
            "DevQL sync memory snapshot for repo `{repo_id}` checkpoint `{}`: rss={} footprint={}",
            snapshot.checkpoint,
            format_bytes(snapshot.resident_bytes),
            snapshot
                .physical_footprint_bytes
                .map(format_bytes)
                .unwrap_or_else(|| "n/a".to_string()),
        );
        self.memory_snapshots.push(snapshot);
    }

    fn snapshot_sets_new_peak(&self, snapshot: &SyncMemorySnapshot) -> bool {
        if self.memory_snapshots.is_empty() {
            return true;
        }

        snapshot.resident_bytes > self.peak_resident_bytes()
            || snapshot.physical_footprint_bytes.unwrap_or(0)
                > self.peak_physical_footprint_bytes().unwrap_or(0)
    }

    fn peak_resident_bytes(&self) -> u64 {
        self.memory_snapshots
            .iter()
            .map(|snapshot| snapshot.resident_bytes)
            .max()
            .unwrap_or(0)
    }

    fn peak_physical_footprint_bytes(&self) -> Option<u64> {
        self.memory_snapshots
            .iter()
            .filter_map(|snapshot| snapshot.physical_footprint_bytes)
            .max()
    }

    fn memory_trace_summary(&self, repo_id: &str, mode: &str) -> Option<String> {
        if self.memory_snapshots.is_empty() {
            return None;
        }

        let checkpoints = self
            .memory_snapshots
            .iter()
            .map(SyncMemorySnapshot::summary_fragment)
            .collect::<Vec<_>>()
            .join(", ");
        let rss_peak = format_bytes(self.peak_resident_bytes());
        let footprint_peak = self
            .peak_physical_footprint_bytes()
            .map(format_bytes)
            .unwrap_or_else(|| "n/a".to_string());

        Some(format!(
            "DevQL sync memory trace for repo `{repo_id}` mode `{mode}`: rss_peak={rss_peak} footprint_peak={footprint_peak} checkpoints=[{checkpoints}]"
        ))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PreparedPathStats {
    pub(crate) cache_lookup: Duration,
    pub(crate) extraction: Duration,
    pub(crate) materialisation_prep: Duration,
}

impl SyncExecutionStats {
    pub(crate) fn add_prepared_path(&mut self, stats: &PreparedPathStats) {
        self.cache_lookup_total += stats.cache_lookup;
        self.extraction_total += stats.extraction;
        self.materialisation_prep_total += stats.materialisation_prep;
    }

    pub(crate) fn add_writer_commit(&mut self, commits: usize, rows_written: usize) {
        self.sqlite_commits += commits;
        self.sqlite_rows_written += rows_written;
    }
}

impl SyncMemorySnapshot {
    pub(crate) fn new(
        checkpoint: impl Into<String>,
        resident_bytes: u64,
        physical_footprint_bytes: Option<u64>,
    ) -> Self {
        Self {
            checkpoint: checkpoint.into(),
            resident_bytes,
            physical_footprint_bytes,
        }
    }

    fn summary_fragment(&self) -> String {
        format!(
            "{}:rss={} footprint={}",
            self.checkpoint,
            format_bytes(self.resident_bytes),
            self.physical_footprint_bytes
                .map(format_bytes)
                .unwrap_or_else(|| "n/a".to_string()),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessMemorySnapshot {
    resident_bytes: u64,
    physical_footprint_bytes: Option<u64>,
}

fn sync_memory_trace_enabled() -> bool {
    sync_memory_trace_enabled_from_flag(env::var(DEVQL_SYNC_MEMORY_ENV).ok().as_deref())
}

fn sync_memory_trace_enabled_from_flag(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "no" | "off"
    )
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes / KIB)
    } else {
        format!("{} B", bytes as u64)
    }
}

fn capture_current_process_memory() -> Option<ProcessMemorySnapshot> {
    capture_current_process_memory_inner()
}

#[cfg(target_os = "macos")]
fn capture_current_process_memory_inner() -> Option<ProcessMemorySnapshot> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    let mut info_ptr = info.as_mut_ptr() as libc::rusage_info_t;
    let status = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as libc::c_int,
            libc::RUSAGE_INFO_V4,
            &mut info_ptr,
        )
    };
    if status != 0 {
        return None;
    }

    let info = unsafe { info.assume_init() };
    Some(ProcessMemorySnapshot {
        resident_bytes: info.ri_resident_size,
        physical_footprint_bytes: Some(info.ri_phys_footprint),
    })
}

#[cfg(target_os = "linux")]
fn capture_current_process_memory_inner() -> Option<ProcessMemorySnapshot> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let resident_kib = status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?;
        value.split_whitespace().next()?.parse::<u64>().ok()
    })?;
    Some(ProcessMemorySnapshot {
        resident_bytes: resident_kib.saturating_mul(1024),
        physical_footprint_bytes: None,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn capture_current_process_memory_inner() -> Option<ProcessMemorySnapshot> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_memory_trace_flag_treats_falsey_values_as_disabled() {
        assert!(!sync_memory_trace_enabled_from_flag(None));
        assert!(!sync_memory_trace_enabled_from_flag(Some("")));
        assert!(!sync_memory_trace_enabled_from_flag(Some("0")));
        assert!(!sync_memory_trace_enabled_from_flag(Some("false")));
        assert!(!sync_memory_trace_enabled_from_flag(Some("off")));
    }

    #[test]
    fn sync_memory_trace_flag_treats_truthy_values_as_enabled() {
        assert!(sync_memory_trace_enabled_from_flag(Some("1")));
        assert!(sync_memory_trace_enabled_from_flag(Some("true")));
        assert!(sync_memory_trace_enabled_from_flag(Some("yes")));
    }

    #[test]
    fn memory_trace_summary_reports_peaks_and_checkpoints() {
        let mut stats = SyncExecutionStats::default();
        stats.memory_snapshots = vec![
            SyncMemorySnapshot::new(
                "after_classification",
                256 * 1024 * 1024,
                Some(200 * 1024 * 1024),
            ),
            SyncMemorySnapshot::new(
                "after_materialisation_flush_peak",
                11 * 1024 * 1024 * 1024,
                Some(10 * 1024 * 1024 * 1024),
            ),
        ];

        let summary = stats
            .memory_trace_summary("repo-a", "full")
            .expect("summary");

        assert!(summary.contains("repo `repo-a`"));
        assert!(summary.contains("mode `full`"));
        assert!(summary.contains("rss_peak=11.0 GiB"));
        assert!(summary.contains("footprint_peak=10.0 GiB"));
        assert!(summary.contains("after_classification:rss=256.0 MiB footprint=200.0 MiB"));
        assert!(
            summary.contains("after_materialisation_flush_peak:rss=11.0 GiB footprint=10.0 GiB")
        );
    }
}
