use anyhow::{Context, Result, anyhow};
use rusqlite::Connection;
use std::fmt;
use std::time::Instant;

use crate::config::resolve_store_backend_config_for_repo;
use crate::storage::PostgresSyncConnection;

use super::runtime_store::{
    delete_runtime_embedding_mailbox_items, delete_runtime_summary_mailbox_items,
    insert_runtime_embedding_mailbox_item, insert_runtime_summary_mailbox_item,
    upsert_runtime_clone_rebuild_signal, upsert_runtime_embedding_mailbox_item,
};
use super::{CommitEmbeddingBatchRequest, CommitSummaryBatchRequest};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SummaryCommitPhaseTimings {
    pub transaction_start_ms: u64,
    pub summary_sql_ms: u64,
    pub runtime_embedding_mailbox_upsert_ms: u64,
    pub replacement_summary_backfill_insert_ms: u64,
    pub summary_mailbox_delete_ms: u64,
    pub transaction_commit_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SummaryCommitPhase {
    TransactionStart,
    SummarySql,
    RuntimeEmbeddingMailboxUpsert,
    RuntimeSummaryBackfillInsert,
    RuntimeSummaryMailboxDelete,
    TransactionCommit,
}

impl SummaryCommitPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TransactionStart => "transaction_start",
            Self::SummarySql => "summary_sql",
            Self::RuntimeEmbeddingMailboxUpsert => "runtime_embedding_mailbox_upsert",
            Self::RuntimeSummaryBackfillInsert => "runtime_summary_backfill_insert",
            Self::RuntimeSummaryMailboxDelete => "runtime_summary_mailbox_delete",
            Self::TransactionCommit => "transaction_commit",
        }
    }
}

#[derive(Debug)]
pub(crate) struct SummaryCommitReport {
    pub timings: SummaryCommitPhaseTimings,
}

#[derive(Debug)]
pub(crate) struct SummaryCommitFailure {
    phase: SummaryCommitPhase,
    timings: SummaryCommitPhaseTimings,
    runtime_store_writes_succeeded_in_tx: bool,
    cause: anyhow::Error,
}

impl SummaryCommitFailure {
    pub(super) fn new(
        phase: SummaryCommitPhase,
        timings: SummaryCommitPhaseTimings,
        runtime_store_writes_succeeded_in_tx: bool,
        cause: anyhow::Error,
    ) -> Self {
        Self {
            phase,
            timings,
            runtime_store_writes_succeeded_in_tx,
            cause,
        }
    }

    pub(crate) fn phase(&self) -> SummaryCommitPhase {
        self.phase
    }

    pub(crate) fn timings(&self) -> SummaryCommitPhaseTimings {
        self.timings
    }

    pub(crate) fn runtime_store_writes_succeeded_in_tx(&self) -> bool {
        self.runtime_store_writes_succeeded_in_tx
    }
}

impl fmt::Display for SummaryCommitFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "summary commit failure_substage={} runtime_store_writes_succeeded_in_tx={} transaction_start_ms={} summary_sql_ms={} runtime_embedding_mailbox_upsert_ms={} replacement_summary_backfill_insert_ms={} summary_mailbox_delete_ms={} transaction_commit_ms={}: ",
            self.phase.as_str(),
            self.runtime_store_writes_succeeded_in_tx,
            self.timings.transaction_start_ms,
            self.timings.summary_sql_ms,
            self.timings.runtime_embedding_mailbox_upsert_ms,
            self.timings.replacement_summary_backfill_insert_ms,
            self.timings.summary_mailbox_delete_ms,
            self.timings.transaction_commit_ms,
        )?;
        if f.alternate() {
            write!(f, "{:#}", self.cause)
        } else {
            write!(f, "{}", self.cause)
        }
    }
}

