use crate::host::capability_host::gateways::CapabilityWorkplaneJob;

use super::super::workplane::{
    REPO_BACKFILL_MAILBOX_CHUNK_SIZE, SemanticClonesMailboxPayload, repo_backfill_chunk_dedupe_key,
    repo_backfill_dedupe_key,
};

pub(super) fn artefact_job(
    mailbox_name: &str,
    artefact_id: &str,
) -> anyhow::Result<CapabilityWorkplaneJob> {
    Ok(CapabilityWorkplaneJob::new(
        mailbox_name,
        Some(format!("{mailbox_name}:{artefact_id}")),
        serde_json::to_value(SemanticClonesMailboxPayload::Artefact {
            artefact_id: artefact_id.to_string(),
        })?,
    ))
}

pub(super) fn repo_backfill_job(
    mailbox_name: &str,
    work_item_count: Option<u64>,
    artefact_ids: Option<Vec<String>>,
    dedupe_key: String,
) -> anyhow::Result<CapabilityWorkplaneJob> {
    Ok(CapabilityWorkplaneJob::new(
        mailbox_name,
        Some(dedupe_key),
        serde_json::to_value(SemanticClonesMailboxPayload::RepoBackfill {
            work_item_count,
            artefact_ids,
        })?,
    ))
}

pub(super) fn repo_backfill_jobs(
    mailbox_name: &str,
    artefact_ids: &[String],
) -> anyhow::Result<Vec<CapabilityWorkplaneJob>> {
    if artefact_ids.is_empty() {
        return Ok(vec![repo_backfill_job(
            mailbox_name,
            Some(0),
            Some(Vec::new()),
            repo_backfill_dedupe_key(mailbox_name),
        )?]);
    }

    let mut jobs = Vec::new();
    let use_chunk_dedupe_keys = artefact_ids.len() > REPO_BACKFILL_MAILBOX_CHUNK_SIZE;
    for chunk in artefact_ids.chunks(REPO_BACKFILL_MAILBOX_CHUNK_SIZE) {
        let chunk_ids = chunk.to_vec();
        let dedupe_key = if use_chunk_dedupe_keys {
            repo_backfill_chunk_dedupe_key(mailbox_name, &chunk_ids)
        } else {
            repo_backfill_dedupe_key(mailbox_name)
        };
        jobs.push(repo_backfill_job(
            mailbox_name,
            Some(chunk_ids.len() as u64),
            Some(chunk_ids),
            dedupe_key,
        )?);
    }

    Ok(jobs)
}

pub(super) fn embedding_jobs_for_artefacts(
    mailbox_name: &str,
    artefact_ids: &[String],
) -> anyhow::Result<Vec<CapabilityWorkplaneJob>> {
    if artefact_ids.len() > REPO_BACKFILL_MAILBOX_CHUNK_SIZE {
        return repo_backfill_jobs(mailbox_name, artefact_ids);
    }

    artefact_ids
        .iter()
        .map(|artefact_id| artefact_job(mailbox_name, artefact_id))
        .collect()
}
