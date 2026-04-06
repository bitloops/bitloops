use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;

use super::gateways::{HostServicesGateway, LanguageServicesGateway};
use crate::host::devql::RelationalStorage;

#[derive(Debug, Clone)]
pub enum HostEvent {
    SyncCompleted(SyncCompletedPayload),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostEventKind {
    SyncCompleted,
}

impl HostEvent {
    pub fn kind(&self) -> HostEventKind {
        match self {
            HostEvent::SyncCompleted(_) => HostEventKind::SyncCompleted,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyncCompletedPayload {
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub active_branch: Option<String>,
    pub head_commit_sha: Option<String>,
    pub sync_mode: String,
    pub sync_completed_at: String,
    pub files: SyncFileDiff,
    pub artefacts: SyncArtefactDiff,
}

#[derive(Debug, Clone, Default)]
pub struct SyncFileDiff {
    pub added: Vec<ChangedFile>,
    pub changed: Vec<ChangedFile>,
    pub removed: Vec<RemovedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub language: String,
    pub content_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedFile {
    pub path: String,
}

#[derive(Debug, Clone, Default)]
pub struct SyncArtefactDiff {
    pub added: Vec<ChangedArtefact>,
    pub changed: Vec<ChangedArtefact>,
    pub removed: Vec<RemovedArtefact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedArtefact {
    pub artefact_id: String,
    pub symbol_id: String,
    pub path: String,
    pub canonical_kind: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedArtefact {
    pub artefact_id: String,
    pub symbol_id: String,
    pub path: String,
}

#[derive(Clone)]
pub struct EventHandlerContext {
    pub storage: Arc<RelationalStorage>,
    pub language_services: Arc<dyn LanguageServicesGateway>,
    pub host_services: Arc<dyn HostServicesGateway>,
}

pub type EventHandlerFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

pub trait HostEventHandler: Send + Sync {
    fn event_kind(&self) -> HostEventKind;
    fn capability_id(&self) -> &str;
    fn handle<'a>(
        &'a self,
        event: &'a HostEvent,
        context: &'a EventHandlerContext,
    ) -> EventHandlerFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sync_payload() -> SyncCompletedPayload {
        SyncCompletedPayload {
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo"),
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("abc123".to_string()),
            sync_mode: "full".to_string(),
            sync_completed_at: "2026-04-06T00:00:00Z".to_string(),
            files: SyncFileDiff::default(),
            artefacts: SyncArtefactDiff::default(),
        }
    }

    #[test]
    fn host_event_kind_matches_variant() {
        let event = HostEvent::SyncCompleted(sync_payload());
        assert_eq!(event.kind(), HostEventKind::SyncCompleted);
    }
}
