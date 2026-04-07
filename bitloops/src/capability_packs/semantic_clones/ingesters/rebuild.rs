use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::json;

use crate::host::capability_host::registrar::{
    BoxFuture, IngestRequest, IngestResult, IngesterHandler, IngesterRegistration,
};

use super::super::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
};
use crate::config::resolve_embedding_capability_config_for_repo;

pub struct SymbolCloneEdgesRebuildIngester;

impl IngesterHandler for SymbolCloneEdgesRebuildIngester {
    fn ingest<'a>(
        &'a self,
        _request: IngestRequest,
        ctx: &'a mut dyn crate::host::capability_host::CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let relational = ctx
                .clone_edges_rebuild_relational()
                .context("clone-edge rebuild relational for semantic clone-edge rebuild")?;

            let repo_id = ctx.repo().repo_id.clone();
            let capability = resolve_embedding_capability_config_for_repo(ctx.repo_root());
            let options = clone_rebuild_scoring_options(&capability);
            let build = crate::capability_packs::semantic_clones::pipeline::rebuild_symbol_clone_edges_with_options(
                relational,
                &repo_id,
                options,
            )
            .await
            .context("rebuilding symbol clone edges")?;
            Ok(IngestResult::new(
                json!({
                    "symbol_clone_edges_upserted": build.edges.len(),
                    "symbol_clone_sources_scored": build.sources_considered,
                }),
                format!(
                    "rebuilt {} clone edges ({} sources scored)",
                    build.edges.len(),
                    build.sources_considered
                ),
            ))
        })
    }
}

fn clone_rebuild_scoring_options(
    capability: &crate::config::EmbeddingCapabilityConfig,
) -> crate::capability_packs::semantic_clones::scoring::CloneScoringOptions {
    crate::capability_packs::semantic_clones::scoring::CloneScoringOptions::new(
        capability.semantic_clones.ann_neighbors,
    )
}

pub fn build_symbol_clone_edges_rebuild_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        SEMANTIC_CLONES_CAPABILITY_ID,
        SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
        Arc::new(SymbolCloneEdgesRebuildIngester),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_rebuild_scoring_options_uses_semantic_clone_ann_neighbors() {
        let capability = crate::config::EmbeddingCapabilityConfig {
            semantic_clones: crate::config::SemanticClonesConfig {
                ann_neighbors: 23,
                ..Default::default()
            },
            ..Default::default()
        };
        let options = clone_rebuild_scoring_options(&capability);
        assert_eq!(options.ann_neighbors, 23);
    }
}
