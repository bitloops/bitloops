use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SyncPhaseTelemetry {
    pub(crate) phase_name: Option<String>,
    pub(crate) reconcile_mode: Option<String>,
    pub(crate) files: usize,
    pub(crate) artefacts: usize,
    pub(crate) edges: usize,
    pub(crate) facts: usize,
    pub(crate) transaction_count: usize,
    pub(crate) max_rss_kb: u64,
    pub(crate) max_sqlite_lock_wait_ms: u64,
    pub(crate) max_sqlite_lock_hold_ms: u64,
}

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
    pub(crate) sync_finalization: SyncPhaseTelemetry,
    pub(crate) current_edge_reconcile: SyncPhaseTelemetry,
}

impl SyncExecutionStats {
    pub(crate) fn log(&self, repo_id: &str, mode: &str) {
        log::info!(
            "DevQL sync stats for repo `{repo_id}` mode `{mode}`: workspace={}ms manifest={}ms stored={}ms cache_lookup={}ms extraction={}ms prep={}ms cache_store={}ms materialisation={}ms current_edge_reconcile={}ms capability_event_enqueue={}ms gc={}ms sqlite_commits={} sqlite_rows_written={} workers={} sync_finalization_phase={} sync_finalization_reconcile_mode={} sync_finalization_files={} sync_finalization_artefacts={} sync_finalization_edges={} sync_finalization_facts={} sync_finalization_transaction_count={} sync_finalization_max_rss_kb={} sync_finalization_max_sqlite_lock_wait_ms={} sync_finalization_max_sqlite_lock_hold_ms={} current_edge_reconcile_phase={} current_edge_reconcile_reconcile_mode={} current_edge_reconcile_files={} current_edge_reconcile_artefacts={} current_edge_reconcile_edges={} current_edge_reconcile_facts={} current_edge_reconcile_transaction_count={} current_edge_reconcile_max_rss_kb={} current_edge_reconcile_max_sqlite_lock_wait_ms={} current_edge_reconcile_max_sqlite_lock_hold_ms={}",
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
            self.sync_finalization.phase_name.as_deref().unwrap_or("-"),
            self.sync_finalization
                .reconcile_mode
                .as_deref()
                .unwrap_or("-"),
            self.sync_finalization.files,
            self.sync_finalization.artefacts,
            self.sync_finalization.edges,
            self.sync_finalization.facts,
            self.sync_finalization.transaction_count,
            self.sync_finalization.max_rss_kb,
            self.sync_finalization.max_sqlite_lock_wait_ms,
            self.sync_finalization.max_sqlite_lock_hold_ms,
            self.current_edge_reconcile
                .phase_name
                .as_deref()
                .unwrap_or("-"),
            self.current_edge_reconcile
                .reconcile_mode
                .as_deref()
                .unwrap_or("-"),
            self.current_edge_reconcile.files,
            self.current_edge_reconcile.artefacts,
            self.current_edge_reconcile.edges,
            self.current_edge_reconcile.facts,
            self.current_edge_reconcile.transaction_count,
            self.current_edge_reconcile.max_rss_kb,
            self.current_edge_reconcile.max_sqlite_lock_wait_ms,
            self.current_edge_reconcile.max_sqlite_lock_hold_ms,
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

    pub(crate) fn record_sync_finalization(
        &mut self,
        reconcile_mode: &str,
        files: usize,
        outcome: &super::sqlite_writer::WriterCommitOutcome,
    ) {
        self.sync_finalization.phase_name =
            outcome.sqlite_phase_metrics.phase_name.map(str::to_string);
        self.sync_finalization.reconcile_mode = Some(reconcile_mode.to_string());
        self.sync_finalization.files = files;
        self.sync_finalization.transaction_count = outcome.sqlite_phase_metrics.transaction_count;
        self.sync_finalization.max_rss_kb = outcome.max_rss_kb;
        self.sync_finalization.max_sqlite_lock_wait_ms = outcome.sqlite_phase_metrics.max_wait_ms;
        self.sync_finalization.max_sqlite_lock_hold_ms = outcome.sqlite_phase_metrics.max_hold_ms;
    }

    pub(crate) fn record_current_edge_reconcile(
        &mut self,
        reconcile_mode: &str,
        files: usize,
        edges: usize,
        outcome: &crate::host::devql::sync::materializer::CurrentEdgeReconcileOutcome,
    ) {
        self.current_edge_reconcile.phase_name =
            outcome.sqlite_phase_metrics.phase_name.map(str::to_string);
        self.current_edge_reconcile.reconcile_mode = Some(reconcile_mode.to_string());
        self.current_edge_reconcile.files = files;
        self.current_edge_reconcile.edges = edges;
        self.current_edge_reconcile.transaction_count =
            outcome.sqlite_phase_metrics.transaction_count;
        self.current_edge_reconcile.max_rss_kb = outcome.max_rss_kb;
        self.current_edge_reconcile.max_sqlite_lock_wait_ms =
            outcome.sqlite_phase_metrics.max_wait_ms;
        self.current_edge_reconcile.max_sqlite_lock_hold_ms =
            outcome.sqlite_phase_metrics.max_hold_ms;
    }
}
