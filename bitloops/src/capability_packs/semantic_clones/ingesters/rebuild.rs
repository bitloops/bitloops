use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::json;

use crate::host::capability_host::registrar::{
    BoxFuture, IngestRequest, IngestResult, IngesterHandler, IngesterRegistration,
};

use super::super::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
};

pub struct SymbolCloneEdgesRebuildIngester;

impl IngesterHandler for SymbolCloneEdgesRebuildIngester {
    fn ingest<'a>(
        &'a self,
        _request: IngestRequest,
        ctx: &'a mut dyn crate::host::capability_host::CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let relational = ctx
                .devql_relational_scoped(SEMANTIC_CLONES_CAPABILITY_ID)
                .context("scoped DevQL relational for semantic clone-edge rebuild")?;

            let repo_id = ctx.repo().repo_id.clone();
            let build =
                crate::capability_packs::semantic_clones::pipeline::rebuild_symbol_clone_edges(
                    relational, &repo_id,
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

pub fn build_symbol_clone_edges_rebuild_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        SEMANTIC_CLONES_CAPABILITY_ID,
        SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
        Arc::new(SymbolCloneEdgesRebuildIngester),
    )
}
