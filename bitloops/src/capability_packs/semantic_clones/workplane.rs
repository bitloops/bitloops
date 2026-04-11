use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::config::{SemanticCloneEmbeddingMode, SemanticClonesConfig, SemanticSummaryMode};
use crate::host::capability_host::gateways::{CapabilityWorkplaneGateway, CapabilityWorkplaneJob};

use super::types::{
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticClonesArtefactMailboxPayload {
    pub artefact_id: String,
}

pub fn enqueue_summary_refresh_jobs(
    workplane: &dyn CapabilityWorkplaneGateway,
    inputs: &[semantic_features::SemanticFeatureInput],
) -> Result<()> {
    enqueue_artefact_jobs(workplane, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX, inputs)
}

pub fn enqueue_embedding_jobs(
    workplane: &dyn CapabilityWorkplaneGateway,
    config: &SemanticClonesConfig,
    inputs: &[semantic_features::SemanticFeatureInput],
    summary_refresh_required: bool,
) -> Result<()> {
    let mut jobs = Vec::new();
    if config.embedding_mode != SemanticCloneEmbeddingMode::Off
        && config
            .inference
            .code_embeddings
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        jobs.extend(build_embedding_jobs(
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
            inputs,
        ));
    }
    if !summary_refresh_required
        && config.embedding_mode != SemanticCloneEmbeddingMode::Off
        && config
            .inference
            .summary_embeddings
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
    {
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

pub fn summary_refresh_required(config: &SemanticClonesConfig) -> bool {
    matches!(config.summary_mode, SemanticSummaryMode::Auto)
        && config
            .inference
            .summary_generation
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
}

pub fn payload_artefact_id(payload: &serde_json::Value) -> Option<String> {
    serde_json::from_value::<SemanticClonesArtefactMailboxPayload>(payload.clone())
        .ok()
        .map(|payload| payload.artefact_id)
}

pub fn payload_representation_kind(mailbox_name: &str) -> Option<EmbeddingRepresentationKind> {
    match mailbox_name {
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX => Some(EmbeddingRepresentationKind::Code),
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => Some(EmbeddingRepresentationKind::Summary),
        _ => None,
    }
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
                serde_json::to_value(SemanticClonesArtefactMailboxPayload {
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
                serde_json::to_value(SemanticClonesArtefactMailboxPayload {
                    artefact_id: input.artefact_id.clone(),
                })
                .expect("embedding workplane payload should serialize"),
            )
        })
        .collect()
}
