use anyhow::Result;

use crate::capability_packs::semantic_clones::ingesters::SymbolEmbeddingsRefreshScope;
use crate::capability_packs::semantic_clones::load_semantic_feature_inputs_for_artefacts;
use crate::capability_packs::semantic_clones::workplane::{
    payload_artefact_id, payload_is_repo_backfill, payload_repo_backfill_artefact_ids,
};
use crate::host::devql::RelationalStorage;
use crate::host::runtime_store::WorkplaneJobRecord;

use super::SemanticFeatureInput;
use super::helpers::load_current_semantic_inputs;

#[cfg(test)]
use super::super::EnrichmentJob;
#[cfg(test)]
use anyhow::Context;

pub(crate) async fn load_workplane_job_inputs(
    relational: &RelationalStorage,
    job: &WorkplaneJobRecord,
) -> Result<Vec<SemanticFeatureInput>> {
    if payload_is_repo_backfill(&job.payload) {
        return load_repo_backfill_inputs(relational, job).await;
    }

    let Some(artefact_id) = payload_artefact_id(&job.payload) else {
        anyhow::bail!("workplane mailbox job missing artefact id");
    };
    load_semantic_feature_inputs_for_artefacts(
        relational,
        &job.repo_root,
        std::slice::from_ref(&artefact_id),
    )
    .await
}

pub(crate) async fn load_workplane_embedding_refresh_inputs(
    relational: &RelationalStorage,
    job: &WorkplaneJobRecord,
) -> Result<(
    SymbolEmbeddingsRefreshScope,
    Option<String>,
    Option<String>,
    Vec<SemanticFeatureInput>,
)> {
    if payload_is_repo_backfill(&job.payload) {
        return Ok((
            SymbolEmbeddingsRefreshScope::Historical,
            None,
            None,
            load_repo_backfill_inputs(relational, job).await?,
        ));
    }

    let Some(artefact_id) = payload_artefact_id(&job.payload) else {
        anyhow::bail!("workplane mailbox job missing artefact id");
    };
    let current_inputs = load_current_semantic_inputs(
        relational,
        &job.repo_root,
        &job.repo_id,
        Some(std::slice::from_ref(&artefact_id)),
    )
    .await?;
    if let Some(first) = current_inputs.first() {
        let single_path = current_inputs
            .iter()
            .all(|input| input.path == first.path && input.blob_sha == first.blob_sha);
        if single_path {
            return Ok((
                SymbolEmbeddingsRefreshScope::CurrentPath,
                Some(first.path.clone()),
                Some(first.blob_sha.clone()),
                current_inputs,
            ));
        }
    }

    Ok((
        SymbolEmbeddingsRefreshScope::Historical,
        None,
        None,
        load_workplane_job_inputs(relational, job).await?,
    ))
}

pub(crate) async fn load_repo_backfill_inputs(
    relational: &RelationalStorage,
    job: &WorkplaneJobRecord,
) -> Result<Vec<SemanticFeatureInput>> {
    let artefact_ids = payload_repo_backfill_artefact_ids(&job.payload);
    load_current_semantic_inputs(
        relational,
        &job.repo_root,
        &job.repo_id,
        artefact_ids.as_deref(),
    )
    .await
}

#[cfg(test)]
pub(crate) async fn load_enrichment_job_inputs(
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    artefact_ids: &[String],
) -> Result<Vec<SemanticFeatureInput>> {
    load_semantic_feature_inputs_for_artefacts(relational, &job.repo_root, artefact_ids)
        .await
        .with_context(|| {
            format!(
                "rehydrating enrichment inputs for job `{}` in repo `{}`",
                job.id, job.repo_id
            )
        })
}
