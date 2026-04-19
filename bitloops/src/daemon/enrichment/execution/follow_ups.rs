use std::collections::BTreeMap;

use crate::host::runtime_store::WorkplaneJobRecord;

use super::super::{EnrichmentJobTarget, FollowUpJob};

#[cfg(test)]
use super::super::EnrichmentJob;

#[cfg(test)]
pub(crate) fn symbol_embeddings_follow_up(
    job: &EnrichmentJob,
    artefact_ids: &[String],
    input_hashes: &BTreeMap<String, String>,
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> FollowUpJob {
    FollowUpJob::SymbolEmbeddings {
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone())
            .with_init_session_id(None),
        artefact_ids: artefact_ids.to_vec(),
        input_hashes: input_hashes.clone(),
        representation_kind,
    }
}

#[cfg(test)]
pub(crate) fn clone_edges_rebuild_follow_up(job: &EnrichmentJob) -> FollowUpJob {
    FollowUpJob::CloneEdgesRebuild {
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone())
            .with_init_session_id(None),
    }
}

pub(crate) fn clone_edges_rebuild_follow_up_from_workplane(
    job: &WorkplaneJobRecord,
) -> FollowUpJob {
    FollowUpJob::CloneEdgesRebuild {
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone())
            .with_init_session_id(job.init_session_id.clone()),
    }
}

pub(crate) fn symbol_embeddings_follow_up_from_artefact_ids(
    job: &WorkplaneJobRecord,
    artefact_ids: &[String],
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> FollowUpJob {
    FollowUpJob::SymbolEmbeddings {
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone())
            .with_init_session_id(job.init_session_id.clone()),
        artefact_ids: artefact_ids.to_vec(),
        input_hashes: BTreeMap::new(),
        representation_kind,
    }
}
