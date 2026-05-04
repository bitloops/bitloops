use anyhow::Result;

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::capability_packs::semantic_clones::workplane::{
    REPO_BACKFILL_MAILBOX_CHUNK_SIZE, SemanticClonesMailboxPayload, repo_backfill_chunk_dedupe_key,
    repo_backfill_dedupe_key,
};
use crate::host::runtime_store::{
    RepoSqliteRuntimeStore, SemanticEmbeddingMailboxItemInsert, SemanticMailboxItemKind,
    SemanticSummaryMailboxItemInsert,
};

use super::super::EnrichmentJobTarget;

fn open_target_store(target: &EnrichmentJobTarget) -> Result<RepoSqliteRuntimeStore> {
    match target.repo_id.as_deref() {
        Some(repo_id) => RepoSqliteRuntimeStore::open_for_roots_with_repo_id(
            &target.config_root,
            &target.repo_root,
            repo_id,
        ),
        None => RepoSqliteRuntimeStore::open_for_roots(&target.config_root, &target.repo_root),
    }
}

pub(crate) fn enqueue_workplane_summary_jobs(
    target: &EnrichmentJobTarget,
    artefact_ids: Vec<String>,
) -> Result<()> {
    let store = open_target_store(target)?;
    let result = if artefact_ids.is_empty() {
        store.enqueue_semantic_summary_mailbox_items(vec![
            SemanticSummaryMailboxItemInsert::new(
                target.init_session_id.clone(),
                SemanticMailboxItemKind::RepoBackfill,
                None,
                None,
                Some(repo_backfill_dedupe_key(
                    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                )),
            ),
        ])?
    } else {
        store.enqueue_semantic_summary_mailbox_items(
            artefact_ids
                .into_iter()
                .map(|artefact_id| {
                    SemanticSummaryMailboxItemInsert::new(
                        target.init_session_id.clone(),
                        SemanticMailboxItemKind::Artefact,
                        Some(artefact_id.clone()),
                        None,
                        Some(format!(
                            "{}:{artefact_id}",
                            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
                        )),
                    )
                })
                .collect(),
        )?
    };
    let _ = result;
    Ok(())
}

pub(crate) fn enqueue_workplane_embedding_jobs(
    target: &EnrichmentJobTarget,
    artefact_ids: Vec<String>,
    representation_kind: EmbeddingRepresentationKind,
) -> Result<()> {
    let mailbox_name = match representation_kind {
        EmbeddingRepresentationKind::Code => SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        EmbeddingRepresentationKind::Identity => SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
        EmbeddingRepresentationKind::Summary => SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    };
    let store = open_target_store(target)?;
    let result = if artefact_ids.is_empty() {
        store.enqueue_semantic_embedding_mailbox_items(vec![
            SemanticEmbeddingMailboxItemInsert::new(
                target.init_session_id.clone(),
                representation_kind.to_string(),
                SemanticMailboxItemKind::RepoBackfill,
                None,
                None,
                Some(repo_backfill_dedupe_key(mailbox_name)),
            ),
        ])?
    } else {
        store.enqueue_semantic_embedding_mailbox_items(
            artefact_ids
                .into_iter()
                .map(|artefact_id| {
                    SemanticEmbeddingMailboxItemInsert::new(
                        target.init_session_id.clone(),
                        representation_kind.to_string(),
                        SemanticMailboxItemKind::Artefact,
                        Some(artefact_id.clone()),
                        None,
                        Some(format!("{mailbox_name}:{artefact_id}")),
                    )
                })
                .collect(),
        )?
    };
    let _ = result;
    Ok(())
}

pub(crate) fn enqueue_workplane_embedding_repo_backfill_job(
    target: &EnrichmentJobTarget,
    artefact_ids: Vec<String>,
    representation_kind: EmbeddingRepresentationKind,
) -> Result<()> {
    if artefact_ids.is_empty() {
        return Ok(());
    }
    let store = open_target_store(target)?;
    let mailbox_name = match representation_kind {
        EmbeddingRepresentationKind::Code => SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        EmbeddingRepresentationKind::Identity => SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
        EmbeddingRepresentationKind::Summary => SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    };
    let use_chunk_dedupe_keys = artefact_ids.len() > REPO_BACKFILL_MAILBOX_CHUNK_SIZE;
    let items = artefact_ids
        .chunks(REPO_BACKFILL_MAILBOX_CHUNK_SIZE)
        .map(|chunk| {
            let chunk_ids = chunk.to_vec();
            let dedupe_key = if use_chunk_dedupe_keys {
                repo_backfill_chunk_dedupe_key(mailbox_name, &chunk_ids)
            } else {
                repo_backfill_dedupe_key(mailbox_name)
            };
            SemanticEmbeddingMailboxItemInsert::new(
                target.init_session_id.clone(),
                representation_kind.to_string(),
                SemanticMailboxItemKind::RepoBackfill,
                None,
                Some(
                    serde_json::to_value(chunk_ids)
                        .expect("embedding repo backfill payload should serialize"),
                ),
                Some(dedupe_key),
            )
        })
        .collect::<Vec<_>>();
    let _ = store.enqueue_semantic_embedding_mailbox_items(items)?;
    Ok(())
}

pub(crate) fn enqueue_workplane_clone_rebuild(target: &EnrichmentJobTarget) -> Result<()> {
    let store = open_target_store(target)?;
    let _ = store.enqueue_capability_workplane_jobs(
        SEMANTIC_CLONES_CAPABILITY_ID,
        vec![
            crate::host::runtime_store::CapabilityWorkplaneJobInsert::new(
                SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
                target.init_session_id.clone(),
                Some(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX.to_string()),
                serde_json::to_value(SemanticClonesMailboxPayload::RepoBackfill {
                    work_item_count: Some(1),
                    artefact_ids: None,
                })
                .expect("clone rebuild payload should serialize"),
            ),
        ],
    )?;
    Ok(())
}
