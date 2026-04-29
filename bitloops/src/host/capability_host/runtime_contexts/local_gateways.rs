use anyhow::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
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
    CapabilityWorkplaneGateway, CapabilityWorkplaneJob, FileHistoryEvent, GitHistoryGateway,
    GitHistoryRequest, ProvenanceBuilder, StoreHealthGateway,
};
use crate::host::checkpoints::strategy::manual_commit::{run_git, try_head_hash};
use crate::host::runtime_store::{
    RepoSqliteRuntimeStore, SemanticEmbeddingMailboxItemInsert, SemanticMailboxItemKind,
    SemanticSummaryMailboxItemInsert,
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

pub struct LocalGitHistoryGateway;

impl GitHistoryGateway for LocalGitHistoryGateway {
    fn available(&self) -> bool {
        true
    }

    fn resolve_head(&self, repo_root: &Path) -> Result<Option<String>> {
        try_head_hash(repo_root)
    }

    fn load_file_history(
        &self,
        repo_root: &Path,
        request: GitHistoryRequest<'_>,
    ) -> Result<Vec<FileHistoryEvent>> {
        if request.paths.is_empty() {
            return Ok(Vec::new());
        }

        let since = format!("--since=@{}", request.since_unix);
        let revision = request.until_commit_sha.unwrap_or("HEAD");
        let format = "--format=%x1e%H%x1f%an%x1f%ae%x1f%ct%x1f%s";
        let mut args = vec![
            "log".to_string(),
            revision.to_string(),
            since,
            "--name-only".to_string(),
            format.to_string(),
            "--no-color".to_string(),
            "--".to_string(),
        ];
        args.extend(request.paths.iter().cloned());
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let raw = run_git(repo_root, &arg_refs)?;
        Ok(parse_file_history_log(&raw, request.bug_patterns))
    }
}

fn parse_file_history_log(raw: &str, bug_patterns: &[String]) -> Vec<FileHistoryEvent> {
    let mut events = Vec::new();
    for record in raw.split('\u{1e}') {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }
        let mut lines = record.lines();
        let Some(header) = lines.next() else {
            continue;
        };
        let mut parts = header.split('\u{1f}');
        let Some(commit_sha) = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let author_name = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let author_email = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let committed_at_unix = parts
            .next()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or_default();
        let message = parts.next().unwrap_or_default().trim().to_string();
        let is_bug_fix = message_matches_bug_patterns(&message, bug_patterns);

        for path in lines.map(str::trim).filter(|line| !line.is_empty()) {
            events.push(FileHistoryEvent {
                path: path.to_string(),
                commit_sha: commit_sha.to_string(),
                author_name: author_name.clone(),
                author_email: author_email.clone(),
                committed_at_unix,
                message: message.clone(),
                is_bug_fix,
                changed_ranges: Vec::new(),
            });
        }
    }
    events
}

fn message_matches_bug_patterns(message: &str, bug_patterns: &[String]) -> bool {
    let message = message.to_ascii_lowercase();
    bug_patterns
        .iter()
        .map(|pattern| pattern.trim().to_ascii_lowercase())
        .filter(|pattern| !pattern.is_empty())
        .any(|pattern| message.contains(&pattern))
}

#[derive(Clone)]
pub struct LocalCapabilityWorkplaneGateway {
    capability_id: String,
    runtime_store: RepoSqliteRuntimeStore,
    declared_mailboxes: BTreeSet<String>,
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
                .map(|registration| registration.mailbox_name.to_string())
                .collect(),
            init_session_id,
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
        let mut workplane_jobs = Vec::new();
        let mut summary_items = Vec::new();
        let mut embedding_items = Vec::new();

        for job in jobs {
            if self.capability_id == SEMANTIC_CLONES_CAPABILITY_ID {
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

            workplane_jobs.push(
                crate::host::runtime_store::CapabilityWorkplaneJobInsert::new(
                    job.mailbox_name,
                    self.init_session_id.clone(),
                    job.dedupe_key,
                    job.payload,
                ),
            );
        }

        let mut totals = CapabilityWorkplaneEnqueueResult::default();
        if !workplane_jobs.is_empty() {
            let result = self
                .runtime_store
                .enqueue_capability_workplane_jobs(&self.capability_id, workplane_jobs)?;
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
