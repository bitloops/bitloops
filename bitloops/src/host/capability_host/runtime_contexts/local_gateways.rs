use anyhow::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::host::capability_host::CapabilityMailboxRegistration;
use crate::host::capability_host::gateways::{
    CanonicalGraphGateway, CapabilityMailboxStatus, CapabilityWorkplaneEnqueueResult,
    CapabilityWorkplaneGateway, CapabilityWorkplaneJob, ProvenanceBuilder, StoreHealthGateway,
};
use crate::host::runtime_store::RepoSqliteRuntimeStore;

pub struct LocalCanonicalGraphGateway;

impl CanonicalGraphGateway for LocalCanonicalGraphGateway {}

pub struct DefaultProvenanceBuilder;

impl ProvenanceBuilder for DefaultProvenanceBuilder {
    fn build(&self, capability_id: &str, operation: &str, details: Value) -> Value {
        serde_json::json!({
            "capability": capability_id,
            "operation": operation,
            "details": details,
        })
    }
}

pub struct LocalStoreHealthGateway;

impl StoreHealthGateway for LocalStoreHealthGateway {
    fn check_relational(&self) -> Result<()> {
        Ok(())
    }

    fn check_documents(&self) -> Result<()> {
        Ok(())
    }

    fn check_blobs(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct LocalCapabilityWorkplaneGateway {
    capability_id: String,
    runtime_store: RepoSqliteRuntimeStore,
    declared_mailboxes: BTreeSet<String>,
}

impl LocalCapabilityWorkplaneGateway {
    pub fn new(
        repo_root: &Path,
        capability_id: &str,
        declared_mailboxes: &[CapabilityMailboxRegistration],
    ) -> Result<Self> {
        Ok(Self {
            capability_id: capability_id.to_string(),
            runtime_store: RepoSqliteRuntimeStore::open(repo_root)?,
            declared_mailboxes: declared_mailboxes
                .iter()
                .map(|registration| registration.mailbox_name.to_string())
                .collect(),
        })
    }

    fn ensure_mailbox_declared(&self, mailbox_name: &str) -> Result<()> {
        anyhow::ensure!(
            self.declared_mailboxes.contains(mailbox_name),
            "mailbox `{mailbox_name}` is not declared for capability `{}`",
            self.capability_id
        );
        Ok(())
    }
}

impl CapabilityWorkplaneGateway for LocalCapabilityWorkplaneGateway {
    fn enqueue_jobs(
        &self,
        jobs: Vec<CapabilityWorkplaneJob>,
    ) -> Result<CapabilityWorkplaneEnqueueResult> {
        for job in &jobs {
            self.ensure_mailbox_declared(&job.mailbox_name)?;
        }
        self.runtime_store
            .enqueue_capability_workplane_jobs(
                &self.capability_id,
                jobs.into_iter()
                    .map(|job| {
                        crate::host::runtime_store::CapabilityWorkplaneJobInsert::new(
                            job.mailbox_name,
                            job.dedupe_key,
                            job.payload,
                        )
                    })
                    .collect(),
            )
            .map(|result| CapabilityWorkplaneEnqueueResult {
                inserted_jobs: result.inserted_jobs,
                updated_jobs: result.updated_jobs,
            })
    }

    fn mailbox_status(&self) -> Result<BTreeMap<String, CapabilityMailboxStatus>> {
        self.runtime_store
            .load_capability_workplane_mailbox_status(
                &self.capability_id,
                self.declared_mailboxes.iter().map(String::as_str),
            )
            .map(|status_by_mailbox| {
                status_by_mailbox
                    .into_iter()
                    .map(|(mailbox_name, status)| {
                        (
                            mailbox_name,
                            CapabilityMailboxStatus {
                                pending_jobs: status.pending_jobs,
                                running_jobs: status.running_jobs,
                                failed_jobs: status.failed_jobs,
                                completed_recent_jobs: status.completed_recent_jobs,
                                pending_cursor_runs: status.pending_cursor_runs,
                                running_cursor_runs: status.running_cursor_runs,
                                failed_cursor_runs: status.failed_cursor_runs,
                                completed_recent_cursor_runs: status.completed_recent_cursor_runs,
                                intent_active: status.intent_active,
                                blocked_reason: None,
                            },
                        )
                    })
                    .collect()
            })
    }
}
