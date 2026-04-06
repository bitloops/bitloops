use std::sync::Arc;

use futures_util::FutureExt;

use super::events::{EventHandlerContext, HostEvent, HostEventHandler};

/// Dispatch a host event to all registered handlers matching its kind.
/// Handlers run in parallel and are not awaited.
pub fn dispatch_event(
    event: HostEvent,
    handlers: &[Arc<dyn HostEventHandler>],
    context: Arc<EventHandlerContext>,
) {
    let target_kind = event.kind();
    let event = Arc::new(event);

    for handler in handlers {
        if handler.event_kind() != target_kind {
            continue;
        }

        let handler = Arc::clone(handler);
        let event = Arc::clone(&event);
        let context = Arc::clone(&context);
        let capability_id = handler.capability_id().to_string();

        tokio::spawn(async move {
            let outcome = std::panic::AssertUnwindSafe(handler.handle(&event, &context))
                .catch_unwind()
                .await;
            match outcome {
                Ok(Ok(())) => {
                    log::debug!(
                        "capability event handler succeeded (capability_id={}, event_kind={:?})",
                        capability_id,
                        target_kind
                    );
                }
                Ok(Err(err)) => {
                    log::warn!(
                        "capability event handler failed (capability_id={}, event_kind={:?}): {err:#}",
                        capability_id,
                        target_kind
                    );
                }
                Err(_) => {
                    log::error!(
                        "capability event handler panicked (capability_id={}, event_kind={:?})",
                        capability_id,
                        target_kind
                    );
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;
    use crate::host::capability_host::events::{
        ChangedArtefact, ChangedFile, EventHandlerFuture, HostEventKind, RemovedArtefact,
        RemovedFile, SyncArtefactDiff, SyncCompletedPayload, SyncFileDiff,
    };
    use crate::host::capability_host::gateways::{
        DefaultHostServicesGateway, EmptyLanguageServicesGateway,
    };
    use crate::host::devql::RelationalStorage;

    struct CountingHandler {
        count: Arc<AtomicUsize>,
    }

    impl HostEventHandler for CountingHandler {
        fn event_kind(&self) -> HostEventKind {
            HostEventKind::SyncCompleted
        }

        fn capability_id(&self) -> &str {
            "counting-pack"
        }

        fn handle<'a>(
            &'a self,
            _event: &'a HostEvent,
            _context: &'a EventHandlerContext,
        ) -> EventHandlerFuture<'a> {
            Box::pin(async move {
                self.count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    struct FailingHandler;

    impl HostEventHandler for FailingHandler {
        fn event_kind(&self) -> HostEventKind {
            HostEventKind::SyncCompleted
        }

        fn capability_id(&self) -> &str {
            "failing-pack"
        }

        fn handle<'a>(
            &'a self,
            _event: &'a HostEvent,
            _context: &'a EventHandlerContext,
        ) -> EventHandlerFuture<'a> {
            Box::pin(async { anyhow::bail!("intentional failure") })
        }
    }

    struct PayloadCaptureHandler {
        captured: Arc<Mutex<Option<SyncCompletedPayload>>>,
    }

    impl HostEventHandler for PayloadCaptureHandler {
        fn event_kind(&self) -> HostEventKind {
            HostEventKind::SyncCompleted
        }

        fn capability_id(&self) -> &str {
            "payload-capture-pack"
        }

        fn handle<'a>(
            &'a self,
            event: &'a HostEvent,
            _context: &'a EventHandlerContext,
        ) -> EventHandlerFuture<'a> {
            Box::pin(async move {
                let HostEvent::SyncCompleted(payload) = event;
                let mut guard = self
                    .captured
                    .lock()
                    .expect("payload capture mutex should not be poisoned");
                *guard = Some(payload.clone());
                Ok(())
            })
        }
    }

    fn test_context() -> Arc<EventHandlerContext> {
        Arc::new(EventHandlerContext {
            storage: Arc::new(RelationalStorage::local_only(
                std::env::temp_dir().join("bitloops-sync-event-dispatch-tests.sqlite"),
            )),
            language_services: Arc::new(EmptyLanguageServicesGateway),
            host_services: Arc::new(DefaultHostServicesGateway::new("repo-1")),
        })
    }

    fn test_event() -> HostEvent {
        HostEvent::SyncCompleted(SyncCompletedPayload {
            repo_id: "repo-1".to_string(),
            repo_root: std::env::temp_dir(),
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("abc123".to_string()),
            sync_mode: "full".to_string(),
            sync_completed_at: "2026-04-06T00:00:00Z".to_string(),
            files: SyncFileDiff::default(),
            artefacts: SyncArtefactDiff::default(),
        })
    }

    #[tokio::test]
    async fn dispatch_calls_matching_handlers() {
        let count = Arc::new(AtomicUsize::new(0));
        let handlers: Vec<Arc<dyn HostEventHandler>> = vec![
            Arc::new(CountingHandler {
                count: Arc::clone(&count),
            }),
            Arc::new(CountingHandler {
                count: Arc::clone(&count),
            }),
        ];

        dispatch_event(test_event(), &handlers, test_context());
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn dispatch_isolates_handler_failures() {
        let count = Arc::new(AtomicUsize::new(0));
        let handlers: Vec<Arc<dyn HostEventHandler>> = vec![
            Arc::new(FailingHandler),
            Arc::new(CountingHandler {
                count: Arc::clone(&count),
            }),
        ];

        dispatch_event(test_event(), &handlers, test_context());
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dispatch_passes_sync_completed_payload_to_handler() {
        let captured = Arc::new(Mutex::new(None));
        let handlers: Vec<Arc<dyn HostEventHandler>> = vec![Arc::new(PayloadCaptureHandler {
            captured: Arc::clone(&captured),
        })];

        let event = HostEvent::SyncCompleted(SyncCompletedPayload {
            repo_id: "repo-42".to_string(),
            repo_root: std::env::temp_dir(),
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("deadbeef".to_string()),
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
        });

        dispatch_event(event, &handlers, test_context());
        tokio::time::sleep(Duration::from_millis(50)).await;

        let payload = captured
            .lock()
            .expect("payload capture mutex should not be poisoned")
            .clone()
            .expect("handler should capture SyncCompleted payload");
        assert_eq!(payload.repo_id, "repo-42");
        assert_eq!(payload.sync_mode, "full");
        assert_eq!(payload.files.added.len(), 1);
        assert_eq!(payload.files.changed.len(), 1);
        assert_eq!(payload.files.removed.len(), 1);
        assert_eq!(payload.artefacts.added.len(), 1);
        assert_eq!(payload.artefacts.changed.len(), 1);
        assert_eq!(payload.artefacts.removed.len(), 1);
        assert_eq!(payload.artefacts.changed[0].name, "changed_fn");
    }
}
