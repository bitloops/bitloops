//! Symbol clone edge rebuild orchestration for the **semantic_clones** capability pack.
//!
//! Pure clone scoring lives in [`crate::capability_packs::semantic_clones::scoring::build_symbol_clone_edges`].
//! This module loads candidates from DevQL relational storage, applies pack DDL when needed,
//! and persists edges. **DevQL ingestion** should trigger rebuild only via the registered
//! ingester ([`super::SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID`]) or
//! [`rebuild_symbol_clone_edges`](fn@rebuild_symbol_clone_edges) (also re-exported at
//! `crate::host::devql` under `cfg(test)` for integration tests), not by duplicating this
//! pipeline.

mod candidates;
mod orchestrator;
mod parse;
mod persistence;
mod queries;
mod schema;
mod state;

#[cfg(test)]
mod tests;

pub(crate) use orchestrator::{
    rebuild_current_symbol_clone_edges, rebuild_symbol_clone_edges,
    rebuild_symbol_clone_edges_with_options, score_symbol_clone_edges_for_source_with_options,
};
#[allow(unused_imports)]
pub(crate) use persistence::delete_repo_current_symbol_clone_edges;
pub(crate) use persistence::delete_repo_symbol_clone_edges;
pub(crate) use schema::init_postgres_semantic_clones_schema;
