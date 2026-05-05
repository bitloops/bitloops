use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::runtime_config::{
    resolve_selected_summary_slot, resolve_semantic_clones_config,
};
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_GENERATION_SLOT, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::daemon::types::BlockedMailboxStatus;
use crate::host::capability_host::{
    CapabilityMailboxReadinessPolicy, CapabilityMailboxRegistration,
};
use crate::host::inference::InferenceGateway;
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, SemanticEmbeddingMailboxItemRecord, SemanticMailboxItemStatus,
    SemanticSummaryMailboxItemRecord, WorkplaneJobRecord, WorkplaneJobStatus,
};

use super::super::WorkplaneMailboxReadiness;
use super::jobs::load_workplane_jobs_by_status;
use super::mailbox_persistence::{
    load_embedding_mailbox_items_by_status, load_summary_mailbox_items_by_status,
};
use super::sql::repo_identity_from_runtime_metadata;

pub(crate) fn current_workplane_mailbox_blocked_statuses(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
) -> Result<Vec<BlockedMailboxStatus>> {
    current_workplane_mailbox_blocked_statuses_for_repo_internal(
        workplane_store,
        runtime_store,
        None,
    )
}

pub(crate) fn current_workplane_mailbox_blocked_statuses_for_repo(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    repo_id: &str,
) -> Result<Vec<BlockedMailboxStatus>> {
    current_workplane_mailbox_blocked_statuses_for_repo_internal(
        workplane_store,
        runtime_store,
        Some(repo_id),
    )
}

fn current_workplane_mailbox_blocked_statuses_for_repo_internal(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    repo_id: Option<&str>,
) -> Result<Vec<BlockedMailboxStatus>> {
    let jobs = workplane_store.with_connection(load_pending_mailbox_readiness_jobs)?;
    let mut readiness_cache = BTreeMap::new();
    let mut blocked_by_mailbox = BTreeMap::<String, String>::new();
    for job in jobs {
        if repo_id.is_some_and(|repo_id| job.repo_id != repo_id) {
            continue;
        }
        let readiness = mailbox_claim_readiness(runtime_store, &mut readiness_cache, &job)?;
        if readiness.blocked
            && let Some(reason) = readiness.reason
        {
            blocked_by_mailbox
                .entry(job.mailbox_name.clone())
                .or_insert(reason);
        }
    }
    Ok(blocked_by_mailbox
        .into_iter()
        .map(|(mailbox_name, reason)| BlockedMailboxStatus {
            mailbox_name,
            reason,
        })
        .collect())
}

pub(crate) fn load_pending_mailbox_readiness_jobs(
    conn: &rusqlite::Connection,
) -> Result<Vec<WorkplaneJobRecord>> {
    let mut jobs = load_workplane_jobs_by_status(conn, WorkplaneJobStatus::Pending)?;
    let summary_items =
        load_summary_mailbox_items_by_status(conn, SemanticMailboxItemStatus::Pending)?;
    jobs.extend(
        summary_items
            .into_iter()
            .map(summary_mailbox_item_as_readiness_job),
    );
    let embedding_items =
        load_embedding_mailbox_items_by_status(conn, SemanticMailboxItemStatus::Pending)?;
    jobs.extend(
        embedding_items
            .into_iter()
            .map(embedding_mailbox_item_as_readiness_job),
    );
    Ok(jobs)
}

pub(crate) fn mailbox_readiness_job(
    repo_id: &str,
    repo_root: &Path,
    config_root: &Path,
    mailbox_name: &str,
) -> WorkplaneJobRecord {
    WorkplaneJobRecord {
        job_id: format!("mailbox-readiness-{mailbox_name}"),
        repo_id: repo_id.to_string(),
        repo_root: repo_root.to_path_buf(),
        config_root: config_root.to_path_buf(),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name: mailbox_name.to_string(),
        init_session_id: None,
        dedupe_key: None,
        payload: serde_json::Value::Null,
        status: WorkplaneJobStatus::Pending,
        attempts: 0,
        available_at_unix: 0,
        submitted_at_unix: 0,
        started_at_unix: None,
        updated_at_unix: 0,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: None,
    }
}

