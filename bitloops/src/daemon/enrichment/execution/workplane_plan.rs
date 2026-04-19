use crate::capability_packs::semantic_clones::ingesters::SymbolEmbeddingsRefreshScope;
use crate::capability_packs::semantic_clones::workplane::{
    payload_artefact_id, payload_is_repo_backfill,
};
use crate::host::runtime_store::WorkplaneJobRecord;

use super::super::{EnrichmentJobTarget, FollowUpJob};
use super::SemanticFeatureInput;
use super::follow_ups::symbol_embeddings_follow_up_from_artefact_ids;

pub(crate) const WORKPLANE_SUMMARY_REPO_BACKFILL_BATCH_SIZE: usize = 16;
pub(crate) const WORKPLANE_EMBEDDING_REPO_BACKFILL_BATCH_SIZE: usize = 8;

pub(crate) struct SummaryRefreshWorkplanePlan {
    pub inputs: Vec<SemanticFeatureInput>,
    pub follow_ups: Vec<FollowUpJob>,
}

pub(crate) struct EmbeddingRefreshWorkplanePlan {
    pub scope: SymbolEmbeddingsRefreshScope,
    pub path: Option<String>,
    pub content_id: Option<String>,
    pub inputs: Vec<SemanticFeatureInput>,
    pub manage_active_state: bool,
    pub follow_ups: Vec<FollowUpJob>,
}

pub(crate) fn build_summary_refresh_workplane_plan(
    job: &WorkplaneJobRecord,
    inputs: Vec<SemanticFeatureInput>,
    summary_embeddings_active: bool,
) -> SummaryRefreshWorkplanePlan {
    if payload_is_repo_backfill(&job.payload) {
        return build_repo_backfill_summary_refresh_workplane_plan(
            job,
            inputs,
            summary_embeddings_active,
        );
    }

    let mut follow_ups = Vec::new();
    if summary_embeddings_active && let Some(artefact_id) = payload_artefact_id(&job.payload) {
        follow_ups.push(symbol_embeddings_follow_up_from_artefact_ids(
            job,
            &[artefact_id],
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
        ));
    }
    SummaryRefreshWorkplanePlan { inputs, follow_ups }
}

fn build_repo_backfill_summary_refresh_workplane_plan(
    job: &WorkplaneJobRecord,
    inputs: Vec<SemanticFeatureInput>,
    summary_embeddings_active: bool,
) -> SummaryRefreshWorkplanePlan {
    let batch_size = inputs.len().min(WORKPLANE_SUMMARY_REPO_BACKFILL_BATCH_SIZE);
    let (batch_inputs, remaining_inputs): (Vec<_>, Vec<_>) = inputs
        .into_iter()
        .enumerate()
        .partition(|(index, _)| *index < batch_size);
    let inputs = batch_inputs
        .into_iter()
        .map(|(_, input)| input)
        .collect::<Vec<_>>();
    let remaining_artefact_ids = remaining_inputs
        .into_iter()
        .map(|(_, input)| input.artefact_id)
        .collect::<Vec<_>>();
    let mut follow_ups = Vec::new();
    if summary_embeddings_active {
        let processed_artefact_ids = inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect::<Vec<_>>();
        if !processed_artefact_ids.is_empty() {
            follow_ups.push(symbol_embeddings_follow_up_from_artefact_ids(
                job,
                &processed_artefact_ids,
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
            ));
        }
    }
    if !remaining_artefact_ids.is_empty() {
        follow_ups.push(FollowUpJob::SemanticSummaries {
            target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone())
                .with_init_session_id(job.init_session_id.clone()),
            artefact_ids: remaining_artefact_ids,
        });
    }
    SummaryRefreshWorkplanePlan { inputs, follow_ups }
}

pub(crate) fn build_embedding_refresh_workplane_plan(
    job: &WorkplaneJobRecord,
    scope: SymbolEmbeddingsRefreshScope,
    path: Option<String>,
    content_id: Option<String>,
    inputs: Vec<SemanticFeatureInput>,
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> EmbeddingRefreshWorkplanePlan {
    if !payload_is_repo_backfill(&job.payload) {
        return EmbeddingRefreshWorkplanePlan {
            scope,
            path,
            content_id,
            inputs,
            manage_active_state: false,
            follow_ups: Vec::new(),
        };
    }

    let batch_size = inputs
        .len()
        .min(WORKPLANE_EMBEDDING_REPO_BACKFILL_BATCH_SIZE);
    let (batch_inputs, remaining_inputs): (Vec<_>, Vec<_>) = inputs
        .into_iter()
        .enumerate()
        .partition(|(index, _)| *index < batch_size);
    let inputs = batch_inputs
        .into_iter()
        .map(|(_, input)| input)
        .collect::<Vec<_>>();
    let remaining_artefact_ids = remaining_inputs
        .into_iter()
        .map(|(_, input)| input.artefact_id)
        .collect::<Vec<_>>();
    let mut follow_ups = Vec::new();
    if !remaining_artefact_ids.is_empty() {
        follow_ups.push(FollowUpJob::RepoBackfillEmbeddings {
            target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone())
                .with_init_session_id(job.init_session_id.clone()),
            artefact_ids: remaining_artefact_ids,
            representation_kind,
        });
    }

    EmbeddingRefreshWorkplanePlan {
        scope,
        path,
        content_id,
        inputs,
        manage_active_state: follow_ups.is_empty(),
        follow_ups,
    }
}
