use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::config::{
    SemanticCloneEmbeddingMode, SemanticClonesConfig, SemanticSummaryMode,
    resolve_bound_daemon_config_path_for_repo, resolve_daemon_config_path_for_repo,
};
use crate::host::capability_host::gateways::{CapabilityWorkplaneGateway, CapabilityWorkplaneJob};
use crate::host::runtime_store::RepoSqliteRuntimeStore;

use super::runtime_config::{embedding_slot_for_representation, resolve_selected_summary_slot};
use super::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};

pub const SEMANTIC_CLONES_EMBEDDING_PIPELINE_MAILBOXES: [&str; 3] = [
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
];

pub const SEMANTIC_CLONES_DEFERRED_PIPELINE_MAILBOXES: [&str; 4] = [
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
];

const REPO_BACKFILL_DEDUPE_SUFFIX: &str = "repo_backfill";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SemanticClonesMailboxPayload {
    Artefact { artefact_id: String },
    RepoBackfill,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum LegacySemanticClonesMailboxPayload {
    Structured(SemanticClonesMailboxPayload),
    LegacyArtefact { artefact_id: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticClonesMailboxIntentState {
    pub summary_refresh_active: bool,
    pub code_embeddings_active: bool,
    pub summary_embeddings_active: bool,
    pub clone_rebuild_active: bool,
}

impl SemanticClonesMailboxIntentState {
    pub fn has_any_pipeline_intent(&self) -> bool {
        self.summary_refresh_active
            || self.code_embeddings_active
            || self.summary_embeddings_active
            || self.clone_rebuild_active
    }

    pub fn has_any_embedding_intent(&self) -> bool {
        self.code_embeddings_active || self.summary_embeddings_active || self.clone_rebuild_active
    }
}

pub fn activate_deferred_pipeline_mailboxes(repo_root: &Path, source: &str) -> Result<()> {
    let store = open_workplane_store_for_repo(repo_root)?;
    store.set_capability_workplane_mailbox_intents(
        SEMANTIC_CLONES_CAPABILITY_ID,
        SEMANTIC_CLONES_DEFERRED_PIPELINE_MAILBOXES.iter().copied(),
        true,
        Some(source),
    )
}

pub fn activate_embedding_pipeline_mailboxes(repo_root: &Path, source: &str) -> Result<()> {
    let store = open_workplane_store_for_repo(repo_root)?;
    store.set_capability_workplane_mailbox_intents(
        SEMANTIC_CLONES_CAPABILITY_ID,
        SEMANTIC_CLONES_EMBEDDING_PIPELINE_MAILBOXES.iter().copied(),
        true,
        Some(source),
    )
}

pub fn resolve_effective_mailbox_intent(
    workplane: &dyn CapabilityWorkplaneGateway,
    config: &SemanticClonesConfig,
) -> Result<SemanticClonesMailboxIntentState> {
    let status = workplane.mailbox_status()?;
    Ok(resolve_effective_mailbox_intent_from_status(
        &status, config,
    ))
}

pub fn load_effective_mailbox_intent_for_repo(
    repo_root: &Path,
    config: &SemanticClonesConfig,
) -> Result<SemanticClonesMailboxIntentState> {
    let store = open_workplane_store_for_repo(repo_root)?;
    let status = store.load_capability_workplane_mailbox_status(
        SEMANTIC_CLONES_CAPABILITY_ID,
        SEMANTIC_CLONES_DEFERRED_PIPELINE_MAILBOXES.iter().copied(),
    )?;
    Ok(resolve_effective_mailbox_intent_from_status(
        &status, config,
    ))
}

fn resolve_effective_mailbox_intent_from_status(
    status: &std::collections::BTreeMap<
        String,
        crate::host::capability_host::gateways::CapabilityMailboxStatus,
    >,
    config: &SemanticClonesConfig,
) -> SemanticClonesMailboxIntentState {
    let repo_intent = |mailbox_name: &str| {
        status
            .get(mailbox_name)
            .map(|status| status.intent_active)
            .unwrap_or(false)
    };
    let summary_live = config.summary_mode != SemanticSummaryMode::Off
        && resolve_selected_summary_slot(config).is_some();
    let code_live = config.embedding_mode != SemanticCloneEmbeddingMode::Off
        && embedding_slot_for_representation(config, EmbeddingRepresentationKind::Code).is_some();
    let summary_embedding_live = config.embedding_mode != SemanticCloneEmbeddingMode::Off
        && embedding_slot_for_representation(config, EmbeddingRepresentationKind::Summary)
            .is_some();

    SemanticClonesMailboxIntentState {
        summary_refresh_active: repo_intent(SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
            || summary_live,
        code_embeddings_active: repo_intent(SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX) || code_live,
        summary_embeddings_active: repo_intent(SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX)
            || summary_embedding_live,
        clone_rebuild_active: repo_intent(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
            || code_live
            || summary_embedding_live,
    }
}

pub fn enqueue_summary_refresh_jobs(
    workplane: &dyn CapabilityWorkplaneGateway,
    inputs: &[semantic_features::SemanticFeatureInput],
    intent: &SemanticClonesMailboxIntentState,
) -> Result<()> {
    if !intent.summary_refresh_active {
        return Ok(());
    }
    enqueue_artefact_jobs(workplane, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX, inputs)
}

pub fn enqueue_embedding_jobs(
    workplane: &dyn CapabilityWorkplaneGateway,
    inputs: &[semantic_features::SemanticFeatureInput],
    intent: &SemanticClonesMailboxIntentState,
) -> Result<()> {
    let mut jobs = Vec::new();
    if intent.code_embeddings_active {
        jobs.extend(build_embedding_jobs(
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            inputs,
        ));
    }
    if intent.summary_embeddings_active && !intent.summary_refresh_active {
        jobs.extend(build_embedding_jobs(
            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            inputs,
        ));
    }
    if jobs.is_empty() {
        return Ok(());
    }
    let _ = workplane.enqueue_jobs(jobs)?;
    Ok(())
}

pub fn payload_artefact_id(payload: &serde_json::Value) -> Option<String> {
    match serde_json::from_value::<LegacySemanticClonesMailboxPayload>(payload.clone()).ok()? {
        LegacySemanticClonesMailboxPayload::Structured(
            SemanticClonesMailboxPayload::Artefact { artefact_id },
        ) => Some(artefact_id),
        LegacySemanticClonesMailboxPayload::Structured(
            SemanticClonesMailboxPayload::RepoBackfill,
        ) => None,
        LegacySemanticClonesMailboxPayload::LegacyArtefact { artefact_id } => Some(artefact_id),
    }
}

pub fn payload_is_repo_backfill(payload: &serde_json::Value) -> bool {
    matches!(
        serde_json::from_value::<LegacySemanticClonesMailboxPayload>(payload.clone()),
        Ok(LegacySemanticClonesMailboxPayload::Structured(
            SemanticClonesMailboxPayload::RepoBackfill
        ))
    )
}

pub fn payload_representation_kind(mailbox_name: &str) -> Option<EmbeddingRepresentationKind> {
    match mailbox_name {
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX => Some(EmbeddingRepresentationKind::Code),
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => Some(EmbeddingRepresentationKind::Summary),
        _ => None,
    }
}

pub fn repo_backfill_dedupe_key(mailbox_name: &str) -> String {
    format!("{mailbox_name}:{REPO_BACKFILL_DEDUPE_SUFFIX}")
}

fn open_workplane_store_for_repo(repo_root: &Path) -> Result<RepoSqliteRuntimeStore> {
    let config_path = resolve_bound_daemon_config_path_for_repo(repo_root)
        .or_else(|_| resolve_daemon_config_path_for_repo(repo_root))?;
    let config_root = config_path.parent().unwrap_or(repo_root);
    RepoSqliteRuntimeStore::open_for_roots(config_root, repo_root)
}

fn enqueue_artefact_jobs(
    workplane: &dyn CapabilityWorkplaneGateway,
    mailbox_name: &str,
    inputs: &[semantic_features::SemanticFeatureInput],
) -> Result<()> {
    let jobs = inputs
        .iter()
        .map(|input| {
            CapabilityWorkplaneJob::new(
                mailbox_name,
                Some(format!("{mailbox_name}:{}", input.artefact_id)),
                serde_json::to_value(SemanticClonesMailboxPayload::Artefact {
                    artefact_id: input.artefact_id.clone(),
                })
                .expect("workplane payload should serialize"),
            )
        })
        .collect::<Vec<_>>();
    if jobs.is_empty() {
        return Ok(());
    }
    let _ = workplane.enqueue_jobs(jobs)?;
    Ok(())
}

fn build_embedding_jobs(
    mailbox_name: &str,
    inputs: &[semantic_features::SemanticFeatureInput],
) -> Vec<CapabilityWorkplaneJob> {
    inputs
        .iter()
        .map(|input| {
            CapabilityWorkplaneJob::new(
                mailbox_name,
                Some(format!("{mailbox_name}:{}", input.artefact_id)),
                serde_json::to_value(SemanticClonesMailboxPayload::Artefact {
                    artefact_id: input.artefact_id.clone(),
                })
                .expect("embedding workplane payload should serialize"),
            )
        })
        .collect()
}