fn summary_mailbox_item_as_readiness_job(
    item: SemanticSummaryMailboxItemRecord,
) -> WorkplaneJobRecord {
    WorkplaneJobRecord {
        job_id: item.item_id,
        repo_id: item.repo_id,
        repo_root: item.repo_root,
        config_root: item.config_root,
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX.to_string(),
        init_session_id: item.init_session_id,
        dedupe_key: item.dedupe_key,
        payload: serde_json::Value::Null,
        status: WorkplaneJobStatus::Pending,
        attempts: item.attempts,
        available_at_unix: item.available_at_unix,
        submitted_at_unix: item.submitted_at_unix,
        started_at_unix: None,
        updated_at_unix: item.updated_at_unix,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: item.last_error,
    }
}

fn embedding_mailbox_item_as_readiness_job(
    item: SemanticEmbeddingMailboxItemRecord,
) -> WorkplaneJobRecord {
    let mailbox_name =
        if item.representation_kind == EmbeddingRepresentationKind::Summary.to_string() {
            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
        } else {
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
        };
    WorkplaneJobRecord {
        job_id: item.item_id,
        repo_id: item.repo_id,
        repo_root: item.repo_root,
        config_root: item.config_root,
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name: mailbox_name.to_string(),
        init_session_id: item.init_session_id,
        dedupe_key: item.dedupe_key,
        payload: serde_json::Value::Null,
        status: WorkplaneJobStatus::Pending,
        attempts: item.attempts,
        available_at_unix: item.available_at_unix,
        submitted_at_unix: item.submitted_at_unix,
        started_at_unix: None,
        updated_at_unix: item.updated_at_unix,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: item.last_error,
    }
}

pub(crate) fn mailbox_claim_readiness(
    runtime_store: &DaemonSqliteRuntimeStore,
    cache: &mut BTreeMap<(PathBuf, String, String), WorkplaneMailboxReadiness>,
    job: &WorkplaneJobRecord,
) -> Result<WorkplaneMailboxReadiness> {
    let key = (
        job.repo_root.clone(),
        job.capability_id.clone(),
        job.mailbox_name.clone(),
    );
    if let Some(readiness) = cache.get(&key) {
        return Ok(readiness.clone());
    }
    let Some(registration) = workplane_mailbox_registration_for_job(job)? else {
        let readiness = WorkplaneMailboxReadiness {
            blocked: true,
            reason: Some(format!(
                "mailbox `{}` is not registered for capability `{}`",
                job.mailbox_name, job.capability_id
            )),
        };
        cache.insert(key, readiness.clone());
        return Ok(readiness);
    };

    mailbox_claim_readiness_for_registration(runtime_store, cache, job, &registration)
}

pub(crate) fn mailbox_claim_readiness_for_registration(
    runtime_store: &DaemonSqliteRuntimeStore,
    cache: &mut BTreeMap<(PathBuf, String, String), WorkplaneMailboxReadiness>,
    job: &WorkplaneJobRecord,
    registration: &CapabilityMailboxRegistration,
) -> Result<WorkplaneMailboxReadiness> {
    let key = (
        job.repo_root.clone(),
        job.capability_id.clone(),
        job.mailbox_name.clone(),
    );
    if let Some(readiness) = cache.get(&key) {
        return Ok(readiness.clone());
    }
    let readiness = match registration.readiness_policy {
        CapabilityMailboxReadinessPolicy::None => WorkplaneMailboxReadiness::default(),
        CapabilityMailboxReadinessPolicy::TextGenerationSlot(slot_name) => {
            resolve_mailbox_provider_readiness(runtime_store, job, slot_name, true)?
        }
        CapabilityMailboxReadinessPolicy::OptionalTextGenerationSlot(slot_name) => {
            resolve_optional_text_generation_readiness(runtime_store, job, slot_name)?
        }
        CapabilityMailboxReadinessPolicy::EmbeddingsSlot(slot_name) => {
            resolve_mailbox_provider_readiness(runtime_store, job, slot_name, false)?
        }
    };
    cache.insert(key, readiness.clone());
    Ok(readiness)
}

