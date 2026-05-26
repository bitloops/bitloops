use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::config::{
    SemanticCloneEmbeddingMode, SemanticClonesConfig, resolve_bound_daemon_config_path_for_repo,
    resolve_daemon_config_path_for_repo,
};
use crate::host::capability_host::gateways::{CapabilityWorkplaneGateway, CapabilityWorkplaneJob};
use crate::host::runtime_store::RepoSqliteRuntimeStore;

use super::runtime_config::{embedding_slot_for_representation, resolve_selected_summary_slot};
use super::types::{
    SEMANTIC_CLONES_ARCHITECTURE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_CAPABILITY_ID,
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};

pub const SEMANTIC_CLONES_EMBEDDING_PIPELINE_MAILBOXES: [&str; 5] = [
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_ARCHITECTURE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
];

pub const SEMANTIC_CLONES_DEFERRED_PIPELINE_MAILBOXES: [&str; 6] = [
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_ARCHITECTURE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
];

const REPO_BACKFILL_DEDUPE_SUFFIX: &str = "repo_backfill";
pub const REPO_BACKFILL_MAILBOX_CHUNK_SIZE: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SemanticClonesMailboxPayload {
    Artefact {
        artefact_id: String,
    },
    PathCleanup {
        path: String,
    },
    RepoBackfill {
        #[serde(default)]
        work_item_count: Option<u64>,
        #[serde(default)]
        artefact_ids: Option<Vec<String>>,
    },
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
    pub architecture_embeddings_active: bool,
    pub summary_embeddings_active: bool,
    pub clone_rebuild_active: bool,
}

impl SemanticClonesMailboxIntentState {
    pub fn has_any_pipeline_intent(&self) -> bool {
        self.summary_refresh_active
            || self.code_embeddings_active
            || self.architecture_embeddings_active
            || self.summary_embeddings_active
            || self.clone_rebuild_active
    }

    pub fn has_any_embedding_intent(&self) -> bool {
        self.code_embeddings_active
            || self.architecture_embeddings_active
            || self.summary_embeddings_active
            || self.clone_rebuild_active
    }
}

#[cfg(test)]
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

pub fn deactivate_embedding_pipeline_mailboxes(repo_root: &Path, source: &str) -> Result<()> {
    let store = open_workplane_store_for_repo(repo_root)?;
    store.set_capability_workplane_mailbox_intents(
        SEMANTIC_CLONES_CAPABILITY_ID,
        SEMANTIC_CLONES_EMBEDDING_PIPELINE_MAILBOXES.iter().copied(),
        false,
        Some(source),
    )
}

