use std::time::Duration;

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
    pub(crate) gc: Duration,
    pub(crate) sqlite_commits: usize,
    pub(crate) sqlite_rows_written: usize,
    pub(crate) prepare_worker_count: usize,
}

impl SyncExecutionStats {
    pub(crate) fn log(&self, repo_id: &str, mode: &str) {
        log::info!(
            "DevQL sync stats for repo `{repo_id}` mode `{mode}`: workspace={}ms manifest={}ms stored={}ms cache_lookup={}ms extraction={}ms prep={}ms cache_store={}ms materialisation={}ms gc={}ms sqlite_commits={} sqlite_rows_written={} workers={}",
            self.workspace_inspection.as_millis(),
            self.desired_manifest_build.as_millis(),
            self.stored_manifest_load.as_millis(),
            self.cache_lookup_total.as_millis(),
            self.extraction_total.as_millis(),
            self.materialisation_prep_total.as_millis(),
            self.cache_store_total.as_millis(),
            self.materialisation_total.as_millis(),
            self.gc.as_millis(),
            self.sqlite_commits,
            self.sqlite_rows_written,
            self.prepare_worker_count,
        );
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
