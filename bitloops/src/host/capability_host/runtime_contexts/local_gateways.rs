use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload;
use crate::host::capability_host::CapabilityMailboxRegistration;
use crate::host::capability_host::gateways::{
    CanonicalGraphGateway, CapabilityMailboxStatus, CapabilityWorkplaneEnqueueResult,
    CapabilityWorkplaneGateway, CapabilityWorkplaneJob, ProvenanceBuilder, StoreHealthGateway,
};
use crate::host::runtime_store::{
    CapabilityWorkplaneJobInsert, RepoSqliteRuntimeStore, SemanticEmbeddingMailboxItemInsert,
    SemanticMailboxItemKind, SemanticSummaryMailboxItemInsert,
};

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
    declared_mailboxes: BTreeMap<(String, String), ()>,
    init_session_id: Option<String>,
}

impl LocalCapabilityWorkplaneGateway {
    pub fn new(
        repo_root: &Path,
        capability_id: &str,
        declared_mailboxes: &[CapabilityMailboxRegistration],
        init_session_id: Option<String>,
    ) -> Result<Self> {
        Ok(Self {
            capability_id: capability_id.to_string(),
            runtime_store: RepoSqliteRuntimeStore::open(repo_root)?,
            declared_mailboxes: declared_mailboxes
                .iter()
                .map(|registration| {
                    (
                        (
                            registration.capability_id.to_string(),
                            registration.mailbox_name.to_string(),
                        ),
                        (),
                    )
                })
                .collect(),
            init_session_id,
        })
    }

    fn target_capability_id<'a>(&'a self, job: &'a CapabilityWorkplaneJob) -> &'a str {
        job.target_capability_id
            .as_deref()
            .unwrap_or(self.capability_id.as_str())
    }

    fn ensure_mailbox_declared(
        &self,
        target_capability_id: &str,
        mailbox_name: &str,
    ) -> Result<()> {
        anyhow::ensure!(
            self.declared_mailboxes
                .contains_key(&(target_capability_id.to_string(), mailbox_name.to_string())),
            "mailbox `{mailbox_name}` is not declared for capability `{}`",
            target_capability_id
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
            self.ensure_mailbox_declared(self.target_capability_id(job), &job.mailbox_name)?;
        }
        let mut workplane_jobs = BTreeMap::<String, Vec<CapabilityWorkplaneJobInsert>>::new();
        let mut summary_items = Vec::new();
        let mut embedding_items = Vec::new();

        for job in jobs {
            let target_capability_id = job
                .target_capability_id
                .clone()
                .unwrap_or_else(|| self.capability_id.clone());
            if target_capability_id == SEMANTIC_CLONES_CAPABILITY_ID {
                match job.mailbox_name.as_str() {
                    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX => {
                        summary_items.push(summary_mailbox_item_from_job(
                            self.init_session_id.clone(),
                            job,
                        )?);
                        continue;
                    }
                    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX => {
                        embedding_items.push(embedding_mailbox_item_from_job(
                            self.init_session_id.clone(),
                            EmbeddingRepresentationKind::Code,
                            job,
                        )?);
                        continue;
                    }
                    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX => {
                        embedding_items.push(embedding_mailbox_item_from_job(
                            self.init_session_id.clone(),
                            EmbeddingRepresentationKind::Identity,
                            job,
                        )?);
                        continue;
                    }
                    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => {
                        embedding_items.push(embedding_mailbox_item_from_job(
                            self.init_session_id.clone(),
                            EmbeddingRepresentationKind::Summary,
                            job,
                        )?);
                        continue;
                    }
                    _ => {}
                }
            }

            workplane_jobs
                .entry(target_capability_id)
                .or_default()
                .push(CapabilityWorkplaneJobInsert::new(
                    job.mailbox_name,
                    self.init_session_id.clone(),
                    job.dedupe_key,
                    job.payload,
                ));
        }

        let mut totals = CapabilityWorkplaneEnqueueResult::default();
        for (target_capability_id, jobs) in workplane_jobs {
            let result = self
                .runtime_store
                .enqueue_capability_workplane_jobs(&target_capability_id, jobs)?;
            totals.inserted_jobs += result.inserted_jobs;
            totals.updated_jobs += result.updated_jobs;
        }
        if !summary_items.is_empty() {
            let result = self
                .runtime_store
                .enqueue_semantic_summary_mailbox_items(summary_items)?;
            totals.inserted_jobs += result.inserted_jobs;
            totals.updated_jobs += result.updated_jobs;
        }
        if !embedding_items.is_empty() {
            let result = self
                .runtime_store
                .enqueue_semantic_embedding_mailbox_items(embedding_items)?;
            totals.inserted_jobs += result.inserted_jobs;
            totals.updated_jobs += result.updated_jobs;
        }

        Ok(CapabilityWorkplaneEnqueueResult {
            inserted_jobs: totals.inserted_jobs,
            updated_jobs: totals.updated_jobs,
        })
    }

    fn mailbox_status(&self) -> Result<BTreeMap<String, CapabilityMailboxStatus>> {
        self.runtime_store
            .load_capability_workplane_mailbox_status(
                &self.capability_id,
                self.declared_mailboxes
                    .keys()
                    .filter_map(|(capability_id, mailbox_name)| {
                        (capability_id == &self.capability_id).then_some(mailbox_name.as_str())
                    }),
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

fn summary_mailbox_item_from_job(
    init_session_id: Option<String>,
    job: CapabilityWorkplaneJob,
) -> Result<SemanticSummaryMailboxItemInsert> {
    let (item_kind, artefact_id, payload_json) = semantic_mailbox_payload_parts(&job.payload)?;
    Ok(SemanticSummaryMailboxItemInsert::new(
        init_session_id,
        item_kind,
        artefact_id,
        payload_json,
        job.dedupe_key,
    ))
}

fn embedding_mailbox_item_from_job(
    init_session_id: Option<String>,
    representation_kind: EmbeddingRepresentationKind,
    job: CapabilityWorkplaneJob,
) -> Result<SemanticEmbeddingMailboxItemInsert> {
    let (item_kind, artefact_id, payload_json) = semantic_mailbox_payload_parts(&job.payload)?;
    Ok(SemanticEmbeddingMailboxItemInsert::new(
        init_session_id,
        representation_kind.to_string(),
        item_kind,
        artefact_id,
        payload_json,
        job.dedupe_key,
    ))
}

fn semantic_mailbox_payload_parts(
    payload: &Value,
) -> Result<(SemanticMailboxItemKind, Option<String>, Option<Value>)> {
    let payload = serde_json::from_value::<SemanticClonesMailboxPayload>(payload.clone())?;
    let parts = match payload {
        SemanticClonesMailboxPayload::Artefact { artefact_id } => {
            (SemanticMailboxItemKind::Artefact, Some(artefact_id), None)
        }
        SemanticClonesMailboxPayload::RepoBackfill { artefact_ids, .. } => (
            SemanticMailboxItemKind::RepoBackfill,
            None,
            artefact_ids.map(serde_json::to_value).transpose()?,
        ),
    };
    Ok(parts)
}
