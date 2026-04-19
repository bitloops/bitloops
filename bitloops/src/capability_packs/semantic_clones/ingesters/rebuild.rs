use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::json;

use crate::host::capability_host::registrar::{
    BoxFuture, IngestRequest, IngestResult, IngesterHandler, IngesterRegistration,
};

use super::super::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
};
use crate::capability_packs::semantic_clones::runtime_config::resolve_semantic_clones_config;

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
            let config = resolve_semantic_clones_config(
                &ctx.config_view(SEMANTIC_CLONES_CAPABILITY_ID)
                    .context("loading semantic_clones config view")?,
            );
            let options = clone_rebuild_scoring_options(&config);
            let build = crate::capability_packs::semantic_clones::pipeline::rebuild_symbol_clone_edges_with_options(
                relational,
                &repo_id,
                options,
            )
            .await
            .context("rebuilding historical symbol clone edges")?;
            let current_build =
                crate::capability_packs::semantic_clones::pipeline::rebuild_current_symbol_clone_edges_with_options(
                    relational,
                    &repo_id,
                    options,
                )
                .await
                .context("rebuilding current symbol clone edges")?;
            Ok(IngestResult::new(
                json!({
                    "symbol_clone_edges_upserted": build.edges.len(),
                    "symbol_clone_sources_scored": build.sources_considered,
                    "current_symbol_clone_edges_upserted": current_build.edges.len(),
                    "current_symbol_clone_sources_scored": current_build.sources_considered,
                }),
                format!(
                    "rebuilt {} historical and {} current clone edges",
                    build.edges.len(),
                    current_build.edges.len()
                ),
            ))
        })
    }
}

fn clone_rebuild_scoring_options(
    config: &crate::config::SemanticClonesConfig,
) -> crate::capability_packs::semantic_clones::scoring::CloneScoringOptions {
    crate::capability_packs::semantic_clones::scoring::CloneScoringOptions::new(
        config.ann_neighbors,
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
        let options = clone_rebuild_scoring_options(&capability.semantic_clones);
        assert_eq!(options.ann_neighbors, 23);
    }
}
