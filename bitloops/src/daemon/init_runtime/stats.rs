use std::collections::{BTreeMap, BTreeSet};

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::runtime_presentation::{
    RETRY_FAILED_ENRICHMENTS_COMMAND, mailbox_label, workplane_warning_message,
};

use super::types::{InitRuntimeLaneProgressView, InitRuntimeLaneWarningView};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StatusCounts {
    pub(crate) pending: u64,
    pub(crate) running: u64,
    pub(crate) failed: u64,
    pub(crate) completed: u64,
}

impl StatusCounts {
    pub(crate) fn queued(self) -> u64 {
        self.pending
    }

    pub(crate) fn has_pending_or_running(self) -> bool {
        self.pending > 0 || self.running > 0
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct SessionWorkplaneStats {
    pub(crate) current_state: StatusCounts,
    pub(crate) embedding_jobs: StatusCounts,
    pub(crate) summary_jobs: StatusCounts,
    pub(crate) code_embedding_jobs: SessionMailboxStats,
    pub(crate) summary_embedding_jobs: SessionMailboxStats,
    pub(crate) clone_rebuild_jobs: SessionMailboxStats,
    pub(crate) summary_refresh_jobs: SessionMailboxStats,
    pub(crate) failed_current_state_detail: Option<String>,
    pub(crate) blocked_code_embedding_reason: Option<String>,
    pub(crate) blocked_summary_embedding_reason: Option<String>,
    pub(crate) blocked_summary_reason: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct SessionMailboxStats {
    pub(crate) counts: StatusCounts,
    pub(crate) latest_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeLaneProgressState {
    pub(crate) code_embeddings: Option<InitRuntimeLaneProgressView>,
    pub(crate) summaries: Option<InitRuntimeLaneProgressView>,
    pub(crate) summary_embeddings: Option<InitRuntimeLaneProgressView>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SummaryInMemoryBatchProgress {
    pub(crate) repo_id: String,
    pub(crate) artefact_ids_by_session: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SummaryFreshnessState {
    pub(crate) eligible_artefact_ids: BTreeSet<String>,
    pub(crate) fresh_model_backed_artefact_ids: BTreeSet<String>,
}

impl SummaryFreshnessState {
    pub(crate) fn artefact_needs_refresh(&self, artefact_id: &str) -> bool {
        self.eligible_artefact_ids.contains(artefact_id)
            && !self.fresh_model_backed_artefact_ids.contains(artefact_id)
    }

    pub(crate) fn outstanding_work_item_count(&self) -> u64 {
        self.eligible_artefact_ids
            .difference(&self.fresh_model_backed_artefact_ids)
            .count() as u64
    }

    pub(crate) fn outstanding_work_item_count_for_artefacts(&self, artefact_ids: &[String]) -> u64 {
        artefact_ids
            .iter()
            .filter(|artefact_id| self.artefact_needs_refresh(artefact_id.as_str()))
            .count() as u64
    }
}

impl SessionWorkplaneStats {
    pub(crate) fn refresh_lane_counts(&mut self) {
        self.summary_jobs = self.summary_refresh_jobs.counts;
        self.embedding_jobs = merge_status_counts([
            self.code_embedding_jobs.counts,
            self.summary_embedding_jobs.counts,
        ]);
    }

    pub(crate) fn warning_failed_jobs_total(&self) -> u64 {
        self.code_embedding_jobs.counts.failed
            + self.summary_embedding_jobs.counts.failed
            + self.summary_refresh_jobs.counts.failed
    }

    pub(crate) fn summary_warnings(&self) -> Vec<InitRuntimeLaneWarningView> {
        mailbox_warning(
            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            &self.summary_refresh_jobs,
        )
        .into_iter()
        .collect()
    }

    pub(crate) fn code_embedding_warnings(&self) -> Vec<InitRuntimeLaneWarningView> {
        mailbox_warning(
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            &self.code_embedding_jobs,
        )
        .into_iter()
        .collect()
    }

    pub(crate) fn summary_embedding_warnings(&self) -> Vec<InitRuntimeLaneWarningView> {
        mailbox_warning(
            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            &self.summary_embedding_jobs,
        )
        .into_iter()
        .collect()
    }
}

fn mailbox_warning(
    mailbox_name: &str,
    mailbox: &SessionMailboxStats,
) -> Option<InitRuntimeLaneWarningView> {
    (mailbox.counts.failed > 0).then(|| InitRuntimeLaneWarningView {
        component_label: mailbox_label(mailbox_name).to_string(),
        message: workplane_warning_message(mailbox.counts.failed, mailbox.latest_error.as_deref()),
        retry_command: RETRY_FAILED_ENRICHMENTS_COMMAND.to_string(),
    })
}

pub(crate) fn merge_status_counts<const N: usize>(counts: [StatusCounts; N]) -> StatusCounts {
    counts
        .into_iter()
        .fold(StatusCounts::default(), |mut acc, counts| {
            acc.pending += counts.pending;
            acc.running += counts.running;
            acc.failed += counts.failed;
            acc.completed += counts.completed;
            acc
        })
}

pub(crate) fn mailbox_stats_mut<'a>(
    stats: &'a mut SessionWorkplaneStats,
    mailbox_name: &str,
) -> &'a mut SessionMailboxStats {
    match mailbox_name {
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX => &mut stats.summary_refresh_jobs,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX | SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX => {
            &mut stats.code_embedding_jobs
        }
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => &mut stats.summary_embedding_jobs,
        SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX => &mut stats.clone_rebuild_jobs,
        _ => &mut stats.clone_rebuild_jobs,
    }
}

pub(crate) fn semantic_embedding_mailbox_name_for_representation(
    representation_kind: &str,
) -> &'static str {
    if representation_kind.eq_ignore_ascii_case(&EmbeddingRepresentationKind::Summary.to_string()) {
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
    } else if EmbeddingRepresentationKind::Identity
        .storage_values()
        .iter()
        .any(|value| representation_kind.eq_ignore_ascii_case(value))
    {
        SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX
    } else {
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
    }
}

pub(crate) fn semantic_embedding_representation_kind_for_mailbox(
    mailbox_name: &str,
) -> &'static str {
    if mailbox_name == SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX {
        "summary"
    } else if mailbox_name == SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX {
        "identity"
    } else {
        "code"
    }
}

#[cfg(test)]
pub(crate) fn is_summary_mailbox(mailbox_name: &str) -> bool {
    mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
}
