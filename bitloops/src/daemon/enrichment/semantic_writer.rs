//! Semantic-writer entry point for enrichment batches.
//!
//! The public request/response types stay here, while the actor, commit execution, and SQLite
//! details live in focused submodules under [`semantic_writer`](self).

#[path = "semantic_writer/actor.rs"]
mod actor;
#[path = "semantic_writer/commit.rs"]
mod commit;
#[path = "semantic_writer/runtime_store.rs"]
mod runtime_store;

#[cfg(test)]
#[path = "semantic_writer/tests.rs"]
mod tests;

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::host::runtime_store::{
    CapabilityWorkplaneJobInsert, SemanticEmbeddingMailboxItemInsert,
    SemanticSummaryMailboxItemInsert,
};

use self::actor::RepoSemanticWriterActor;
pub(crate) use self::commit::{
    SummaryCommitFailure, SummaryCommitPhase, SummaryCommitPhaseTimings, SummaryCommitReport,
};

#[derive(Debug, Clone)]
pub(crate) struct SemanticBatchRepoContext {
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub config_root: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct CommitSummaryBatchRequest {
    pub repo: SemanticBatchRepoContext,
    pub lease_token: String,
    pub semantic_statements: Vec<String>,
    pub embedding_follow_ups: Vec<SemanticEmbeddingMailboxItemInsert>,
    pub replacement_backfill_item: Option<SemanticSummaryMailboxItemInsert>,
    pub acked_item_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CommitEmbeddingBatchRequest {
    pub repo: SemanticBatchRepoContext,
    pub lease_token: String,
    pub embedding_statements: Vec<String>,
    pub setup_statements: Vec<String>,
    pub remote_embedding_statements: Vec<String>,
    pub remote_setup_statements: Vec<String>,
    pub clone_rebuild_signal: Option<CapabilityWorkplaneJobInsert>,
    pub replacement_backfill_item: Option<SemanticEmbeddingMailboxItemInsert>,
    pub acked_item_ids: Vec<String>,
}

pub(crate) async fn commit_summary_batch(
    runtime_db_path: &Path,
    relational_db_path: &Path,
    request: CommitSummaryBatchRequest,
) -> std::result::Result<SummaryCommitReport, SummaryCommitFailure> {
    RepoSemanticWriterActor::shared(runtime_db_path, relational_db_path, &request.repo.repo_id)
        .map_err(|err| {
            SummaryCommitFailure::new(
                SummaryCommitPhase::TransactionStart,
                SummaryCommitPhaseTimings::default(),
                false,
                err.context("creating summary semantic writer actor"),
            )
        })?
        .commit_summary(request)
        .await
}

pub(crate) async fn commit_embedding_batch(
    runtime_db_path: &Path,
    relational_db_path: &Path,
    request: CommitEmbeddingBatchRequest,
) -> Result<()> {
    RepoSemanticWriterActor::shared(runtime_db_path, relational_db_path, &request.repo.repo_id)?
        .commit_embedding(request)
        .await
}
