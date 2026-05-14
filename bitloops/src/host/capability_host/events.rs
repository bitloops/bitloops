use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::gateways::{
    CapabilityWorkplaneGateway, GitHistoryGateway, HostServicesGateway, LanguageServicesGateway,
    RelationalGateway,
};
use crate::capability_packs::test_harness::storage::BitloopsTestHarnessRepository;
use crate::host::devql::RelationalStorage;
use crate::host::inference::InferenceGateway;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    pub language: String,
    pub content_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedFile {
    pub path: String,
}

#[derive(Debug, Clone, Default)]
pub struct SyncArtefactDiff {
    pub added: Vec<ChangedArtefact>,
    pub changed: Vec<ChangedArtefact>,
    pub removed: Vec<RemovedArtefact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedArtefact {
    pub artefact_id: String,
    pub symbol_id: String,
    pub path: String,
    pub canonical_kind: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedArtefact {
    pub artefact_id: String,
    pub symbol_id: String,
    pub path: String,
}

#[derive(Clone)]
pub struct CurrentStateConsumerContext {
    pub config_root: Value,
    pub storage: Arc<RelationalStorage>,
    pub relational: Arc<dyn RelationalGateway>,
    pub language_services: Arc<dyn LanguageServicesGateway>,
    pub git_history: Arc<dyn GitHistoryGateway>,
    pub inference: Arc<dyn InferenceGateway>,
    pub host_services: Arc<dyn HostServicesGateway>,
    pub workplane: Arc<dyn CapabilityWorkplaneGateway>,
    pub test_harness: Option<Arc<std::sync::Mutex<BitloopsTestHarnessRepository>>>,
    pub init_session_id: Option<String>,
    pub parent_pid: Option<u32>,
}

impl CurrentStateConsumerContext {
    pub fn ensure_parent_process_alive(&self, stage: &str) -> Result<()> {
        let Some(parent_pid) = self.parent_pid else {
            return Ok(());
        };

        if parent_process_is_running(parent_pid) {
            return Ok(());
        }

        bail!("current-state worker parent process {parent_pid} is not running while {stage}");
    }
}

pub type EventHandlerContext = CurrentStateConsumerContext;
pub type CurrentStateConsumerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<CurrentStateConsumerResult>> + Send + 'a>>;
pub type EventHandlerFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileMode {
    MergedDelta,
    FullReconcile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentStateConsumerRequest {
    pub run_id: Option<String>,
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub active_branch: Option<String>,
    pub head_commit_sha: Option<String>,
    pub from_generation_seq_exclusive: u64,
    pub to_generation_seq_inclusive: u64,
    pub reconcile_mode: ReconcileMode,
    pub file_upserts: Vec<ChangedFile>,
    pub file_removals: Vec<RemovedFile>,
    pub affected_paths: Vec<String>,
    pub artefact_upserts: Vec<ChangedArtefact>,
    pub artefact_removals: Vec<RemovedArtefact>,
}

fn parent_process_is_running(pid: u32) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args([
                "/C",
                &format!("tasklist /FI \"PID eq {pid}\" | findstr {pid}"),
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(not(windows))]
    {
        if pid > i32::MAX as u32 {
            return false;
        }

        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if result != 0
            && !matches!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EPERM)
            )
        {
            return false;
        }

        let output = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .output();
        let Ok(output) = output else {
            return true;
        };
        if !output.status.success() {
            return true;
        }

        !String::from_utf8_lossy(&output.stdout)
            .trim_start()
            .starts_with('Z')
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurrentStateConsumerResult {
    pub applied_to_generation_seq: u64,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub metrics: Option<Value>,
}

impl CurrentStateConsumerResult {
    pub fn applied(applied_to_generation_seq: u64) -> Self {
        Self {
            applied_to_generation_seq,
            warnings: Vec::new(),
            metrics: None,
        }
    }
}

pub trait CurrentStateConsumer: Send + Sync {
    fn capability_id(&self) -> &str;
    fn consumer_id(&self) -> &str;
    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a>;
}

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

    #[test]
    fn current_state_consumer_result_helper_defaults_to_empty_metadata() {
        let result = CurrentStateConsumerResult::applied(7);
        assert_eq!(result.applied_to_generation_seq, 7);
        assert!(result.warnings.is_empty());
        assert_eq!(result.metrics, None);
    }
}
