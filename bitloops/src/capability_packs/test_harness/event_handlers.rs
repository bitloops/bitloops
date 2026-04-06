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
            log::debug!(
                "received SyncCompleted event for test harness (repo_id={}, files_added={}, files_changed={}, files_removed={}, artefacts_added={}, artefacts_changed={}, artefacts_removed={})",
                payload.repo_id,
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
