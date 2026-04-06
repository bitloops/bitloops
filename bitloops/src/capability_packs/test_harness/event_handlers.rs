use crate::host::capability_host::{
    EventHandlerContext, EventHandlerFuture, HostEvent, HostEventHandler, HostEventKind,
};

use super::types::TEST_HARNESS_CAPABILITY_ID;

pub struct TestHarnessSyncHandler;

impl HostEventHandler for TestHarnessSyncHandler {
    fn event_kind(&self) -> HostEventKind {
        HostEventKind::SyncCompleted
    }

    fn capability_id(&self) -> &str {
        TEST_HARNESS_CAPABILITY_ID
    }

    fn handle<'a>(
        &'a self,
        event: &'a HostEvent,
        _context: &'a EventHandlerContext,
    ) -> EventHandlerFuture<'a> {
        Box::pin(async move {
            let HostEvent::SyncCompleted(payload) = event;
            log::info!(
                "test_harness sync event received (repo_id={}, mode={}, branch={}, files_added={}, files_changed={}, files_removed={}, artefacts_added={}, artefacts_changed={}, artefacts_removed={})",
                payload.repo_id,
                payload.sync_mode,
                payload.active_branch.as_deref().unwrap_or("unknown"),
                payload.files.added.len(),
                payload.files.changed.len(),
                payload.files.removed.len(),
                payload.artefacts.added.len(),
                payload.artefacts.changed.len(),
                payload.artefacts.removed.len()
            );
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::capability_host::{
        ChangedArtefact, ChangedFile, RemovedArtefact, RemovedFile, SyncArtefactDiff,
        SyncCompletedPayload, SyncFileDiff,
    };

    fn test_event() -> HostEvent {
        HostEvent::SyncCompleted(SyncCompletedPayload {
            repo_id: "repo-1".to_string(),
            repo_root: std::env::temp_dir(),
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("abc123".to_string()),
            sync_mode: "full".to_string(),
            sync_completed_at: "2026-04-06T00:00:00Z".to_string(),
            files: SyncFileDiff {
                added: vec![ChangedFile {
                    path: "src/new.rs".to_string(),
                    language: "rust".to_string(),
                    content_id: "blob-new".to_string(),
                }],
                changed: vec![ChangedFile {
                    path: "src/changed.rs".to_string(),
                    language: "rust".to_string(),
                    content_id: "blob-changed".to_string(),
                }],
                removed: vec![RemovedFile {
                    path: "src/old.rs".to_string(),
                }],
            },
            artefacts: SyncArtefactDiff {
                added: vec![ChangedArtefact {
                    artefact_id: "aid-add".to_string(),
                    symbol_id: "sid-add".to_string(),
                    path: "src/new.rs".to_string(),
                    canonical_kind: Some("function".to_string()),
                    name: "new_fn".to_string(),
                }],
                changed: vec![ChangedArtefact {
                    artefact_id: "aid-changed".to_string(),
                    symbol_id: "sid-changed".to_string(),
                    path: "src/changed.rs".to_string(),
                    canonical_kind: Some("function".to_string()),
                    name: "changed_fn".to_string(),
                }],
                removed: vec![RemovedArtefact {
                    artefact_id: "aid-removed".to_string(),
                    symbol_id: "sid-removed".to_string(),
                    path: "src/old.rs".to_string(),
                }],
            },
        })
    }

    #[test]
    fn handler_subscribes_to_sync_completed() {
        let handler = TestHarnessSyncHandler;
        assert_eq!(handler.event_kind(), HostEventKind::SyncCompleted);
        assert_eq!(handler.capability_id(), TEST_HARNESS_CAPABILITY_ID);
    }

    #[tokio::test]
    async fn handler_accepts_sync_completed_payload() {
        let handler = TestHarnessSyncHandler;
        let event = test_event();
        let context = EventHandlerContext {
            storage: std::sync::Arc::new(crate::host::devql::RelationalStorage::local_only(
                std::env::temp_dir().join("bitloops-test-harness-sync-handler-tests.sqlite"),
            )),
            language_services: std::sync::Arc::new(
                crate::host::capability_host::gateways::EmptyLanguageServicesGateway,
            ),
            host_services: std::sync::Arc::new(
                crate::host::capability_host::gateways::DefaultHostServicesGateway::new("repo-1"),
            ),
        };

        handler
            .handle(&event, &context)
            .await
            .expect("test harness sync handler should accept SyncCompleted payload");
    }
}