fn resolve_mailbox_provider_readiness(
    runtime_store: &DaemonSqliteRuntimeStore,
    job: &WorkplaneJobRecord,
    slot_name: &str,
    text_generation: bool,
) -> Result<WorkplaneMailboxReadiness> {
    if !text_generation {
        let gate_status = crate::daemon::embeddings_bootstrap::gate_status_for_config_path(
            runtime_store,
            &job.config_root
                .join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        )?;
        if gate_status.blocked {
            return Ok(WorkplaneMailboxReadiness {
                blocked: true,
                reason: gate_status.reason,
            });
        }
    }

    let repo = repo_identity_from_runtime_metadata(&job.repo_root, &job.repo_id);
    let capability_host = crate::host::devql::build_capability_host(&job.repo_root, repo)?;
    if text_generation
        && job.capability_id == SEMANTIC_CLONES_CAPABILITY_ID
        && job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
        && slot_name == SEMANTIC_CLONES_SUMMARY_GENERATION_SLOT
    {
        let semantic_clones = resolve_semantic_clones_config(
            &capability_host.config_view(SEMANTIC_CLONES_CAPABILITY_ID),
        );
        if semantic_clones.summary_mode != crate::config::SemanticSummaryMode::Off
            && resolve_selected_summary_slot(&semantic_clones).is_none()
        {
            return Ok(WorkplaneMailboxReadiness::default());
        }
    }
    let inference = capability_host.inference_for_capability(&job.capability_id);
    let Some(_slot) = inference.describe(slot_name) else {
        return Ok(WorkplaneMailboxReadiness {
            blocked: true,
            reason: Some(format!(
                "{} slot `{slot_name}` is not configured yet",
                if text_generation {
                    "text-generation"
                } else {
                    "embedding"
                }
            )),
        });
    };

    let resolution = if text_generation {
        inference.text_generation(slot_name).map(|_| ())
    } else {
        inference.embeddings(slot_name).map(|_| ())
    };
    match resolution {
        Ok(()) => Ok(WorkplaneMailboxReadiness::default()),
        Err(err) => Ok(WorkplaneMailboxReadiness {
            blocked: true,
            reason: Some(format!("{err:#}")),
        }),
    }
}

fn resolve_optional_text_generation_readiness(
    _runtime_store: &DaemonSqliteRuntimeStore,
    job: &WorkplaneJobRecord,
    slot_name: &str,
) -> Result<WorkplaneMailboxReadiness> {
    let repo = repo_identity_from_runtime_metadata(&job.repo_root, &job.repo_id);
    let capability_host = crate::host::devql::build_capability_host(&job.repo_root, repo)?;
    let inference = capability_host.inference_for_capability(&job.capability_id);

    if inference.describe(slot_name).is_none() {
        return Ok(WorkplaneMailboxReadiness::default());
    }

    match inference.text_generation(slot_name).map(|_| ()) {
        Ok(()) => Ok(WorkplaneMailboxReadiness::default()),
        Err(err) => Ok(WorkplaneMailboxReadiness {
            blocked: true,
            reason: Some(format!("{err:#}")),
        }),
    }
}

pub(crate) fn workplane_mailbox_registration_for_job(
    job: &WorkplaneJobRecord,
) -> Result<Option<CapabilityMailboxRegistration>> {
    let repo = repo_identity_from_runtime_metadata(&job.repo_root, &job.repo_id);
    let capability_host = crate::host::devql::build_capability_host(&job.repo_root, repo)?;
    Ok(capability_host.mailbox_registration(&job.capability_id, &job.mailbox_name))
}
