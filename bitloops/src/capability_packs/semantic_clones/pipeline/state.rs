use anyhow::Result;

use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::{
    load_active_embedding_setup, load_current_repo_embedding_states, persist_active_embedding_setup,
};
use crate::host::devql::RelationalStorage;

use super::schema::CloneProjection;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ActiveCloneEmbeddingStates {
    pub(super) code: embeddings::ActiveEmbeddingRepresentationState,
    pub(super) summary: Option<embeddings::ActiveEmbeddingRepresentationState>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LoadedRepresentationEmbedding {
    pub(super) setup: embeddings::EmbeddingSetup,
    pub(super) embedding: Vec<f32>,
}

pub(super) async fn resolve_active_embedding_states_for_clone_rebuild(
    relational: &RelationalStorage,
    repo_id: &str,
    projection: CloneProjection,
) -> Result<Option<ActiveCloneEmbeddingStates>> {
    let Some(code) = resolve_clone_rebuild_embedding_state(
        relational,
        repo_id,
        projection,
        embeddings::EmbeddingRepresentationKind::Code,
        true,
    )
    .await?
    else {
        return Ok(None);
    };
    let summary = resolve_clone_rebuild_embedding_state(
        relational,
        repo_id,
        projection,
        embeddings::EmbeddingRepresentationKind::Summary,
        false,
    )
    .await?;

    Ok(Some(ActiveCloneEmbeddingStates { code, summary }))
}

async fn resolve_clone_rebuild_embedding_state(
    relational: &RelationalStorage,
    repo_id: &str,
    projection: CloneProjection,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    required: bool,
) -> Result<Option<embeddings::ActiveEmbeddingRepresentationState>> {
    if projection == CloneProjection::Current {
        let current_states =
            load_current_repo_embedding_states(relational, repo_id, Some(representation_kind))
                .await?;
        return choose_current_projection_embedding_state(&current_states);
    }

    if let Some(active_state) =
        load_active_embedding_setup(relational, repo_id, representation_kind).await?
    {
        return Ok(Some(active_state));
    }

    let current_states =
        load_current_repo_embedding_states(relational, repo_id, Some(representation_kind)).await?;
    match current_states.as_slice() {
        [state] => {
            persist_active_embedding_setup(relational, repo_id, state).await?;
            Ok(Some(state.clone()))
        }
        [] => Ok(None),
        _ => {
            if required {
                log::warn!(
                    "semantic_clones clone rebuild skipped for repo {}: multiple {} embedding setups exist but no active state is persisted",
                    repo_id,
                    representation_kind
                );
            } else {
                log::warn!(
                    "semantic_clones clone rebuild will ignore summary embedding view for repo {}: multiple {} embedding setups exist but no active state is persisted",
                    repo_id,
                    representation_kind
                );
            }
            Ok(None)
        }
    }
}

pub(super) fn choose_current_projection_embedding_state(
    current_states: &[embeddings::ActiveEmbeddingRepresentationState],
) -> Result<Option<embeddings::ActiveEmbeddingRepresentationState>> {
    match current_states {
        [] => Ok(None),
        [state] => Ok(Some(state.clone())),
        _ => Ok(None),
    }
}