pub(super) fn execute_summary_commit(
    connection: &mut Connection,
    request: &CommitSummaryBatchRequest,
) -> std::result::Result<SummaryCommitReport, SummaryCommitFailure> {
    let mut timings = SummaryCommitPhaseTimings::default();
    let mut runtime_store_writes_succeeded_in_tx = false;

    let stage_started = Instant::now();
    let tx = match connection.transaction() {
        Ok(tx) => tx,
        Err(err) => {
            timings.transaction_start_ms = elapsed_ms(stage_started);
            return Err(SummaryCommitFailure::new(
                SummaryCommitPhase::TransactionStart,
                timings,
                runtime_store_writes_succeeded_in_tx,
                anyhow!(err).context("starting semantic summary batch transaction"),
            ));
        }
    };
    timings.transaction_start_ms = elapsed_ms(stage_started);

    let stage_started = Instant::now();
    for statement in &request.semantic_statements {
        if statement.trim().is_empty() {
            continue;
        }
        if let Err(err) = tx.execute_batch(statement) {
            timings.summary_sql_ms = elapsed_ms(stage_started);
            return Err(SummaryCommitFailure::new(
                SummaryCommitPhase::SummarySql,
                timings,
                runtime_store_writes_succeeded_in_tx,
                anyhow!(err).context("executing semantic summary SQL"),
            ));
        }
    }
    timings.summary_sql_ms = elapsed_ms(stage_started);

    let stage_started = Instant::now();
    for item in &request.embedding_follow_ups {
        if let Err(err) = upsert_runtime_embedding_mailbox_item(&tx, &request.repo, item) {
            timings.runtime_embedding_mailbox_upsert_ms = elapsed_ms(stage_started);
            return Err(SummaryCommitFailure::new(
                SummaryCommitPhase::RuntimeEmbeddingMailboxUpsert,
                timings,
                runtime_store_writes_succeeded_in_tx,
                err,
            ));
        }
        runtime_store_writes_succeeded_in_tx = true;
    }
    timings.runtime_embedding_mailbox_upsert_ms = elapsed_ms(stage_started);

    let stage_started = Instant::now();
    if let Some(item) = request.replacement_backfill_item.as_ref() {
        if let Err(err) = insert_runtime_summary_mailbox_item(&tx, &request.repo, item) {
            timings.replacement_summary_backfill_insert_ms = elapsed_ms(stage_started);
            return Err(SummaryCommitFailure::new(
                SummaryCommitPhase::RuntimeSummaryBackfillInsert,
                timings,
                runtime_store_writes_succeeded_in_tx,
                err,
            ));
        }
        runtime_store_writes_succeeded_in_tx = true;
    }
    timings.replacement_summary_backfill_insert_ms = elapsed_ms(stage_started);

    let stage_started = Instant::now();
    if let Err(err) =
        delete_runtime_summary_mailbox_items(&tx, &request.lease_token, &request.acked_item_ids)
    {
        timings.summary_mailbox_delete_ms = elapsed_ms(stage_started);
        return Err(SummaryCommitFailure::new(
            SummaryCommitPhase::RuntimeSummaryMailboxDelete,
            timings,
            runtime_store_writes_succeeded_in_tx,
            err,
        ));
    }
    if !request.acked_item_ids.is_empty() {
        runtime_store_writes_succeeded_in_tx = true;
    }
    timings.summary_mailbox_delete_ms = elapsed_ms(stage_started);

    let stage_started = Instant::now();
    if let Err(err) = tx.commit() {
        timings.transaction_commit_ms = elapsed_ms(stage_started);
        return Err(SummaryCommitFailure::new(
            SummaryCommitPhase::TransactionCommit,
            timings,
            runtime_store_writes_succeeded_in_tx,
            anyhow!(err).context("committing semantic summary batch transaction"),
        ));
    }
    timings.transaction_commit_ms = elapsed_ms(stage_started);
    Ok(SummaryCommitReport { timings })
}

pub(super) fn execute_embedding_commit(
    connection: &mut Connection,
    request: &CommitEmbeddingBatchRequest,
) -> Result<()> {
    let tx = connection
        .transaction()
        .context("starting semantic embedding batch transaction")?;
    for statement in request
        .embedding_statements
        .iter()
        .chain(request.setup_statements.iter())
    {
        if statement.trim().is_empty() {
            continue;
        }
        tx.execute_batch(statement)
            .context("executing semantic embedding SQL")?;
    }
    if !request.remote_embedding_statements.is_empty()
        || !request.remote_setup_statements.is_empty()
    {
        let backends = resolve_store_backend_config_for_repo(&request.repo.config_root)
            .context("resolving backend config for semantic embedding remote commit")?;
        let dsn = backends
            .relational
            .postgres_dsn
            .ok_or_else(|| anyhow!("semantic embedding remote commit requires Postgres DSN"))?;
        PostgresSyncConnection::connect(dsn)?
            .with_client(|client| {
                let statements = request
                    .remote_embedding_statements
                    .iter()
                    .chain(request.remote_setup_statements.iter())
                    .cloned()
                    .collect::<Vec<_>>();
                Box::pin(async move {
                    let tx = client
                        .transaction()
                        .await
                        .context("starting remote semantic embedding transaction")?;
                    for statement in &statements {
                        if statement.trim().is_empty() {
                            continue;
                        }
                        tx.batch_execute(statement)
                            .await
                            .context("executing remote semantic embedding SQL")?;
                    }
                    tx.commit()
                        .await
                        .context("committing remote semantic embedding transaction")?;
                    Ok(())
                })
            })
            .context("mirroring semantic embedding batch to Postgres")?;
    }
    if let Some(signal) = request.clone_rebuild_signal.as_ref() {
        upsert_runtime_clone_rebuild_signal(&tx, &request.repo, signal)?;
    }
    if let Some(item) = request.replacement_backfill_item.as_ref() {
        insert_runtime_embedding_mailbox_item(&tx, &request.repo, item)?;
    }
    delete_runtime_embedding_mailbox_items(&tx, &request.lease_token, &request.acked_item_ids)?;
    tx.commit()
        .context("committing semantic embedding batch transaction")?;
    Ok(())
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}
