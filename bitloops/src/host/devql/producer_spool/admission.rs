use crate::daemon::{DevqlTaskKind, DevqlTaskSource, DevqlTaskSpec};

use super::ProducerSpoolJobPayload;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProducerSpoolAdmissionClass {
    PromoteVisibleTask { kind: DevqlTaskKind },
    ExpandVisibleTask { kind: DevqlTaskKind },
    ExpandVisibleTasks { kinds: &'static [DevqlTaskKind] },
    InlineRepoExclusive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProducerSpoolRunningTask {
    pub(crate) repo_id: String,
    pub(crate) kind: DevqlTaskKind,
    pub(crate) source: DevqlTaskSource,
}

impl ProducerSpoolRunningTask {
    pub(crate) fn new(
        repo_id: impl Into<String>,
        kind: DevqlTaskKind,
        source: DevqlTaskSource,
    ) -> Self {
        Self {
            repo_id: repo_id.into(),
            kind,
            source,
        }
    }
}

pub(crate) fn producer_spool_admission_class(
    payload: &ProducerSpoolJobPayload,
) -> ProducerSpoolAdmissionClass {
    match payload {
        ProducerSpoolJobPayload::Task { spec, .. } => {
            ProducerSpoolAdmissionClass::PromoteVisibleTask {
                kind: task_kind_from_spec(spec),
            }
        }
        ProducerSpoolJobPayload::PostCommitRefresh { .. }
        | ProducerSpoolJobPayload::PostMergeSyncRefresh { .. } => {
            ProducerSpoolAdmissionClass::ExpandVisibleTask {
                kind: DevqlTaskKind::Sync,
            }
        }
        ProducerSpoolJobPayload::PostMergeIngestBackfill { .. } => {
            ProducerSpoolAdmissionClass::ExpandVisibleTask {
                kind: DevqlTaskKind::Ingest,
            }
        }
        ProducerSpoolJobPayload::PostMergeRefresh { .. } => {
            ProducerSpoolAdmissionClass::ExpandVisibleTasks {
                kinds: &[DevqlTaskKind::Sync, DevqlTaskKind::Ingest],
            }
        }
        ProducerSpoolJobPayload::PostCommitDerivation { .. }
        | ProducerSpoolJobPayload::PrePushSync { .. } => {
            ProducerSpoolAdmissionClass::InlineRepoExclusive
        }
    }
}

pub(crate) fn producer_spool_payload_conflicts_with_running_task(
    payload: &ProducerSpoolJobPayload,
    job_repo_id: &str,
    running_task: &ProducerSpoolRunningTask,
) -> bool {
    if running_task.repo_id != job_repo_id {
        return false;
    }
    if running_task.kind == DevqlTaskKind::Sync
        && running_task.source == DevqlTaskSource::RepoPolicyChange
    {
        return true;
    }

    match producer_spool_admission_class(payload) {
        ProducerSpoolAdmissionClass::PromoteVisibleTask { kind }
        | ProducerSpoolAdmissionClass::ExpandVisibleTask { kind } => running_task.kind == kind,
        ProducerSpoolAdmissionClass::ExpandVisibleTasks { kinds } => {
            kinds.contains(&running_task.kind)
        }
        ProducerSpoolAdmissionClass::InlineRepoExclusive => true,
    }
}

fn task_kind_from_spec(spec: &DevqlTaskSpec) -> DevqlTaskKind {
    match spec {
        DevqlTaskSpec::Sync(_) => DevqlTaskKind::Sync,
        DevqlTaskSpec::Ingest(_) => DevqlTaskKind::Ingest,
        DevqlTaskSpec::EmbeddingsBootstrap(_) => DevqlTaskKind::EmbeddingsBootstrap,
        DevqlTaskSpec::SummaryBootstrap(_) => DevqlTaskKind::SummaryBootstrap,
    }
}

#[cfg(test)]
mod tests {
    use crate::daemon::{
        DevqlTaskKind, DevqlTaskSource, DevqlTaskSpec, IngestTaskSpec, SyncTaskMode, SyncTaskSpec,
    };

    use super::*;

    fn sync_payload() -> ProducerSpoolJobPayload {
        ProducerSpoolJobPayload::Task {
            source: DevqlTaskSource::Watcher,
            spec: DevqlTaskSpec::Sync(SyncTaskSpec {
                mode: SyncTaskMode::Auto,
                post_commit_snapshot: None,
            }),
        }
    }

    fn ingest_payload() -> ProducerSpoolJobPayload {
        ProducerSpoolJobPayload::Task {
            source: DevqlTaskSource::PostCommit,
            spec: DevqlTaskSpec::Ingest(IngestTaskSpec::default()),
        }
    }

    fn pre_push_payload() -> ProducerSpoolJobPayload {
        ProducerSpoolJobPayload::PrePushSync {
            remote: "origin".to_string(),
            stdin_lines: Vec::new(),
        }
    }

    fn post_commit_refresh_payload() -> ProducerSpoolJobPayload {
        ProducerSpoolJobPayload::PostCommitRefresh {
            commit_sha: "commit-a".to_string(),
            changed_files: vec!["src/lib.rs".to_string()],
        }
    }

    fn post_merge_refresh_payload() -> ProducerSpoolJobPayload {
        ProducerSpoolJobPayload::PostMergeRefresh {
            head_sha: "commit-b".to_string(),
            changed_files: vec!["src/lib.rs".to_string()],
        }
    }

    fn post_merge_sync_payload() -> ProducerSpoolJobPayload {
        ProducerSpoolJobPayload::PostMergeSyncRefresh {
            merge_head_sha: "merge-head".to_string(),
            changed_files: vec!["src/lib.rs".to_string()],
            is_squash: false,
        }
    }

    fn post_merge_ingest_payload() -> ProducerSpoolJobPayload {
        ProducerSpoolJobPayload::PostMergeIngestBackfill {
            merge_head_sha: "merge-head".to_string(),
            is_squash: false,
        }
    }

    fn running_task(kind: DevqlTaskKind) -> ProducerSpoolRunningTask {
        ProducerSpoolRunningTask::new("repo-a", kind, DevqlTaskSource::Watcher)
    }

    #[test]
    fn producer_spool_admission_sync_payload_does_not_conflict_with_same_repo_running_ingest() {
        assert!(!producer_spool_payload_conflicts_with_running_task(
            &sync_payload(),
            "repo-a",
            &running_task(DevqlTaskKind::Ingest),
        ));
    }

    #[test]
    fn producer_spool_admission_sync_payload_conflicts_with_same_repo_running_sync() {
        assert!(producer_spool_payload_conflicts_with_running_task(
            &sync_payload(),
            "repo-a",
            &running_task(DevqlTaskKind::Sync),
        ));
    }

    #[test]
    fn producer_spool_admission_ingest_payload_does_not_conflict_with_same_repo_running_sync() {
        assert!(!producer_spool_payload_conflicts_with_running_task(
            &ingest_payload(),
            "repo-a",
            &running_task(DevqlTaskKind::Sync),
        ));
    }

    #[test]
    fn producer_spool_admission_any_payload_conflicts_with_same_repo_repo_policy_change_sync() {
        let running_task = ProducerSpoolRunningTask::new(
            "repo-a",
            DevqlTaskKind::Sync,
            DevqlTaskSource::RepoPolicyChange,
        );

        assert!(producer_spool_payload_conflicts_with_running_task(
            &ingest_payload(),
            "repo-a",
            &running_task,
        ));
    }

    #[test]
    fn producer_spool_admission_inline_payload_conflicts_with_any_same_repo_running_task() {
        assert!(producer_spool_payload_conflicts_with_running_task(
            &pre_push_payload(),
            "repo-a",
            &running_task(DevqlTaskKind::EmbeddingsBootstrap),
        ));
    }

    #[test]
    fn producer_spool_admission_post_commit_refresh_conflicts_with_same_repo_running_sync() {
        assert!(producer_spool_payload_conflicts_with_running_task(
            &post_commit_refresh_payload(),
            "repo-a",
            &running_task(DevqlTaskKind::Sync),
        ));
    }

    #[test]
    fn producer_spool_admission_post_merge_sync_conflicts_with_same_repo_running_sync() {
        assert!(producer_spool_payload_conflicts_with_running_task(
            &post_merge_sync_payload(),
            "repo-a",
            &running_task(DevqlTaskKind::Sync),
        ));
    }

    #[test]
    fn producer_spool_admission_post_merge_sync_does_not_conflict_with_running_ingest() {
        assert!(!producer_spool_payload_conflicts_with_running_task(
            &post_merge_sync_payload(),
            "repo-a",
            &running_task(DevqlTaskKind::Ingest),
        ));
    }

    #[test]
    fn producer_spool_admission_post_merge_ingest_conflicts_with_same_repo_running_ingest() {
        assert!(producer_spool_payload_conflicts_with_running_task(
            &post_merge_ingest_payload(),
            "repo-a",
            &running_task(DevqlTaskKind::Ingest),
        ));
    }

    #[test]
    fn producer_spool_admission_post_merge_ingest_does_not_conflict_with_running_sync() {
        assert!(!producer_spool_payload_conflicts_with_running_task(
            &post_merge_ingest_payload(),
            "repo-a",
            &running_task(DevqlTaskKind::Sync),
        ));
    }

    #[test]
    fn producer_spool_admission_legacy_post_merge_refresh_stays_conservative() {
        assert!(producer_spool_payload_conflicts_with_running_task(
            &post_merge_refresh_payload(),
            "repo-a",
            &running_task(DevqlTaskKind::Sync),
        ));
        assert!(producer_spool_payload_conflicts_with_running_task(
            &post_merge_refresh_payload(),
            "repo-a",
            &running_task(DevqlTaskKind::Ingest),
        ));
    }

    #[test]
    fn producer_spool_admission_running_task_for_another_repo_does_not_conflict() {
        let running_task =
            ProducerSpoolRunningTask::new("repo-b", DevqlTaskKind::Sync, DevqlTaskSource::Watcher);

        assert!(!producer_spool_payload_conflicts_with_running_task(
            &pre_push_payload(),
            "repo-a",
            &running_task,
        ));
    }
}