#[allow(dead_code)]
pub fn activate_selected_pipeline_mailboxes(
    repo_root: &Path,
    source: &str,
    summary_refresh_active: bool,
    code_embeddings_active: bool,
    summary_embeddings_active: bool,
    clone_rebuild_active: bool,
) -> Result<()> {
    let mut mailboxes = Vec::new();
    if summary_refresh_active {
        mailboxes.push(SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX);
    }
    if code_embeddings_active {
        mailboxes.push(SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX);
        mailboxes.push(SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX);
        mailboxes.push(SEMANTIC_CLONES_ARCHITECTURE_EMBEDDING_MAILBOX);
    }
    if summary_embeddings_active {
        mailboxes.push(SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX);
    }
    if clone_rebuild_active {
        mailboxes.push(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX);
    }
    if mailboxes.is_empty() {
        return Ok(());
    }

    let store = open_workplane_store_for_repo(repo_root)?;
    store.set_capability_workplane_mailbox_intents(
        SEMANTIC_CLONES_CAPABILITY_ID,
        mailboxes,
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
    let summary_slot_live = resolve_selected_summary_slot(config).is_some();
    let summary_refresh_live = summary_slot_live;
    let embeddings_policy_enabled = config.embedding_mode != SemanticCloneEmbeddingMode::Off;
    let code_live = embeddings_policy_enabled
        && embedding_slot_for_representation(config, EmbeddingRepresentationKind::Code).is_some();
    let summary_embedding_live = summary_slot_live
        && embeddings_policy_enabled
        && embedding_slot_for_representation(config, EmbeddingRepresentationKind::Summary)
            .is_some();
    let stored_code_intent =
        embeddings_policy_enabled && repo_intent(SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX);
    let stored_clone_rebuild_intent =
        embeddings_policy_enabled && repo_intent(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX);

    SemanticClonesMailboxIntentState {
        summary_refresh_active: summary_refresh_live,
        architecture_embeddings_active: repo_intent(SEMANTIC_CLONES_ARCHITECTURE_EMBEDDING_MAILBOX)
            || code_live,
        code_embeddings_active: stored_code_intent || code_live,
        summary_embeddings_active: summary_embedding_live,
        clone_rebuild_active: stored_clone_rebuild_intent || code_live || summary_embedding_live,
    }
}

pub fn payload_artefact_id(payload: &serde_json::Value) -> Option<String> {
    match serde_json::from_value::<LegacySemanticClonesMailboxPayload>(payload.clone()).ok()? {
        LegacySemanticClonesMailboxPayload::Structured(
            SemanticClonesMailboxPayload::Artefact { artefact_id },
        ) => Some(artefact_id),
        LegacySemanticClonesMailboxPayload::Structured(
            SemanticClonesMailboxPayload::RepoBackfill { .. },
        ) => None,
        LegacySemanticClonesMailboxPayload::Structured(
            SemanticClonesMailboxPayload::PathCleanup { .. },
        ) => None,
        LegacySemanticClonesMailboxPayload::LegacyArtefact { artefact_id } => Some(artefact_id),
    }
}

pub fn payload_is_repo_backfill(payload: &serde_json::Value) -> bool {
    matches!(
        serde_json::from_value::<LegacySemanticClonesMailboxPayload>(payload.clone()),
        Ok(LegacySemanticClonesMailboxPayload::Structured(
            SemanticClonesMailboxPayload::RepoBackfill { .. }
        ))
    )
}

pub fn payload_repo_backfill_artefact_ids(payload: &serde_json::Value) -> Option<Vec<String>> {
    match serde_json::from_value::<LegacySemanticClonesMailboxPayload>(payload.clone()).ok()? {
        LegacySemanticClonesMailboxPayload::Structured(
            SemanticClonesMailboxPayload::RepoBackfill { artefact_ids, .. },
        ) => artefact_ids,
        LegacySemanticClonesMailboxPayload::Structured(
            SemanticClonesMailboxPayload::PathCleanup { .. },
        ) => None,
        _ => None,
    }
}

pub fn payload_work_item_count(payload: &serde_json::Value, mailbox_name: &str) -> u64 {
    match serde_json::from_value::<LegacySemanticClonesMailboxPayload>(payload.clone()) {
        Ok(LegacySemanticClonesMailboxPayload::Structured(
            SemanticClonesMailboxPayload::Artefact { .. },
        ))
        | Ok(LegacySemanticClonesMailboxPayload::LegacyArtefact { .. }) => 1,
        Ok(LegacySemanticClonesMailboxPayload::Structured(
            SemanticClonesMailboxPayload::PathCleanup { .. },
        )) => 1,
        Ok(LegacySemanticClonesMailboxPayload::Structured(
            SemanticClonesMailboxPayload::RepoBackfill {
                work_item_count,
                artefact_ids,
            },
        )) => artefact_ids
            .map(|artefact_ids| artefact_ids.len() as u64)
            .or(work_item_count)
            .unwrap_or_else(|| default_work_item_count_for_mailbox(mailbox_name)),
        Err(_) => default_work_item_count_for_mailbox(mailbox_name),
    }
}

pub fn payload_representation_kind(mailbox_name: &str) -> Option<EmbeddingRepresentationKind> {
    match mailbox_name {
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX => Some(EmbeddingRepresentationKind::Code),
        SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX => Some(EmbeddingRepresentationKind::Identity),
        SEMANTIC_CLONES_ARCHITECTURE_EMBEDDING_MAILBOX => {
            Some(EmbeddingRepresentationKind::Architecture)
        }
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => Some(EmbeddingRepresentationKind::Summary),
        _ => None,
    }
}

pub fn repo_backfill_dedupe_key(mailbox_name: &str) -> String {
    format!("{mailbox_name}:{REPO_BACKFILL_DEDUPE_SUFFIX}")
}

pub fn repo_backfill_chunk_dedupe_key(mailbox_name: &str, artefact_ids: &[String]) -> String {
    let mut digest = Sha256::new();
    digest.update(mailbox_name.as_bytes());
    for artefact_id in artefact_ids {
        digest.update([0]);
        digest.update(artefact_id.as_bytes());
    }

    let mut suffix = String::with_capacity(64);
    for byte in digest.finalize() {
        suffix.push_str(&format!("{byte:02x}"));
    }

    format!("{mailbox_name}:{REPO_BACKFILL_DEDUPE_SUFFIX}:chunk:{suffix}")
}

pub fn architecture_embedding_jobs_for_artefacts(
    artefact_ids: &[String],
) -> Result<Vec<CapabilityWorkplaneJob>> {
    embedding_jobs_for_artefacts(SEMANTIC_CLONES_ARCHITECTURE_EMBEDDING_MAILBOX, artefact_ids)
}

pub fn architecture_embedding_path_cleanup_jobs(
    paths: &[String],
) -> Result<Vec<CapabilityWorkplaneJob>> {
    paths
        .iter()
        .map(|path| {
            Ok(CapabilityWorkplaneJob::new_for_capability(
                SEMANTIC_CLONES_CAPABILITY_ID,
                SEMANTIC_CLONES_ARCHITECTURE_EMBEDDING_MAILBOX,
                Some(format!(
                    "{}:path_cleanup:{path}",
                    SEMANTIC_CLONES_ARCHITECTURE_EMBEDDING_MAILBOX
                )),
                serde_json::to_value(SemanticClonesMailboxPayload::PathCleanup {
                    path: path.clone(),
                })?,
            ))
        })
        .collect()
}

fn embedding_jobs_for_artefacts(
    mailbox_name: &str,
    artefact_ids: &[String],
) -> Result<Vec<CapabilityWorkplaneJob>> {
    if artefact_ids.len() > REPO_BACKFILL_MAILBOX_CHUNK_SIZE {
        return repo_backfill_jobs(mailbox_name, artefact_ids);
    }

    artefact_ids
        .iter()
        .map(|artefact_id| artefact_job(mailbox_name, artefact_id))
        .collect()
}

fn artefact_job(mailbox_name: &str, artefact_id: &str) -> Result<CapabilityWorkplaneJob> {
    Ok(CapabilityWorkplaneJob::new_for_capability(
        SEMANTIC_CLONES_CAPABILITY_ID,
        mailbox_name,
        Some(format!("{mailbox_name}:{artefact_id}")),
        serde_json::to_value(SemanticClonesMailboxPayload::Artefact {
            artefact_id: artefact_id.to_string(),
        })?,
    ))
}

fn repo_backfill_jobs(
    mailbox_name: &str,
    artefact_ids: &[String],
) -> Result<Vec<CapabilityWorkplaneJob>> {
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

fn repo_backfill_job(
    mailbox_name: &str,
    work_item_count: Option<u64>,
    artefact_ids: Option<Vec<String>>,
    dedupe_key: String,
) -> Result<CapabilityWorkplaneJob> {
    Ok(CapabilityWorkplaneJob::new_for_capability(
        SEMANTIC_CLONES_CAPABILITY_ID,
        mailbox_name,
        Some(dedupe_key),
        serde_json::to_value(SemanticClonesMailboxPayload::RepoBackfill {
            work_item_count,
            artefact_ids,
        })?,
    ))
}

fn default_work_item_count_for_mailbox(mailbox_name: &str) -> u64 {
    match mailbox_name {
        SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX => 1,
        _ => 1,
    }
}

fn open_workplane_store_for_repo(repo_root: &Path) -> Result<RepoSqliteRuntimeStore> {
    let config_path = resolve_bound_daemon_config_path_for_repo(repo_root)
        .or_else(|_| resolve_daemon_config_path_for_repo(repo_root))?;
    let config_root = config_path.parent().unwrap_or(repo_root);
    RepoSqliteRuntimeStore::open_for_roots(config_root, repo_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SemanticSummaryMode;
    use crate::host::capability_host::gateways::CapabilityMailboxStatus;
    use std::collections::BTreeMap;

    fn status_with_active_intents() -> BTreeMap<String, CapabilityMailboxStatus> {
        SEMANTIC_CLONES_DEFERRED_PIPELINE_MAILBOXES
            .iter()
            .map(|mailbox_name| {
                (
                    (*mailbox_name).to_string(),
                    CapabilityMailboxStatus {
                        intent_active: true,
                        ..CapabilityMailboxStatus::default()
                    },
                )
            })
            .collect()
    }

    #[test]
    fn summary_refresh_deactivates_when_no_summary_provider_is_configured() {
        let intent = resolve_effective_mailbox_intent_from_status(
            &status_with_active_intents(),
            &SemanticClonesConfig::default(),
        );

        assert!(
            !intent.summary_refresh_active,
            "providerless auto summary mode should not keep summary refresh active"
        );
        assert!(intent.code_embeddings_active);
        assert!(!intent.summary_embeddings_active);
        assert!(intent.clone_rebuild_active);
    }

    #[test]
    fn summary_mode_off_disables_summary_refresh_and_summary_embeddings() {
        let config = SemanticClonesConfig {
            summary_mode: SemanticSummaryMode::Off,
            ..SemanticClonesConfig::default()
        };

        let intent =
            resolve_effective_mailbox_intent_from_status(&status_with_active_intents(), &config);

        assert!(!intent.summary_refresh_active);
        assert!(!intent.summary_embeddings_active);
    }

    #[test]
    fn embedding_mode_off_disables_stored_embedding_and_clone_rebuild_intent() {
        let status = BTreeMap::from([
            (
                SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX.to_string(),
                CapabilityMailboxStatus {
                    intent_active: true,
                    ..CapabilityMailboxStatus::default()
                },
            ),
            (
                SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX.to_string(),
                CapabilityMailboxStatus {
                    intent_active: true,
                    ..CapabilityMailboxStatus::default()
                },
            ),
            (
                SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX.to_string(),
                CapabilityMailboxStatus {
                    intent_active: true,
                    ..CapabilityMailboxStatus::default()
                },
            ),
        ]);
        let config = SemanticClonesConfig {
            embedding_mode: SemanticCloneEmbeddingMode::Off,
            inference: crate::config::SemanticClonesInferenceBindings {
                code_embeddings: Some("local_code".to_string()),
                summary_embeddings: Some("local_code".to_string()),
                ..crate::config::SemanticClonesInferenceBindings::default()
            },
            ..SemanticClonesConfig::default()
        };

        let intent = resolve_effective_mailbox_intent_from_status(&status, &config);

        assert!(!intent.code_embeddings_active);
        assert!(!intent.summary_embeddings_active);
        assert!(!intent.clone_rebuild_active);
    }

    #[test]
    fn repo_backfill_chunk_dedupe_key_changes_with_chunk_contents() {
        let mailbox_name = SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX;
        let left = repo_backfill_chunk_dedupe_key(
            mailbox_name,
            &["artefact-1".to_string(), "artefact-2".to_string()],
        );
        let right = repo_backfill_chunk_dedupe_key(
            mailbox_name,
            &["artefact-1".to_string(), "artefact-3".to_string()],
        );

        assert_ne!(left, right);
    }

    #[test]
    fn identity_embedding_mailbox_maps_to_identity_representation() {
        assert_eq!(
            payload_representation_kind(SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX),
            Some(EmbeddingRepresentationKind::Identity)
        );
    }

    #[test]
    fn architecture_embedding_mailbox_maps_to_architecture_representation() {
        assert_eq!(
            payload_representation_kind(SEMANTIC_CLONES_ARCHITECTURE_EMBEDDING_MAILBOX),
            Some(EmbeddingRepresentationKind::Architecture)
        );
    }
}
