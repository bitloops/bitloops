//! Current-state reconciliation entry point for the **semantic_clones** capability pack.
//!
//! [`consumer::SemanticClonesCurrentStateConsumer`] is registered with the host and orchestrates
//! the per-repository reconciliation: it clears the projection rows for affected paths, decides
//! which mailboxes to enqueue work onto, and issues the relevant repo-backfill or per-artefact
//! jobs. The implementation details live in cohesive submodules under [`current_state`](self).

mod consumer;
mod jobs;
mod projection;

#[cfg(test)]
mod tests;

pub use consumer::SemanticClonesCurrentStateConsumer;
